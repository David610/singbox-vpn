#!/usr/bin/env bash
# Static source-inspection test for update.sh's two fixes:
#   (1) it must gate acceptance on a real protocol handshake
#       (`doctor --protocol --require-protocol`), not just the shallow
#       L1-L3 health check — see install.sh's acceptance_stage for the
#       precedent this must mirror.
#   (2) sing-box must NOT be unconditionally restarted every update —
#       only when its rendered config content or its systemd unit file
#       actually changed. Does not run update.sh itself (needs
#       root/systemd/a real deployment) — it parses the script's own
#       source for the specific patterns the fix requires and fails
#       loudly if either regresses.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"

failures=0
ok() { echo "ok: $1"; }
fail() { echo "FAIL: $1"; failures=$((failures + 1)); }

# --- (1) real protocol acceptance gate ---

if grep -qE 'doctor --protocol --require-protocol' "$UPDATE_SH"; then
  ok "update.sh calls 'doctor --protocol --require-protocol'"
else
  fail "update.sh does not call 'doctor --protocol --require-protocol' — an update could commit a syntactically-valid but functionally-broken REALITY config"
fi

# update.sh now has two structurally separate code paths — the explicit
# --dev-rebuild/SINGBOX_VPN_CHANNEL=dev escape hatch, and the default
# production release-to-release path — each with its OWN doctor/
# health-check/committed=1 sequence. Check each occurrence's ordering
# against its own nearest-preceding health-check/committed line rather
# than assuming a single linear script.
doctor_lines="$(grep -n 'doctor --protocol --require-protocol' "$UPDATE_SH" | cut -d: -f1)"
committed_lines="$(grep -n '^committed=1$' "$UPDATE_SH" | cut -d: -f1)"
health_check_lines="$(grep -n '/usr/local/bin/vpn-health-check$' "$UPDATE_SH" | cut -d: -f1)"
[ -n "$doctor_lines" ] || fail "update.sh has no 'doctor --protocol --require-protocol' occurrence to check ordering for"

all_before_committed_ok=1
all_after_health_check_ok=1
while IFS= read -r doctor_line; do
  [ -z "$doctor_line" ] && continue
  # nearest committed=1 AFTER this doctor call
  next_committed="$(echo "$committed_lines" | awk -v d="$doctor_line" '$1 > d {print; exit}')"
  [ -n "$next_committed" ] || { all_before_committed_ok=0; continue; }
  # nearest health-check line BEFORE this doctor call
  prev_health="$(echo "$health_check_lines" | awk -v d="$doctor_line" '$1 < d {v=$1} END{print v}')"
  [ -n "$prev_health" ] || { all_after_health_check_ok=0; continue; }
done <<EOF
$doctor_lines
EOF
if [ "$all_before_committed_ok" -eq 1 ]; then
  ok "every protocol acceptance check runs before its path's transaction commits (rollback still armed)"
else
  fail "some protocol acceptance check does not clearly run before its path's 'committed=1' — a failed handshake might not trigger rollback"
fi
if [ "$all_after_health_check_ok" -eq 1 ]; then
  ok "every protocol acceptance check runs after a preceding health check"
else
  fail "some protocol acceptance check does not clearly run after a preceding health check"
fi

# On failure it must go through `die` (which the existing EXIT trap
# turns into a rollback), not a bare `exit`. Check every occurrence.
die_after_every_doctor_call=1
while IFS= read -r doctor_line; do
  [ -z "$doctor_line" ] && continue
  doctor_block="$(sed -n "${doctor_line},+6p" "$UPDATE_SH" 2>/dev/null || true)"
  echo "$doctor_block" | grep -q 'die "post-update protocol acceptance' || die_after_every_doctor_call=0
done <<EOF
$doctor_lines
EOF
if [ "$die_after_every_doctor_call" -eq 1 ]; then
  ok "protocol acceptance failure calls die() (reuses the existing rollback trap, not a new mechanism)"
else
  fail "protocol acceptance failure does not clearly call die() — rollback may not trigger"
fi

# --- (2) no unconditional sing-box restart ---

# The old, fixed bug: a bare, unguarded `systemctl reload-or-restart
# sing-box` with nothing above it deciding whether one is needed. The
# fix keeps `systemctl reload-or-restart sing-box` (it's still the
# right command for the cases that DO need a restart) but every
# occurrence must now sit inside a conditional.
unguarded_restart=0
while IFS= read -r lineno; do
  # Look at the preceding non-blank line; it must be part of an if/elif
  # chain (or the line itself must be indented under one), not sit at
  # column 0 as an unconditional statement.
  line="$(sed -n "${lineno}p" "$UPDATE_SH")"
  case "$line" in
    "systemctl reload-or-restart sing-box")
      # Column-0 (unindented) occurrence: only acceptable if it is the
      # rollback path's own restart (rollback must unconditionally
      # restore the working config regardless of what changed).
      context="$(sed -n "$((lineno > 5 ? lineno - 5 : 1)),${lineno}p" "$UPDATE_SH")"
      if ! echo "$context" | grep -q 'rollback_update'; then
        unguarded_restart=1
      fi
      ;;
  esac
done < <(grep -n 'systemctl reload-or-restart sing-box' "$UPDATE_SH" | cut -d: -f1)
if [ "$unguarded_restart" -eq 0 ]; then
  ok "no unconditional (unguarded) sing-box restart outside the rollback path"
else
  fail "found an unconditional 'systemctl reload-or-restart sing-box' outside the rollback path — every routine update would drop live connections again"
fi

if grep -q 'singbox_config_changed' "$UPDATE_SH" && grep -q 'singbox_unit_changed' "$UPDATE_SH"; then
  ok "update.sh tracks whether sing-box's config content and unit file actually changed"
else
  fail "update.sh does not track config/unit-file change state before deciding whether to restart sing-box"
fi

if [ "$failures" -eq 0 ]; then
  echo "PASS: update.sh conditional-restart + protocol-acceptance checks"
  exit 0
fi
echo "FAILED: $failures check(s) above"
exit 1
