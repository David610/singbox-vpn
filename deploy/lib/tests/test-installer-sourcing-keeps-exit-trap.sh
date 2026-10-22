#!/usr/bin/env bash
# I-74: sourcing install.sh (as many shell tests do) used to replace the
# caller's EXIT trap with its own scratch cleanup, so the test's own temp
# directories were never removed. The caller's trap must still run, and the
# installer's scratch workspace must still be removed.
set -Eeuo pipefail

ACTUAL="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# Case 1: a caller trap set before sourcing still runs at exit.
MARK1="$TMP/caller-trap-ran"
OUT1="$TMP/case1.out"
bash -c '
  trap "touch \"$1\"" EXIT
  REPO_ROOT="$2"
  . "$2/deploy/almalinux/install.sh"
  echo "$INSTALL_WORK" >"$3"
' _ "$MARK1" "$ACTUAL" "$OUT1"
[ -e "$MARK1" ] && ok "the sourcing script's own EXIT trap still runs" || fail "caller EXIT trap was replaced"
WORK1="$(cat "$OUT1")"
[ -n "$WORK1" ] && [ ! -e "$WORK1" ] && ok "the installer scratch workspace is still removed at exit" \
  || fail "installer scratch workspace left behind: $WORK1"

# Case 2: no caller trap -> installer cleanup alone, unchanged behaviour.
OUT2="$TMP/case2.out"
bash -c 'REPO_ROOT="$1"; . "$1/deploy/almalinux/install.sh"; echo "$INSTALL_WORK" >"$2"' _ "$ACTUAL" "$OUT2"
WORK2="$(cat "$OUT2")"
[ -n "$WORK2" ] && [ ! -e "$WORK2" ] && ok "without a caller trap the scratch workspace is removed" \
  || fail "scratch workspace left behind without a caller trap: $WORK2"

# Case 3: a caller trap whose command contains quotes and a path with spaces.
MARK3="$TMP/dir with space/ran"
mkdir -p "$TMP/dir with space"
bash -c '
  trap "touch \"$1\"; echo \"it'"'"'s done\" >/dev/null" EXIT
  REPO_ROOT="$2"
  . "$2/deploy/almalinux/install.sh"
' _ "$MARK3" "$ACTUAL"
[ -e "$MARK3" ] && ok "a quoted caller trap survives chaining intact" || fail "quoted caller trap broken"

[ "$failures" -eq 0 ] || { echo "installer sourcing exit trap: $failures failure(s)"; exit 1; }
echo "installer sourcing exit trap: PASS"
