#!/usr/bin/env bash
# Unit tests for deploy/lib/preflight.sh's preflight_curl_retry() — the
# shared bounded-retry curl wrapper used by preflight_check_connectivity
# and preflight_detect_public_ip so a single transient network blip does
# not hard-abort the whole installer, while still failing closed once
# retries are exhausted. Stubs `curl` entirely — no real network access,
# no root required.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LIB_DIR="$REPO_ROOT/deploy/lib"

log() { :; }
warn() { :; }
die() { echo "[test-die] $*" >&2; return 1; }

# shellcheck source=deploy/lib/preflight.sh
. "$LIB_DIR/preflight.sh"

failures=0
assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $desc — expected [$expected], got [$actual]"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

echo "--- preflight_curl_retry(): first attempt succeeds ---"
CURL_CALLS_FILE="$TMPDIR_TEST/calls-1"
: > "$CURL_CALLS_FILE"
curl() {
  echo "$*" >> "$CURL_CALLS_FILE"
  return 0
}
export -f curl
(
  # shellcheck disable=SC2034  # consumed by preflight_curl_retry() in the sourced lib
  CURL_NET_FLAGS=(--connect-timeout 1 --max-time 1)
  preflight_curl_retry -fsS -o /dev/null "https://example.test/ok"
)
assert_eq "curl invoked exactly once when the first attempt succeeds" "1" "$(wc -l < "$CURL_CALLS_FILE" | tr -d ' ')"

echo
echo "--- preflight_curl_retry(): first attempt fails, IPv4-fallback attempt succeeds ---"
CURL_CALLS_FILE="$TMPDIR_TEST/calls-2"
: > "$CURL_CALLS_FILE"
CURL_CALL_COUNT_FILE="$TMPDIR_TEST/count-2"
echo 0 > "$CURL_CALL_COUNT_FILE"
curl() {
  echo "$*" >> "$CURL_CALLS_FILE"
  local n
  n="$(cat "$CURL_CALL_COUNT_FILE")"
  n=$((n + 1))
  echo "$n" > "$CURL_CALL_COUNT_FILE"
  # Fail the first (normal) attempt, succeed on the second (the -4
  # fallback attempt) — asserts the fallback is a SECOND attempt, not
  # the only mode ever tried.
  [ "$n" -ge 2 ]
}
export -f curl
rc=0
(
  # shellcheck disable=SC2034  # consumed by preflight_curl_retry() in the sourced lib
  CURL_NET_FLAGS=(--connect-timeout 1 --max-time 1)
  preflight_curl_retry -fsS -o /dev/null "https://example.test/retry"
) || rc=$?
assert_eq "preflight_curl_retry succeeds once the IPv4-fallback attempt succeeds" "0" "$rc"
assert_eq "curl invoked exactly twice (normal attempt, then IPv4 fallback)" "2" "$(wc -l < "$CURL_CALLS_FILE" | tr -d ' ')"
if grep -q -- '^-4 ' "$CURL_CALLS_FILE"; then
  echo "ok: the fallback attempt uses -4 (IPv4-preferring), not as the only mode"
else
  echo "FAIL: no fallback attempt used -4 — expected exactly one of the two calls to"
  cat "$CURL_CALLS_FILE"
  failures=$((failures + 1))
fi
first_call="$(head -n1 "$CURL_CALLS_FILE")"
if [[ "$first_call" != -4* ]]; then
  echo "ok: the FIRST attempt does not force -4 (a working IPv6-only/dual-stack host is never broken by forcing IPv4 as the only mode)"
else
  echo "FAIL: the first attempt already forced -4 — IPv6 hosts would be broken unnecessarily"
  failures=$((failures + 1))
fi

echo
echo "--- preflight_curl_retry(): both attempts fail => hard failure (fail-closed) ---"
CURL_CALLS_FILE="$TMPDIR_TEST/calls-3"
: > "$CURL_CALLS_FILE"
curl() {
  echo "$*" >> "$CURL_CALLS_FILE"
  return 7
}
export -f curl
rc=0
(
  # shellcheck disable=SC2034  # consumed by preflight_curl_retry() in the sourced lib
  CURL_NET_FLAGS=(--connect-timeout 1 --max-time 1)
  preflight_curl_retry -fsS -o /dev/null "https://example.test/always-fails"
) || rc=$?
assert_eq "preflight_curl_retry fails (non-zero) once both attempts are exhausted" "1" "$([ "$rc" -ne 0 ] && echo 1 || echo 0)"
assert_eq "curl invoked exactly twice before giving up" "2" "$(wc -l < "$CURL_CALLS_FILE" | tr -d ' ')"

echo
echo "--- preflight_check_connectivity(): a single transient failure does not hard-abort (retry succeeds) ---"
CURL_CALLS_FILE="$TMPDIR_TEST/calls-4"
: > "$CURL_CALLS_FILE"
CURL_CALL_COUNT_FILE="$TMPDIR_TEST/count-4"
echo 0 > "$CURL_CALL_COUNT_FILE"
curl() {
  echo "$*" >> "$CURL_CALLS_FILE"
  local n
  n="$(cat "$CURL_CALL_COUNT_FILE")"
  n=$((n + 1))
  echo "$n" > "$CURL_CALL_COUNT_FILE"
  [ "$n" -ge 2 ]
}
export -f curl
rc=0
(
  # shellcheck disable=SC2034  # consumed by preflight_curl_retry() in the sourced lib
  CURL_NET_FLAGS=(--connect-timeout 1 --max-time 1)
  preflight_check_connectivity "https://example.test/flaky"
) || rc=$?
assert_eq "preflight_check_connectivity does not hard-abort on one transient failure" "0" "$rc"

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
