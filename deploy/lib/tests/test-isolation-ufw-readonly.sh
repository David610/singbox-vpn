#!/usr/bin/env bash
# Regression (I-52): the update test-phase isolation guard recorded the doctor's
# read-only `ufw status` as a refused host-control attempt, so --dev-rebuild
# could never pass its own test phase on a ufw host ("this is a test defect").
# Read-only queries must be refused silently (exit 97, no log entry); mutating
# calls must still be recorded.
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/test-isolation.sh"
isolation_write_guards "$T" || { echo "cannot write guards"; exit 1; }
LOG="$T/refused-host-control.log"

"$T/bin/ufw" status >/dev/null 2>&1; rc=$?
[ "$rc" -eq 97 ] && ok "ufw status is still refused (exit 97): tests never reach the real tool" || bad "ufw status exit $rc"
[ ! -s "$LOG" ] && ok "read-only 'ufw status' is NOT recorded as a host-control attempt" || bad "ufw status recorded as an attempt: $(cat "$LOG")"

: >"$LOG"
for args in "allow 22" "enable" "--force disable" "delete allow 443"; do
  # shellcheck disable=SC2086
  "$T/bin/ufw" $args >/dev/null 2>&1
done
[ "$(wc -l <"$LOG")" -eq 4 ] && ok "mutating ufw calls are all recorded" || bad "mutating ufw calls not all recorded: $(cat "$LOG")"

: >"$LOG"; "$T/bin/firewall-cmd" --state >/dev/null 2>&1; "$T/bin/nft" list ruleset >/dev/null 2>&1
[ "$(wc -l <"$LOG")" -eq 1 ] && ok "firewall-cmd query stays silent, nft is still recorded (behaviour unchanged)" || bad "other guards changed: $(cat "$LOG")"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
