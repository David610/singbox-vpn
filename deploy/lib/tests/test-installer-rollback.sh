#!/usr/bin/env bash
# Functional regression coverage for the installer's transactional
# fresh-install rollback guarantee: on_fatal_error() (fired either by the
# inherited ERR trap, or now directly by die() — see install.sh's die())
# must reliably roll back a failed FRESH install exactly once, must never
# auto-rollback a failed REPAIR, must respect SINGBOX_VPN_NO_AUTO_ROLLBACK=1, and
# must preserve the original failure's exit code even when rollback itself
# fails.
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
IS_FRESH_INSTALL="${TEST_IS_FRESH_INSTALL:-1}"
SINGBOX_VPN_NO_AUTO_ROLLBACK="${TEST_NO_AUTO_ROLLBACK:-0}"
SINGBOX_VPN_STAGE=packages
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
  # Exercises the real, unmodified die() from install.sh directly — the
  # exact path stage 17's acceptance_stage() takes
  # ('vpn doctor --protocol --require-protocol' failing) — rather than an
  # ordinary command failing on its own and being caught by the ERR trap.
  # `exit` called explicitly inside a function does NOT re-trigger a
  # `trap ... ERR` in bash, so this shape is the one that would have
  # silently skipped rollback before die() was made to call
  # on_fatal_error() itself.
  die) die "simulated post-install acceptance failure (exit ${FAIL_CODE:-37} not preserved by die — always 1)" ;;
  *) exit 99 ;;
esac
TRIGGER
chmod +x "$TMP/trigger.sh"

export TEST_ROOT="$ROOT" TEST_TMP="$TMP" ROLLBACK_LOG="$TMP/rollback.log" FAIL_CODE=37

echo "--- fresh install: ordinary command failure -> rollback ---"
for shape in subshell substitution function; do
  : > "$ROLLBACK_LOG"
  export FAIL_SHAPE="$shape" TEST_IS_FRESH_INSTALL=1 TEST_NO_AUTO_ROLLBACK=0
  unset ROLLBACK_EXIT
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
echo "ok: subshell/substitution/function failures roll back a fresh install exactly once, original exit code preserved"

