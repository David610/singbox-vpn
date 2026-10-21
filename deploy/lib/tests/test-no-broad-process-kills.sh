#!/usr/bin/env bash
# Probe/lab/acceptance tooling must clean up only processes it owns (tracked
# PIDs / process groups). A name- or pattern-based kill (`pkill -f`, `killall`)
# can terminate an unrelated service on the host it runs against — including
# the production sing-box a probe was started beside — so none may be added to
# the shipped scripts or workflows.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

# This file and deploy/lib/test-isolation.sh (which lists such commands in
# order to NEUTRALISE them inside test sandboxes) are the only mentions allowed.
# `pkill -u USER` (kill by owning user, as printed in uninstall advice) is a
# narrow, user-scoped form and is not flagged; `-f` pattern kills and bare
# name kills are.
# `find` rather than `git ls-files`: this must work (and fail closed) in an
# exported tree without .git, e.g. a release archive or a container copy.
hits="$(find deploy .github bin install.sh uninstall.sh -type f 2>/dev/null   | grep -vE '^deploy/lib/(tests/test-no-broad-process-kills\.sh|test-isolation\.sh)$'   | while IFS= read -r f; do
      grep -HnE '(^|[^[:alnum:]_.-])(killall([[:space:]]|$)|pkill[[:space:]]+(-[A-Za-z]*f|[^-[:space:]]))' "$f" || true
    done)"
[ "$(find deploy .github -type f | wc -l)" -gt 20 ] \
  || { echo "FAIL: scanned suspiciously few files; refusing a vacuous pass" >&2; exit 1; }
if [ -n "$hits" ]; then
  echo "FAIL: broad process-kill commands found (track and kill owned PIDs instead):" >&2
  echo "$hits" >&2
  exit 1
fi

# Discovery has the same failure mode as killing: `pgrep -f foo` matches any process whose argv merely CONTAINS foo -- the
# calling shell, its watcher loop, an editor or a tail -- so "is the service still running?" can be wrong in both
# directions and a PID taken from it can be the wrong process. Allowed: a PID this script started itself, systemd's
# MainPID, `pgrep -x NAME` (exact comm), an ANCHORED `pgrep -x -f '^...$'`, or deploy/lib/proc-safe.sh (proc_pids_exact).
find_hits="$(find deploy bin install.sh uninstall.sh -type f -name '*.sh' 2>/dev/null   | grep -vE '^deploy/lib/tests/test-no-broad-process-kills\.sh$'   | while IFS= read -r f; do
      grep -HnE '(^|[^[:alnum:]_.-])pgrep[[:space:]]+(-[A-Za-z]*[[:space:]]+)*-f([[:space:]]|$)' "$f" | grep -vE "pgrep -x -f '\^" || true
    done)"
if [ -n "$find_hits" ]; then
  echo "FAIL: unanchored pgrep -f (use an exact PID, MainPID, pgrep -x, an anchored pattern or proc-safe.sh):" >&2
  echo "$find_hits" >&2
  exit 1
fi
echo "PASS: test-no-broad-process-kills.sh (no pkill/killall, no unanchored pgrep -f in shipped scripts or workflows)"
