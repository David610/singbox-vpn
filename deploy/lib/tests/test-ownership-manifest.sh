#!/usr/bin/env bash
# Unit tests for deploy/lib/ownership.sh — the incremental ownership
# manifest install.sh writes (starting at stage 1, not only at the end)
# and uninstall.sh reads to decide exactly what is safe to remove or
# must be restored. Exercises the real functions against a throwaway
# directory; no root required.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
OWNERSHIP_SH="$REPO_ROOT/deploy/lib/ownership.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

log() { :; }
warn() { :; }

OWNERSHIP_DIR="$TMPDIR_TEST/var-lib-singbox-vpn"
OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
# shellcheck source=/dev/null
. "$OWNERSHIP_SH"

echo "--- ownership_init: creates the state dir and an empty file, mode 0600 ---"
ownership_init
if [ -d "$OWNERSHIP_DIR" ] && [ -f "$OWNERSHIP_FILE" ]; then
  ok "ownership_init created $OWNERSHIP_DIR and $OWNERSHIP_FILE"
else
  fail "ownership_init did not create the expected paths"
fi
perm="$(stat -c '%a' "$OWNERSHIP_FILE" 2>/dev/null || stat -f '%Lp' "$OWNERSHIP_FILE")"
[ "$perm" = "600" ] && ok "ownership file is 0600 (secrets-adjacent metadata, root-only)" || fail "ownership file mode is $perm, expected 600"

echo
echo "--- ownership_set / ownership_get: round-trips a value ---"
ownership_set "TEST_KEY" "hello"
[ "$(ownership_get TEST_KEY)" = "hello" ] && ok "value round-trips" || fail "value did not round-trip"

echo
echo "--- ownership_set: last write wins, does not duplicate lines ---"
ownership_set "TEST_KEY" "world"
[ "$(ownership_get TEST_KEY)" = "world" ] && ok "overwrite takes effect" || fail "overwrite did not take effect"
count="$(grep -c '^TEST_KEY=' "$OWNERSHIP_FILE")"
[ "$count" -eq 1 ] || fail "TEST_KEY appears $count times, expected exactly 1 (no duplicate/stale lines)"
[ "$count" -eq 1 ] && ok "no duplicate lines after overwrite"

echo
echo "--- ownership_get: missing key returns the default, not an error ---"
[ "$(ownership_get NO_SUCH_KEY "fallback")" = "fallback" ] && ok "missing key returns default" || fail "missing key did not return default"
[ -z "$(ownership_get NO_SUCH_KEY)" ] && ok "missing key with no default returns empty" || fail "missing key with no default did not return empty"
[ "$(ownership_is_marked NO_SUCH_KEY)" = "0" ] && ok "missing mark returns 0 with success status" || fail "missing mark result/status is wrong"
[ -z "$(ownership_list_get NO_SUCH_KEY)" ] && ok "missing list returns empty with success status" || fail "missing list result/status is wrong"

echo "--- empty manifest: every optional read is safe under set -Eeuo pipefail ---"
: > "$OWNERSHIP_FILE"
strict_out="$(
  set -Eeuo pipefail
  printf '%s|%s|%s' "$(ownership_get MISSING fallback)" "$(ownership_is_marked MISSING)" "$(ownership_list_get MISSING)"
)"
[ "$strict_out" = "fallback|0|" ] && ok "empty manifest optional reads return defaults without aborting" || fail "empty manifest strict read returned '$strict_out'"
ownership_set "TEST_KEY" "world"

echo
echo "--- ownership_mark / ownership_is_marked: boolean facts ---"
if [ "$(ownership_is_marked USER_SINGBOX_CREATED)" = "1" ]; then
  fail "USER_SINGBOX_CREATED reported marked before it was ever set"
else
  ok "unset boolean fact correctly reports as not marked"
fi
ownership_mark USER_SINGBOX_CREATED
[ "$(ownership_is_marked USER_SINGBOX_CREATED)" = "1" ] && ok "marked boolean fact reports as marked" || fail "ownership_mark did not take effect"

echo
echo "--- ownership_list_add / ownership_list_get: de-duplicated space-separated list ---"
ownership_list_add PKGS_INSTALLED_BY_SINGBOX_VPN "nginx"
ownership_list_add PKGS_INSTALLED_BY_SINGBOX_VPN "certbot"
ownership_list_add PKGS_INSTALLED_BY_SINGBOX_VPN "nginx" # duplicate, must not appear twice
list="$(ownership_list_get PKGS_INSTALLED_BY_SINGBOX_VPN)"
case " $list " in
  *" nginx "*) ok "list contains nginx" ;;
  *) fail "list missing nginx: '$list'" ;;
esac
case " $list " in
  *" certbot "*) ok "list contains certbot" ;;
  *) fail "list missing certbot: '$list'" ;;
esac
nginx_count="$(echo " $list " | grep -o ' nginx ' | wc -l)"
[ "$nginx_count" -eq 1 ] && ok "nginx appears exactly once despite being added twice (de-duplicated)" || fail "nginx appears $nginx_count times, expected 1"

echo
echo "--- ownership_set_baseline_once: first write wins, later calls are no-ops ---"
ownership_set_baseline_once BASELINE_KEY "original"
ownership_set_baseline_once BASELINE_KEY "should-be-ignored"
[ "$(ownership_get BASELINE_KEY)" = "original" ] && ok "baseline value from the FIRST call is preserved, not overwritten by a later run" || fail "baseline was overwritten by a subsequent call — this would corrupt pre-singbox-vpn state tracking on a repair re-run"

echo
echo "--- persistence: values survive re-sourcing (simulates a separate uninstall.sh process reading the same file) ---"
(
  OWNERSHIP_DIR="$OWNERSHIP_DIR"
  OWNERSHIP_FILE="$OWNERSHIP_FILE"
  # shellcheck source=/dev/null
  . "$OWNERSHIP_SH"
  [ "$(ownership_get TEST_KEY)" = "world" ] || { echo "FAIL: value did not persist across a fresh source of ownership.sh"; exit 1; }
  [ "$(ownership_is_marked USER_SINGBOX_CREATED)" = "1" ] || { echo "FAIL: boolean fact did not persist across a fresh source of ownership.sh"; exit 1; }
  echo "PERSISTED_OK"
) | grep -q PERSISTED_OK && ok "manifest values persist across a fresh process re-sourcing ownership.sh (as uninstall.sh does)" || fail "manifest values did not persist for a fresh reader"

echo
echo "--- ownership_path_is_safe(): refuses obviously unsafe manifest-sourced paths before destructive use ---"
for good in "/root" "/opt/singbox-vpn" "/home/someuser/.rustup-parent"; do
  ownership_path_is_safe "$good" && ok "accepts safe absolute path '$good'" || fail "wrongly rejected safe path '$good'"
done
for bad in "" "/" "relative/path" "../etc" "/root/../.." "/a/../../etc"; do
  if ownership_path_is_safe "$bad"; then
    fail "wrongly accepted unsafe path '$bad'"
  else
    ok "rejects unsafe path '$bad'"
  fi
done

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all ownership-manifest tests passed"
