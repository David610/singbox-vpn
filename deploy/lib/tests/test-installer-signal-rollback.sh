#!/usr/bin/env bash
# Functional regression coverage for SIGINT/SIGTERM during a fresh
# install: before this, install.sh had no handler for either signal
# outside a narrow window during certbot ACME issuance, so bash's
# default disposition (terminate immediately) applied everywhere else
# -- which does NOT fire the `trap ... ERR` on_fatal_error() rollback
# relies on. A Ctrl-C (or `systemctl stop`/orchestrator kill) during a
# real mutating stage silently left the host partially mutated with no
# automatic rollback at all. on_interrupt()/on_terminate() (global INT/
# TERM traps installed alongside the existing ERR trap) close that gap.
set -Eeuo pipefail
# Monitor mode (job control) matters here, not just habit: bash gives an
# asynchronous ("cmd &") child SIGINT/SIGQUIT pre-set to ignored when job
# control is OFF (the default for a non-interactive script), and — per
# POSIX — a signal that is already ignored when a non-interactive shell
# starts can never be trapped away from inside that shell. Without `set
# -m` here, the backgrounded trigger.sh below would silently be unable
# to install its own SIGINT trap no matter what install.sh does, and
# `kill -INT` on it would hang forever waiting for a process that will
# never react. This is a property of how THIS TEST launches its subject
# process, not of install.sh itself — a normal foreground invocation
# (a human's Ctrl-C, or `kill -INT`/`kill -TERM` on a directly-run
# install.sh) never inherits SIGINT as ignored in the first place.
set -m

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/source/deploy/almalinux"
cat > "$TMP/source/deploy/almalinux/uninstall.sh" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$ROLLBACK_LOG"
exit 0
MOCK
chmod +x "$TMP/source/deploy/almalinux/uninstall.sh"

# Sources the real install.sh (for on_interrupt/on_terminate/
# on_fatal_error and the global trap setup exactly as shipped), marks a
# fresh install as already underway, then blocks in the middle of a
# simulated mutating stage so the test can deliver a real signal at a
# known point -- not a `die()`/nonzero-exit simulation, which
# test-installer-rollback.sh already covers.
cat > "$TMP/trigger.sh" <<'TRIGGER'
#!/usr/bin/env bash
set -Eeuo pipefail
export OWNERSHIP_DIR="$TEST_TMP/state"
export OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
# shellcheck source=/dev/null
. "$TEST_ROOT/deploy/almalinux/install.sh"
ownership_mark INSTALL_ATTEMPTED
IS_FRESH_INSTALL=1
REPO_ROOT="$TEST_TMP/source"
SINGBOX_VPN_STAGE=packages
echo "TRIGGER_READY"
# A single long `sleep N` would delay trap delivery until that command
# completes (bash only runs a pending trap between commands, not while
# blocked inside one) -- a tight loop of short sleeps keeps the signal
# response bounded to that short interval instead of the whole 30s.
while :; do sleep 0.05; done
TRIGGER
chmod +x "$TMP/trigger.sh"

export TEST_ROOT="$ROOT" TEST_TMP="$TMP" ROLLBACK_LOG="$TMP/rollback.log"

wait_for_ready() {
  local out="$1" tries=0
  while [ "$tries" -lt 50 ]; do
    grep -q '^TRIGGER_READY$' "$out" 2>/dev/null && return 0
    sleep 0.1
    tries=$((tries + 1))
  done
  return 1
}

echo "--- SIGTERM during a fresh install mid-stage -> rollback, exit 143 ---"
: > "$ROLLBACK_LOG"
out="$TMP/out-term"
: > "$out"
bash "$TMP/trigger.sh" > "$out" 2>&1 &
pid=$!
wait_for_ready "$out" || { echo "FAIL: trigger process never reached TRIGGER_READY"; exit 1; }
kill -TERM "$pid"
rc=0
wait "$pid" || rc=$?
[ "$rc" -eq 143 ] || { echo "FAIL: expected exit 143 after SIGTERM, got $rc"; cat "$out"; exit 1; }
[ "$(cat "$ROLLBACK_LOG" 2>/dev/null)" = "--yes" ] || { echo "FAIL: SIGTERM did not trigger the fresh-install rollback (mock uninstall.sh was not invoked with --yes)"; cat "$out"; exit 1; }
grep -q "rolling back everything" "$out" || { echo "FAIL: rollback log message missing after SIGTERM"; cat "$out"; exit 1; }
grep -q "stage=packages" "$out" || { echo "FAIL: reported stage missing/wrong after SIGTERM"; cat "$out"; exit 1; }
echo "ok: SIGTERM mid-stage rolls back a fresh install and exits 143"

echo "--- SIGINT during a fresh install mid-stage -> rollback, exit 130 ---"
: > "$ROLLBACK_LOG"
out="$TMP/out-int"
: > "$out"
bash "$TMP/trigger.sh" > "$out" 2>&1 &
pid=$!
wait_for_ready "$out" || { echo "FAIL: trigger process never reached TRIGGER_READY"; exit 1; }
kill -INT "$pid"
rc=0
wait "$pid" || rc=$?
[ "$rc" -eq 130 ] || { echo "FAIL: expected exit 130 after SIGINT, got $rc"; cat "$out"; exit 1; }
[ "$(cat "$ROLLBACK_LOG" 2>/dev/null)" = "--yes" ] || { echo "FAIL: SIGINT did not trigger the fresh-install rollback"; cat "$out"; exit 1; }
echo "ok: SIGINT mid-stage rolls back a fresh install and exits 130"

echo "--- static: the ACME issuance window restores the global handler instead of clearing it (trap - INT TERM regression) ---"
if grep -q "trap - INT TERM" "$ROOT/deploy/almalinux/install.sh"; then
  echo "FAIL: install.sh still clears INT/TERM to no handler somewhere (trap - INT TERM) instead of restoring on_interrupt/on_terminate -- a signal after that point would again skip rollback"
  exit 1
fi
echo "ok: no bare 'trap - INT TERM' remains in install.sh"

echo "installer signal-rollback regression passed"
