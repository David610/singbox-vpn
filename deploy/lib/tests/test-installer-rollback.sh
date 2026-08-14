#!/usr/bin/env bash
# Functional regression coverage for the real inherited ERR trap.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/source/deploy/almalinux"
cat > "$TMP/source/deploy/almalinux/uninstall.sh" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$ROLLBACK_LOG"
exit "${ROLLBACK_EXIT:-0}"
MOCK
chmod +x "$TMP/source/deploy/almalinux/uninstall.sh"

cat > "$TMP/trigger.sh" <<'TRIGGER'
#!/usr/bin/env bash
set -Eeuo pipefail
export OWNERSHIP_DIR="$TEST_TMP/state"
export OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
# shellcheck source=/dev/null
. "$TEST_ROOT/deploy/almalinux/install.sh"
ownership_mark INSTALL_ATTEMPTED
IS_FRESH_INSTALL=1
VPN1_NO_AUTO_ROLLBACK=0
VPN1_STAGE=packages
REPO_ROOT="$TEST_TMP/source"

case "${FAIL_SHAPE:-subshell}" in
  # These are the three contexts in which `set -E` inheritance previously
  # caused misleading/repeated handlers. Keep all three as real trap tests.
  subshell) ( exit "${FAIL_CODE:-37}" ) ;;
  substitution) ignored="$(exit "${FAIL_CODE:-37}")" ;;
  function)
    fail_inside_function() { return "${FAIL_CODE:-37}"; }
    fail_inside_function
    ;;
  *) exit 99 ;;
esac
TRIGGER
chmod +x "$TMP/trigger.sh"

export TEST_ROOT="$ROOT" TEST_TMP="$TMP" ROLLBACK_LOG="$TMP/rollback.log" FAIL_CODE=37
for shape in subshell substitution function; do
  : > "$ROLLBACK_LOG"
  export FAIL_SHAPE="$shape"
  rc=0
  bash "$TMP/trigger.sh" >"$TMP/out-$shape" 2>&1 || rc=$?
  [ "$rc" -eq 37 ]
  [ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
  [ "$(cat "$ROLLBACK_LOG")" = "--yes" ]
  [ "$(grep -c 'installation failed:' "$TMP/out-$shape")" -eq 1 ]
  [ "$(grep -c 'rolling back everything' "$TMP/out-$shape")" -eq 1 ]
  grep -q 'stage=packages.*exit=37' "$TMP/out-$shape"
  ! grep -qiE 'prompt|/dev/tty' "$TMP/out-$shape"
done

# Rollback failure must be reported once and must not replace the original
# installer status or recursively invoke rollback.
: > "$ROLLBACK_LOG"
export ROLLBACK_EXIT=19
export FAIL_SHAPE=subshell
rc=0
bash "$TMP/trigger.sh" >"$TMP/fail-out" 2>&1 || rc=$?
[ "$rc" -eq 37 ]
[ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
[ "$(grep -c 'automatic rollback failed once' "$TMP/fail-out")" -eq 1 ]
[ "$(grep -c 'installation failed:' "$TMP/fail-out")" -eq 1 ]

echo "installer rollback regression passed"
