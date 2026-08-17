#!/usr/bin/env bash
# Regression/parity tests for `vpn doctor`'s new read-only
# expected-public-surface and firewall-ownership reporting
# (apps/admin/src/main.rs: check_expected_public_surface,
# check_firewall_zone_and_ownership, read_firewall_ownership). The
# classification/parsing logic itself is unit-tested directly in Rust
# (apps/admin/src/main.rs's `public_surface_tests` module, with
# synthetic/fake `ss` output and a fake firewall-owned.env file) — this
# file covers what only makes sense to check from the shell side:
#   - the ownership-file KEY NAMES firewall.sh/firewall-ufw.sh write
#     exactly match the ones `vpn doctor` reads, so the two can never
#     silently drift apart (one side renames a field, the other keeps
#     reading the old name forever).
#   - the fixed path both sides agree the file lives at.
#   - `vpn doctor` never invokes anything that could mutate firewall,
#     listener, sshd, DNS, MTU, or IPv6 state (this whole phase is
#     read-only by requirement) — a static source-pattern guard against
#     regressions, not a functional proof (that needs a real host).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FIREWALL_SH="$REPO_ROOT/deploy/almalinux/firewall.sh"
FIREWALL_UFW_SH="$REPO_ROOT/deploy/almalinux/firewall-ufw.sh"
MAIN_RS="$REPO_ROOT/apps/admin/src/main.rs"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

for f in "$FIREWALL_SH" "$FIREWALL_UFW_SH" "$MAIN_RS"; do
  [ -f "$f" ] || fail "expected source file is missing: $f"
done

echo "--- ownership file path agreement ---"
for f in "$FIREWALL_SH" "$FIREWALL_UFW_SH"; do
  if grep -q '/var/lib/vpn1/firewall-owned.env' "$f"; then
    ok "$(basename "$f") writes the ownership file at /var/lib/vpn1/firewall-owned.env"
  else
    fail "$(basename "$f") does not reference /var/lib/vpn1/firewall-owned.env"
  fi
done
if grep -q '"/var/lib/vpn1/firewall-owned.env"' "$MAIN_RS"; then
  ok "vpn-admin reads the same fixed path"
else
  fail "vpn-admin does not reference the same /var/lib/vpn1/firewall-owned.env path"
fi

echo
echo "--- ownership-file field names: written by firewall.sh, read by vpn-admin ---"
# Every KEY=VALUE field firewall.sh actually writes into the ownership
# state file (the heredoc that produces $OWNERSHIP_STATE.tmp).
written_fields="$(sed -n '/cat >"\$OWNERSHIP_STATE.tmp"/,/^EOF$/p' "$FIREWALL_SH" \
  | grep -oE '^[a-z0-9_]+=' | tr -d '=' | sort -u)"
if [ -z "$written_fields" ]; then
  fail "could not extract any field written by firewall.sh — this test may no longer match its source structure"
fi
while IFS= read -r field; do
  [ -z "$field" ] && continue
  # firewall_zone is firewall.sh-internal bookkeeping (used only to
  # detect a zone change invalidating prior ownership) — vpn-admin
  # queries the live zone directly via `firewall-cmd
  # --get-default-zone` instead of trusting a possibly-stale recorded
  # value, so it deliberately does not read this field back.
  case "$field" in
    firewall_zone) continue ;;
  esac
  if grep -q "\"$field\"" "$MAIN_RS"; then
    ok "vpn-admin reads the '$field' field firewall.sh writes"
  else
    fail "firewall.sh writes '$field' but vpn-admin's read_firewall_ownership()/check_firewall_zone_and_ownership() never reference it — teach vpn-admin about it, or document why not"
  fi
done <<EOF
$written_fields
EOF

echo
echo "--- firewall-ufw.sh writes the same field shape as firewall.sh ---"
ufw_fields="$(sed -n '/cat >"\$OWNERSHIP_STATE.tmp"/,/^EOF$/p' "$FIREWALL_UFW_SH" \
  | grep -oE '^[a-z0-9_]+=' | tr -d '=' | sort -u)"
if [ "$written_fields" = "$ufw_fields" ]; then
  ok "firewall.sh and firewall-ufw.sh write identical ownership-file field sets"
else
  fail "firewall.sh and firewall-ufw.sh write different ownership-file fields — vpn-admin's single reader must handle both:
firewall.sh:     $written_fields
firewall-ufw.sh: $ufw_fields"
fi

echo
echo "--- vpn doctor's public-surface/firewall reporting is read-only (static guard) ---"
# Extract the body of check_expected_public_surface,
# check_firewall_zone_and_ownership, and enumerate_listeners (the only
# functions in this phase that shell out at all) and confirm none of
# them invoke a mutating command. This is a coarse guard, not a proof —
# it exists to catch an obvious regression (someone adding a
# `firewall-cmd --add-port` or `ip link set` call inside a function this
# phase promised was read-only), not to replace code review.
extract_fn_body() {
  # Prints from the function's `fn name(` line up to the first line at
  # column 0 that is a lone closing brace, i.e. the end of the function.
  awk -v pat="^fn $1\\\\(" '
    $0 ~ pat { printing=1 }
    printing { print }
    printing && /^}$/ { exit }
  ' "$MAIN_RS"
}
for fn in check_expected_public_surface check_firewall_zone_and_ownership enumerate_listeners detect_ssh_port; do
  body="$(extract_fn_body "$fn")"
  if [ -z "$body" ]; then
    fail "could not extract the body of $fn() from main.rs — this guard may no longer match its source structure"
    continue
  fi
  mutating_hit=""
  for pattern in \
    '"add-port"' '"remove-port"' '"--add-' '"--remove-' '"--reload"' \
    '"reset-failed"' '"disable"' '"enable"' '"stop"' '"restart"' \
    '"set"' 'set-default-zone'; do
    if printf '%s' "$body" | grep -qF -- "$pattern"; then
      mutating_hit="$pattern"
      break
    fi
  done
  if [ -n "$mutating_hit" ]; then
    fail "$fn() appears to contain a mutating pattern ($mutating_hit) — this phase must stay read-only"
  else
    ok "$fn() contains no obviously-mutating firewall/service/listener command"
  fi
done

echo
if [ "$failures" -eq 0 ]; then
  echo "PASS: test-doctor-public-surface-parity.sh"
  exit 0
else
  echo "FAIL: test-doctor-public-surface-parity.sh ($failures failure(s))"
  exit 1
fi
