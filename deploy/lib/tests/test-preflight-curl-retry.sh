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
echo "--- policy: production CURL_NET_FLAGS get real exponential backoff, not a fixed delay ---"
# preflight_curl_retry()'s own internal retry loop is exactly 2 attempts
# (normal, then one -4 fallback) — already exercised above. The actual
# many-attempt resilience against a flaky host comes from curl's OWN
# internal --retry handling *within* each of those two attempts, which
# only backs off exponentially (~1s/2s/4s/8s/16s) when --retry-delay is
# NOT set (a fixed --retry-delay overrides curl's default exponential
# schedule with a constant gap). A bash mock cannot exercise curl's own
# internal retry loop without reimplementing curl, so this is a static
# assertion on the shipped policy in every production caller rather than
# a dynamic mock — it is what makes a "several refusals then success"
# host actually recover in practice.
check_retry_policy() {
  local label="$1" file="$2" line
  line="$(grep -m1 '^CURL_NET_FLAGS=' "$file" || true)"
  if [ -z "$line" ]; then
    fail_policy "$label: no CURL_NET_FLAGS= definition found in $file"
    return
  fi
  if [[ "$line" == *"--retry-delay"* ]]; then
    fail_policy "$label: CURL_NET_FLAGS sets a fixed --retry-delay, which overrides curl's own exponential backoff"
    return
  fi
  local retry_n
  retry_n="$(sed -n 's/.*--retry \([0-9]\+\).*/\1/p' <<<"$line")"
  if [ -z "$retry_n" ] || [ "$retry_n" -lt 5 ]; then
    fail_policy "$label: --retry count is '${retry_n:-<missing>}', expected >= 5"
    return
  fi
  ok_policy "$label: --retry $retry_n with no fixed --retry-delay (curl's default exponential backoff applies)"
}
ok_policy() { echo "ok: $*"; }
fail_policy() { echo "FAIL: $*"; failures=$((failures + 1)); }
check_retry_policy "root install.sh" "$REPO_ROOT/install.sh"
check_retry_policy "root uninstall.sh" "$REPO_ROOT/uninstall.sh"
check_retry_policy "deploy/almalinux/install.sh" "$REPO_ROOT/deploy/almalinux/install.sh"
check_retry_policy "deploy/almalinux/update.sh" "$REPO_ROOT/deploy/almalinux/update.sh"
# deploy/almalinux/install.sh relies on preflight.sh auto-appending
# --retry-connrefused (see preflight.sh's `CURL_NET_FLAGS+=(--retry-connrefused)`
# idempotent guard) rather than listing it inline; the other three list it
# inline. Either is fine as long as it ends up present at runtime.
(
  CURL_NET_FLAGS=(--connect-timeout 10 --max-time 300 --speed-limit 1024 --speed-time 30 --retry 5)
  log() { :; }; warn() { :; }; die() { return 1; }
  # shellcheck source=deploy/lib/preflight.sh
  . "$LIB_DIR/preflight.sh"
  case " ${CURL_NET_FLAGS[*]} " in
    *" --retry-connrefused "*) ok_policy "deploy/almalinux/install.sh: preflight.sh augments CURL_NET_FLAGS with --retry-connrefused at source time" ;;
    *) fail_policy "deploy/almalinux/install.sh: preflight.sh did not augment CURL_NET_FLAGS with --retry-connrefused" ;;
  esac
)

echo
echo "--- control-flow: a failed download never reaches checksum verification or extraction with a partial file ---"
INSTALL_SH_ALMALINUX="$REPO_ROOT/deploy/almalinux/install.sh"
download_line="$(grep -n 'die "download failed: \$url"' "$INSTALL_SH_ALMALINUX" | head -n1 | cut -d: -f1)"
checksum_line="$(grep -n 'checksum verification failed for \$tarball' "$INSTALL_SH_ALMALINUX" | head -n1 | cut -d: -f1)"
extract_line="$(grep -n 'tar -xzf "\$tmpdir/\$tarball"' "$INSTALL_SH_ALMALINUX" | head -n1 | cut -d: -f1)"
if [ -n "$download_line" ] && [ -n "$checksum_line" ] && [ -n "$extract_line" ] \
    && [ "$download_line" -lt "$checksum_line" ] && [ "$checksum_line" -lt "$extract_line" ]; then
  ok_policy "install_singbox() dies on download failure strictly before checksum verification and extraction — a partial/failed download is never treated as valid input"
else
  fail_policy "install_singbox() control-flow ordering changed — a failed download might now reach checksum verification or extraction (download:${download_line:-?} checksum:${checksum_line:-?} extract:${extract_line:-?})"
fi
if grep -q 'network_diagnose_download_failure github.com >&2' "$INSTALL_SH_ALMALINUX" \
    && grep -B2 'network_diagnose_download_failure github.com >&2' "$INSTALL_SH_ALMALINUX" | grep -q 'preflight_curl_retry -fsSL -o "\$tmpdir/\$tarball" "\$url"'; then
  ok_policy "the pinned sing-box tarball download is routed through preflight_curl_retry (retry + IPv4 fallback), with non-fatal diagnostics only after it is exhausted"
else
  fail_policy "the pinned sing-box tarball download no longer uses preflight_curl_retry with diagnostics-on-exhaustion"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
