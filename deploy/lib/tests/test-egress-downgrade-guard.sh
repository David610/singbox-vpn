#!/usr/bin/env bash
# Behavioural test of the update/repair/downgrade guard for a deployment that
# declares an [egress] literal-IPv6 policy.
#
# Why this exists: a vpn-admin that predates the policy (v1.2.0) silently
# ignores an unknown [egress] table and re-renders a plain exit, i.e. it would
# drop a security-relevant setting without any error. update.sh therefore must
# refuse to switch such a deployment to such a build BEFORE anything live
# changes. This test runs update.sh's REAL pre-switch block (extracted from the
# script, not re-typed) against:
#   * every spelling of the table TOML allows (not only the literal header),
#   * an "old" vpn-admin that ignores [egress] and a "new" one that understands
#     it,
# and proves the refusal leaves the live tree byte-identical.
#
# By default the two vpn-admin builds are small stand-ins that print exactly what
# the real builds print. Set OLD_VPN_ADMIN (a v1.2.0 build) and NEW_VPN_ADMIN
# (a build from this tree) to run the same cases against the real binaries; the
# TOML spellings are always checked against the production parser by the Rust
# test `every_toml_spelling_of_the_egress_table_is_detected` in apps/admin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$ROOT/deploy/almalinux/update.sh"
# shellcheck source=deploy/lib/node-identity.sh
. "$ROOT/deploy/lib/node-identity.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# ---- the two vpn-admin builds -------------------------------------------------
make_fake() { # $1 = path, $2 = old|new
  cat >"$1" <<EOF
#!/usr/bin/env bash
# stand-in for a vpn-admin '$2' build: only 'config validate' is implemented
[ "\${3:-} \${4:-}" = "config validate" ] || exit 64
echo "deployment.toml: CURRENT (schema_version 2)"
echo "$RELAY_ENFORCEMENT_CAPABILITY (node role: exit)"
if [ "$2" = new ]; then
  echo "$EXIT_EGRESS_POLICY_CAPABILITY"
  echo "egress-policy: \${FAKE_EGRESS_STATE:-absent}"
fi
exit 0
EOF
  chmod 0755 "$1"
}
OLD="$WORK/old-vpn-admin"; NEW="$WORK/new-vpn-admin"
if [ -n "${OLD_VPN_ADMIN:-}" ] && [ -n "${NEW_VPN_ADMIN:-}" ]; then
  OLD="$OLD_VPN_ADMIN"; NEW="$NEW_VPN_ADMIN"; REAL=1
  echo "using REAL binaries: old=$OLD new=$NEW"
else
  make_fake "$OLD" old; make_fake "$NEW" new; REAL=0
fi

# ---- a complete, loadable deployment in every spelling TOML allows ---------------
variants="$WORK/variants"; negatives="$WORK/negatives"; mkdir -p "$variants" "$negatives"
write_toml() { # $1 = output file, $2 = lines placed before the first table, $3 = lines after the tables
  {
    cat <<'TOP'
public_host = "vpn.example.com"
subscription_host = "sub.example.com"
state_dir = "/var/lib/vpn-test"
singbox_binary = "/usr/local/bin/sing-box"
role = "exit"
node_id = "n1"
schema_version = 2
TOP
    printf '%s\n' "$2"
    cat <<'TABLES'

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
TABLES
    printf '%s\n' "$3"
  } >"$1"
}
write_toml "$variants/header.toml" '' '
[egress]
ipv6_literal = "reject_google"'
write_toml "$variants/spaced.toml" '' '
[ egress ]
ipv6_literal = "reject_google"'
write_toml "$variants/quoted.toml" '' '
["egress"]
ipv6_literal = "reject_google"'
write_toml "$variants/single-quoted.toml" '' "
['egress']
ipv6_literal = \"reject_google\""
write_toml "$variants/trailing-comment.toml" '' '
[egress] # explained
ipv6_literal = "reject_all"'
# dotted keys and inline tables must precede the first [table] header
write_toml "$variants/dotted.toml" 'egress.ipv6_literal = "reject_google"' ''
write_toml "$variants/inline.toml" 'egress = { ipv6_literal = "reject_all" }' ''
write_toml "$negatives/comment.toml" '' '
# [egress] is only mentioned in a comment'
write_toml "$negatives/lookalike.toml" '' '
[egress_notes]
x = 1'
write_toml "$negatives/plain.toml" '' ''

