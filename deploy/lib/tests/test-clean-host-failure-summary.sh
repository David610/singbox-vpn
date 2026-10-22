#!/usr/bin/env bash
# The clean-host runner prints a failing test as `FAIL <test>` plus a few reason lines. A missing prerequisite ("PyYAML is
# required ...", "No module named", "command not found") contains none of FAIL/UNEXPECTED/error, so the summary used to be a
# bare `FAIL` and a deterministic environment problem looked like an unexplained flake (2026-10-21, qualification 625a34f).
# Rule: a failure ALWAYS prints at least one reason line; never an empty reason.
set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=deploy/lib/test-failure-summary.sh
. "$LIB/test-failure-summary.sh"
failures=0
ok() { echo "ok: $1"; }
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

r="$(test_failure_reason $'ok: a\nPyYAML is required for the workflow contract test')"
grep -q 'PyYAML is required' <<<"$r" && ok "a missing-prerequisite message is shown" || fail "prerequisite message lost: [$r]"

r="$(test_failure_reason $'ok: a\nFAIL: the thing broke\nok: b')"
grep -q 'FAIL: the thing broke' <<<"$r" && ok "an explicit FAIL line is shown" || fail "FAIL line lost"

r="$(test_failure_reason $'line1\nline2\nline3\nline4\nline5\nline6\nline7')"
[ -n "$r" ] && [ "$(wc -l <<<"$r")" -le 8 ] && grep -q 'line7' <<<"$r" && ok "no keyword: the last lines are shown, never nothing" || fail "empty/oversized reason: [$r]"

r="$(test_failure_reason '')"
grep -q 'no output' <<<"$r" && ok "a test that failed silently says so" || fail "silent failure not reported: [$r]"

r="$(test_failure_reason "$(seq 1 40 | sed 's/^/FAIL: n/')")"
[ "$(wc -l <<<"$r")" -le 8 ] && ok "output is bounded" || fail "unbounded reason"
grep -q 'test_failure_reason "\$out2"' "$LIB/run-shell-tests-clean-host.sh" && ok "the runner uses the helper for its FAIL reasons" \
  || fail "run-shell-tests-clean-host.sh no longer prints reasons through test_failure_reason"
[ "$failures" -eq 0 ] && echo "PASS: test-clean-host-failure-summary.sh"
