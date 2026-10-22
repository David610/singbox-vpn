#!/usr/bin/env bash
# #170: `offline-full-verdict` is meant to be THE required status check. A job
# whose `needs` failed is SKIPPED by default, and GitHub treats a skipped
# required check as passing, so a failing `test`/`shell`/... job would not have
# blocked a merge. The verdict job must run `always()` and fail itself unless
# every needed job succeeded.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TOOL="$ROOT/deploy/lib/ci-needs-verdict.py"
CI_YML="$ROOT/.github/workflows/ci.yml"
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

echo "--- static: the verdict job cannot be skipped by a failed dependency ---"
job="$(sed -n '/^  offline-full-verdict:$/,/^  [a-z0-9-]*:$/p' "$CI_YML")"
grep -qE '^    if: \$\{\{ always\(\) \}\}$' <<<"$job" \
  && ok "verdict job runs if: always()" || fail "verdict job is skipped when a needed job fails"
grep -q 'NEEDS_JSON: \${{ toJSON(needs) }}' <<<"$job" \
  && ok "verdict job receives every dependency result" || fail "verdict job does not read the needs results"
check_line="$(grep -n 'ci-needs-verdict.py' <<<"$job" | head -1 | cut -d: -f1 || true)"
equiv_line="$(grep -n 'test-offline-full-ci-equivalence.sh' <<<"$job" | head -1 | cut -d: -f1 || true)"
[ -n "$check_line" ] && [ -n "$equiv_line" ] && [ "$check_line" -lt "$equiv_line" ] \
  && ok "dependency results are checked before the equivalence test and PASS line" \
  || fail "dependency check missing or after the PASS line"

echo "--- functional: the checker ---"
run() { NEEDS_JSON="$1" python3 -I "$TOOL" >/dev/null 2>&1; }
all_ok='{"test":{"result":"success","outputs":{}},"shell":{"result":"success","outputs":{}}}'
run "$all_ok" && ok "all needed jobs succeeded -> pass" || fail "rejected an all-success graph"
for bad in failure skipped cancelled; do
  doc="{\"test\":{\"result\":\"success\"},\"shell\":{\"result\":\"$bad\"}}"
  if run "$doc"; then fail "accepted a needed job with result $bad"; else ok "a needed job with result '$bad' fails the verdict"; fi
done
for broken in '' 'not json' '{}' '[]' '{"test":"success"}' '{"test":{}}' 'null'; do
  if run "$broken"; then fail "accepted unusable input: ${broken:-<empty>}"; else ok "unusable input fails closed: ${broken:-<empty>}"; fi
done
msg="$(NEEDS_JSON='{"test":{"result":"success"},"shell":{"result":"failure"},"fmt":{"result":"skipped"}}' python3 -I "$TOOL" 2>&1 || true)"
grep -q 'shell' <<<"$msg" && grep -q 'fmt' <<<"$msg" && ! grep -q 'test' <<<"$msg" \
  && ok "the message names exactly the jobs that did not succeed" || fail "message did not name the failing jobs: $msg"

[ "$failures" -eq 0 ] || { echo "ci verdict semantics: $failures failure(s)"; exit 1; }
echo "ci verdict semantics: PASS"