validate_of() { # $1 = binary, $2 = toml, $3 = state for the stand-in new build
  FAKE_EGRESS_STATE="${3:-absent}" "$1" --config "$2" config validate 2>&1 || true
}
state_for() { # state a correct new build reports for a file (stand-in only)
  case "$1" in "$variants"/*) echo declared ;; *) echo absent ;; esac
}

echo "--- old installed build (no egress-policy line): the text fallback must catch every spelling ---"
for f in "$variants"/*.toml; do
  out_old="$(validate_of "$OLD" "$f")"
  if egress_policy_switch_refusal "$f" "$out_old" "$out_old" >/dev/null; then
    ok "$(basename "$f"): refused (target ignores [egress])"
  else
    fail "$(basename "$f"): a deployment with an egress policy would be switched to a build that ignores it"
  fi
done
for f in "$negatives"/*.toml; do
  out_old="$(validate_of "$OLD" "$f")"
  if egress_policy_switch_refusal "$f" "$out_old" "$out_old" >/dev/null; then
    fail "$(basename "$f"): refused although no policy is declared"
  else
    ok "$(basename "$f"): allowed (no policy declared)"
  fi
done

echo "--- new installed build: its parsed answer decides, whatever the spelling ---"
for f in "$variants"/*.toml; do
  out_new="$(validate_of "$NEW" "$f" "$(state_for "$f")")"
  out_old="$(validate_of "$OLD" "$f")"
  if egress_policy_switch_refusal "$f" "$out_new" "$out_old" >/dev/null; then
    ok "$(basename "$f"): refused to switch to an old target"
  else
    fail "$(basename "$f"): switch to an old target was allowed"
  fi
  if egress_policy_switch_refusal "$f" "$out_new" "$out_new" >/dev/null; then
    fail "$(basename "$f"): refused although the target understands the policy"
  else
    ok "$(basename "$f"): allowed to a target that understands it"
  fi
done
# the parser's answer beats the text: a commented header with 'absent' is allowed
f="$negatives/comment.toml"
if egress_policy_switch_refusal "$f" "$(validate_of "$NEW" "$f" absent)" "$(validate_of "$OLD" "$f")" >/dev/null; then
  fail "a header inside a comment was treated as a policy"
else
  ok "a commented-out header does not block a switch (parser says absent)"
fi

echo "--- the real update.sh pre-switch block refuses BEFORE anything live changes ---"
block="$(sed -n '/^if \[ -f "\$DEPLOYMENT_TOML" \]; then$/,/^fi$/p' "$UPDATE_SH")"
[ -n "$block" ] || fail "could not extract update.sh's pre-switch block"
live="$WORK/live"; staged="$WORK/staged"; mkdir -p "$live/bin" "$staged"
cp "$NEW" "$live/bin/vpn-admin"       # the host currently runs a build that understands the policy
cp "$OLD" "$staged/vpn-admin"         # the target is an older release
printf 'live binary\n' >"$live/bin/vpn-subscription-svc"
printf 'rendered config\n' >"$live/config.json"
run_block() { # $1 toml; runs the extracted block in a subshell, returns its status; output kept
  (
    set -euo pipefail
    export BIN_DIR="$live/bin" STAGED_BIN_DIR="$staged" DEPLOYMENT_TOML="$1" TARGET_VERSION="v1.2.0"
    export FAKE_EGRESS_STATE; FAKE_EGRESS_STATE="$(state_for "$1")"
    log() { echo "[log] $*"; }
    die() { echo "[die] $*" >&2; exit 42; }
    eval "$block"
  )
}
before="$(find "$live" -type f -exec sha256sum {} + | sort | sha256sum)"
for f in "$variants"/*.toml; do
  rc=0; run_block "$f" >"$WORK/block.out" 2>&1 || rc=$?
  if [ "$rc" -eq 42 ] && grep -q "Nothing live has been changed" "$WORK/block.out"; then
    ok "$(basename "$f"): update.sh stops with 'Nothing live has been changed'"
  else
    fail "$(basename "$f"): update.sh block returned $rc instead of refusing: $(tail -3 "$WORK/block.out")"
  fi
done
after="$(find "$live" -type f -exec sha256sum {} + | sort | sha256sum)"
[ "$before" = "$after" ] && ok "live binaries and rendered config are byte-identical after every refusal" \
  || fail "a refusal changed the live tree"
cp "$NEW" "$staged/vpn-admin"   # a target that understands the policy proceeds
rc=0; run_block "$variants/header.toml" >"$WORK/block.out" 2>&1 || rc=$?
[ "$rc" -eq 0 ] && ok "the same deployment proceeds when the target understands the policy" \
  || fail "a capable target was refused (rc=$rc): $(tail -3 "$WORK/block.out")"

echo "--- ordering: the refusal sits before any live mutation in update.sh ---"
block_end="$(awk '/egress_policy_switch_refusal/ {found=NR} END {print found+0}' "$UPDATE_SH")"
first_mut="$(grep -n '^mutation_started=1$' "$UPDATE_SH" | tail -1 | cut -d: -f1)"
first_install="$(grep -n 'install -d -m 0700 -o root -g root "$BACKUP_ROOT"' "$UPDATE_SH" | tail -1 | cut -d: -f1)"
if [ "$block_end" -gt 0 ] && [ "$block_end" -lt "$first_mut" ] && [ "$block_end" -lt "${first_install:-999999}" ]; then
  ok "guard (line $block_end) precedes the PREPARE backup directory creation (line $first_install) and mutation_started=1 (line $first_mut)"
else
  fail "the guard is not provably before the first live mutation (guard=$block_end mutation=$first_mut prepare=${first_install:-none})"
fi

if [ "$REAL" = 1 ]; then
  echo "--- real binaries: v1.2.0 really does load a file with [egress] and say nothing ---"
  for f in "$variants"/header.toml "$variants"/inline.toml "$variants"/dotted.toml; do
    rc=0; out="$("$OLD" --config "$f" config validate 2>&1)" || rc=$?
    if [ "$rc" -le 2 ] && ! printf '%s' "$out" | grep -qi 'egress'; then
      ok "$(basename "$f"): the old build accepts the file and never mentions the policy (the silent-ignore hazard)"
    else
      fail "$(basename "$f"): the old build did not behave as a silent-ignore (rc=$rc): $out"
    fi
  done
  for f in "$variants"/*.toml; do
    if "$NEW" --config "$f" config validate 2>&1 | grep -q '^egress-policy: declared'; then
      ok "$(basename "$f"): the new build reports the policy as declared"
    else
      fail "$(basename "$f"): the new build did not report the policy"
    fi
  done
fi

if [ "$failures" -ne 0 ]; then
  echo "FAIL: test-egress-downgrade-guard.sh ($failures failure(s))" >&2
  exit 1
fi
echo "PASS: test-egress-downgrade-guard.sh ($([ "$REAL" = 1 ] && echo real || echo stand-in) vpn-admin builds)"
