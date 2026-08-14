#!/usr/bin/env bash
# Functional regression coverage for the fresh-install ERR handler.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export OWNERSHIP_DIR="$TMP/state"
export OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"

# shellcheck source=/dev/null
. "$ROOT/deploy/almalinux/install.sh"
trap - ERR
ownership_mark INSTALL_ATTEMPTED
IS_FRESH_INSTALL=1
VPN1_NO_AUTO_ROLLBACK=0
VPN1_STAGE=packages
REPO_ROOT="$TMP/source"
mkdir -p "$REPO_ROOT/deploy/almalinux"
cat > "$REPO_ROOT/deploy/almalinux/uninstall.sh" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$ROLLBACK_LOG"
exit "${ROLLBACK_EXIT:-0}"
MOCK
chmod +x "$REPO_ROOT/deploy/almalinux/uninstall.sh"
export ROLLBACK_LOG="$TMP/rollback.log"

rc=0
( on_fatal_error 37 424 persist_source_tree packages ) >"$TMP/out" 2>&1 || rc=$?
[ "$rc" -eq 37 ]
[ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
[ "$(cat "$ROLLBACK_LOG")" = "--yes" ]
grep -q 'stage=packages function=persist_source_tree line=424.*exit=37' "$TMP/out"
! grep -qiE 'prompt|/dev/tty' "$TMP/out"

# Rollback failure must be reported once and must not replace the original
# installer status or recursively invoke rollback.
: > "$ROLLBACK_LOG"
export ROLLBACK_EXIT=19
rc=0
( on_fatal_error 37 425 packages_stage packages ) >"$TMP/fail-out" 2>&1 || rc=$?
[ "$rc" -eq 37 ]
[ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
[ "$(grep -c 'automatic rollback failed once' "$TMP/fail-out")" -eq 1 ]

echo "installer rollback regression passed"
