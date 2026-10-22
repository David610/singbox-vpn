#!/usr/bin/env bash
# Regression (I-43): acceptance-test.sh printed "PASSED (all required checks
# passed)" directly beneath FAIL lines (3 on Ubuntu 26.04) and verified no
# firewall at all on ufw hosts. The verdict must count non-required failures.
# shellcheck disable=SC2034  # variables are consumed by the eval-loaded acceptance_summary
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/deploy/almalinux/acceptance-test.sh"
failures=0
t_ok() { echo "ok: $*"; }
t_fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# Load only the helper block (counters, pass/fail helpers, verdict function).
eval "$(sed -n '/^NONREQ_FAIL=0/,/^section() /p' "$SCRIPT" | sed '$d')"

run_case() { # $1=label ; uses REQUIRED_FAIL/NONREQ_*/ACCEPTANCE_STRICT set by caller
  out="$(acceptance_summary)"; RC=$?
}

REQUIRED_FAIL=0; NONREQ_FAIL=0; NONREQ_NAMES=(); ACCEPTANCE_STRICT=0
run_case; grep -q 'PASSED (all required checks passed; no failures' <<<"$out" && [ "$RC" = 0 ] \
  && t_ok "no failures at all: clean PASSED, exit 0" || t_fail "clean case wrong: $out rc=$RC"

NONREQ_FAIL=3; NONREQ_NAMES=("AlmaLinux 9 detected" "SELinux enforcing" "firewalld active")
run_case
if grep -q 'PASSED WITH 3 NON-REQUIRED' <<<"$out" && grep -q 'NOT a clean pass' <<<"$out" && [ "$RC" = 0 ]; then
  t_ok "3 non-required FAILs: verdict says PASSED WITH 3 NON-REQUIRED FAILURE(S), not a clean pass"
else t_fail "non-required failures were summarised as a clean pass: $out"; fi
grep -q 'SELinux enforcing' <<<"$out" && t_ok "failing check names are listed" || t_fail "names missing"

ACCEPTANCE_STRICT=1; run_case
[ "$RC" = 1 ] && grep -q 'FAILED (strict' <<<"$out" && t_ok "ACCEPTANCE_STRICT=1 fails the run" || t_fail "strict mode did not fail: $out rc=$RC"

ACCEPTANCE_STRICT=0; REQUIRED_FAIL=1; run_case
[ "$RC" = 1 ] && grep -q 'FAILED (one or more required' <<<"$out" && t_ok "required failure still fails" || t_fail "required failure wrong: $out"

echo "--- fail() really counts (helper wiring, not just the verdict) ---"
REQUIRED_FAIL=0; NONREQ_FAIL=0; NONREQ_NAMES=()
fail "dummy" >/dev/null; [ "$NONREQ_FAIL" -eq 1 ] && t_ok "fail() increments the counter" || t_fail "fail() does not count"

grep -q 'ufw allows' "$SCRIPT" && t_ok "script verifies ufw allow rules on ufw hosts" || t_fail "no ufw verification"
[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