echo "--- fresh install: explicit die() path -> rollback (the reproduced v0.1.2 bug) ---"
: > "$ROLLBACK_LOG"
export FAIL_SHAPE=die TEST_IS_FRESH_INSTALL=1 TEST_NO_AUTO_ROLLBACK=0
unset ROLLBACK_EXIT
rc=0
bash "$TMP/trigger.sh" >"$TMP/out-die" 2>&1 || rc=$?
[ "$rc" -eq 1 ]
[ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
[ "$(cat "$ROLLBACK_LOG")" = "--yes" ]
[ "$(grep -c 'installation failed:' "$TMP/out-die")" -eq 1 ]
[ "$(grep -c 'rolling back everything' "$TMP/out-die")" -eq 1 ]
grep -q 'ERROR: simulated post-install acceptance failure' "$TMP/out-die"
echo "ok: die() now routes through on_fatal_error and rolls back a fresh install, exactly once"

echo "--- stage-17 acceptance failure via the REAL acceptance_stage() die() call sites -> rollback ---"
: > "$ROLLBACK_LOG"
cat > "$TMP/trigger-acceptance.sh" <<'TRIGGER2'
#!/usr/bin/env bash
set -Eeuo pipefail
export OWNERSHIP_DIR="$TEST_TMP/state"
export OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
# shellcheck source=/dev/null
. "$TEST_ROOT/deploy/almalinux/install.sh"
ownership_mark INSTALL_ATTEMPTED
IS_FRESH_INSTALL=1
SINGBOX_VPN_NO_AUTO_ROLLBACK=0
SINGBOX_VPN_STAGE=acceptance
REPO_ROOT="$TEST_TMP/source"
BIN_DIR="$TEST_TMP/bin"
DEPLOYMENT_TOML="$TEST_TMP/deployment.toml"
PRIOR_ACCEPTANCE_STATE=""
: > "$DEPLOYMENT_TOML"

# Stand in for ensure_first_user (already covered by other tests) and
# vpn-health-check so this test isolates exactly the doctor-failure ->
# die() -> rollback path stage 17 takes in the field incident.
ensure_first_user() { :; }
mkdir -p "$BIN_DIR"
cat > "$BIN_DIR/vpn-health-check" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$BIN_DIR/vpn-health-check"
cat > "$BIN_DIR/vpn" <<'EOF'
#!/usr/bin/env bash
echo "[FAIL] [L2] public hostname \"example.test\" does not resolve after 3 attempt(s): simulated transient resolver failure"
exit 1
EOF
chmod +x "$BIN_DIR/vpn"

acceptance_stage
TRIGGER2
chmod +x "$TMP/trigger-acceptance.sh"
rc=0
bash "$TMP/trigger-acceptance.sh" >"$TMP/out-acceptance" 2>&1 || rc=$?
[ "$rc" -eq 1 ]
[ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
[ "$(cat "$ROLLBACK_LOG")" = "--yes" ]
grep -q 'post-install acceptance check' "$TMP/out-acceptance"
grep -q 'rolling back everything' "$TMP/out-acceptance"
echo "ok: acceptance_stage()'s real die() call site rolls back a failed fresh install"

echo "--- repair install: same failure shapes -> NO destructive rollback ---"
for shape in subshell die; do
  : > "$ROLLBACK_LOG"
  export FAIL_SHAPE="$shape" TEST_IS_FRESH_INSTALL=0 TEST_NO_AUTO_ROLLBACK=0
  unset ROLLBACK_EXIT
  rc=0
  bash "$TMP/trigger.sh" >"$TMP/out-repair-$shape" 2>&1 || rc=$?
  [ ! -s "$ROLLBACK_LOG" ]
  grep -q 'NOT auto-rolling-back' "$TMP/out-repair-$shape"
  [ "$shape" = "die" ] && [ "$rc" -eq 1 ]
  [ "$shape" = "subshell" ] && [ "$rc" -eq 37 ]
done
echo "ok: a repair-run failure (IS_FRESH_INSTALL=0) never invokes the uninstaller, for both trap-caught and die()-caught failures"

echo "--- SINGBOX_VPN_NO_AUTO_ROLLBACK=1 -> NO rollback, debugging escape hatch preserved ---"
for shape in subshell die; do
  : > "$ROLLBACK_LOG"
  export FAIL_SHAPE="$shape" TEST_IS_FRESH_INSTALL=1 TEST_NO_AUTO_ROLLBACK=1
  unset ROLLBACK_EXIT
  rc=0
  bash "$TMP/trigger.sh" >"$TMP/out-noauto-$shape" 2>&1 || rc=$?
  [ ! -s "$ROLLBACK_LOG" ]
  grep -q 'SINGBOX_VPN_NO_AUTO_ROLLBACK=1 set' "$TMP/out-noauto-$shape"
done
echo "ok: SINGBOX_VPN_NO_AUTO_ROLLBACK=1 disables automatic rollback for both trap-caught and die()-caught failures"

echo "--- rollback runs exactly once: the ROLLBACK_HANDLER_ACTIVE re-entrancy guard itself ---"
# Directly pins the guard that makes "exactly once" true regardless of
# which path (ERR trap or die()) reaches on_fatal_error: if the handler
# is already active (e.g. a nested failure while rollback itself is
# running), a second on_fatal_error call at the SAME root PID must
# return immediately without invoking the uninstaller again.
: > "$ROLLBACK_LOG"
cat > "$TMP/trigger-reentrant.sh" <<'TRIGGER3'
#!/usr/bin/env bash
set -Eeuo pipefail
export OWNERSHIP_DIR="$TEST_TMP/state"
export OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
# shellcheck source=/dev/null
. "$TEST_ROOT/deploy/almalinux/install.sh"
ownership_mark INSTALL_ATTEMPTED
IS_FRESH_INSTALL=1
SINGBOX_VPN_NO_AUTO_ROLLBACK=0
SINGBOX_VPN_STAGE=packages
REPO_ROOT="$TEST_TMP/source"
# Simulate the handler already being mid-rollback (as it would be for
# the real second entry a recursive/nested fatal error could cause) and
# confirm the guard makes the second call a no-op instead of a second
# uninstall run.
ROLLBACK_HANDLER_ACTIVE=1
# `|| true`: on_fatal_error's guarded return is itself a nonzero "command"
# result under this script's own inherited `set -e` — guard the call so
# THIS test harness doesn't itself exit on that return before the
# assertion below runs; the guard behavior under test (did it exit vs.
# return, did it re-invoke the uninstaller) is unaffected either way.
on_fatal_error 42 99 nested_failure packages || true
echo "on_fatal_error returned (did not exit) with guard active, as expected"
TRIGGER3
chmod +x "$TMP/trigger-reentrant.sh"
rc=0
bash "$TMP/trigger-reentrant.sh" >"$TMP/out-reentrant" 2>&1 || rc=$?
[ "$rc" -eq 0 ]
[ ! -s "$ROLLBACK_LOG" ]
grep -q 'on_fatal_error returned' "$TMP/out-reentrant"
echo "ok: on_fatal_error's ROLLBACK_HANDLER_ACTIVE guard prevents a second rollback when already active, regardless of which caller (ERR trap or die()) reaches it"

echo "--- rollback failure is reported alongside the original failure, root cause not masked ---"
: > "$ROLLBACK_LOG"
export ROLLBACK_EXIT=19
export FAIL_SHAPE=die TEST_IS_FRESH_INSTALL=1 TEST_NO_AUTO_ROLLBACK=0
rc=0
bash "$TMP/trigger.sh" >"$TMP/out-rollback-fail" 2>&1 || rc=$?
[ "$rc" -eq 1 ]
[ "$(wc -l < "$ROLLBACK_LOG")" -eq 1 ]
[ "$(grep -c 'automatic rollback failed once' "$TMP/out-rollback-fail")" -eq 1 ]
[ "$(grep -c 'ERROR: simulated post-install acceptance failure' "$TMP/out-rollback-fail")" -eq 1 ]
unset ROLLBACK_EXIT
echo "ok: a rollback failure via the die() path is reported without masking the original error"

echo "installer rollback regression passed"
