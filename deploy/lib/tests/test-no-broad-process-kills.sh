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
echo "PASS: test-no-broad-process-kills.sh (no pkill/killall in shipped scripts or workflows)"
