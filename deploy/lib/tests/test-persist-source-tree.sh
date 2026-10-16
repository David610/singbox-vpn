#!/usr/bin/env bash
# Regression for the real-VPS 0775 source metadata persistence failure.
set -Eeuo pipefail

REPO_ROOT_ACTUAL="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
SRC="$TMP/source"
mkdir -p "$SRC/deploy/almalinux" "$SRC/nested"
chmod 0775 "$SRC" "$SRC/nested"
printf '#!/usr/bin/env bash\nexit 0\n' > "$SRC/deploy/almalinux/uninstall.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$SRC/deploy/almalinux/install.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$SRC/nested/executable.sh"
printf 'ordinary\n' > "$SRC/nested/file.txt"
chmod 0775 "$SRC/deploy" "$SRC/deploy/almalinux"
chmod 0775 "$SRC/deploy/almalinux/uninstall.sh" "$SRC/deploy/almalinux/install.sh" "$SRC/nested/executable.sh"
chmod 0664 "$SRC/nested/file.txt"

# Source production functions without executing main, then redirect the
# canonical destination to this root-owned throwaway path.
REPO_ROOT="$SRC"
# shellcheck source=/dev/null
. "$REPO_ROOT_ACTUAL/deploy/almalinux/install.sh"
REPO_ROOT="$SRC"
PERSIST_DIR="$TMP/opt/singbox-vpn"
PERSIST_PREV_DIR="$TMP/opt/singbox-vpn.prev-install"
persist_source_tree

[ -x "$PERSIST_DIR/nested/executable.sh" ]
[ -f "$PERSIST_DIR/nested/file.txt" ]
[ -z "$(find "$PERSIST_DIR" -perm /022 -print -quit)" ]
if [ "$(id -u)" -eq 0 ]; then
  [ -z "$(find "$PERSIST_DIR" ! -user root -print -quit)" ]
fi

# Exercise the uninstaller's exact canonical-path trust arithmetic against
# both the tree root and persisted executable.
for path in "$PERSIST_DIR" "$PERSIST_DIR/deploy/almalinux/uninstall.sh"; do
  mode="$(stat -c '%a' "$path")"
  group_digit="${mode: -2:1}"
  other_digit="${mode: -1:1}"
  [ $((group_digit & 2)) -eq 0 ]
  [ $((other_digit & 2)) -eq 0 ]
done

# I-16: the tree that is built is always the REQUESTED one.
# (1) identical content is reused, nothing is swapped.
IDENTICAL="$TMP/identical-source"
cp -a "$SRC" "$IDENTICAL"
REPO_ROOT="$IDENTICAL"
persist_source_tree
[ "$REPO_ROOT" = "$PERSIST_DIR" ]
[ ! -e "$PERSIST_PREV_DIR" ]
echo "ok: identical source is reused without a swap"

# Credentials and configuration live outside the source tree and must survive.
mkdir -p "$TMP/etc/vpn"
printf 'secret-state\n' > "$TMP/etc/vpn/users.json"

# (2) different content REPLACES the persistent copy (the old behaviour built
# the stale tree and reported success for the requested ref).
SECOND="$TMP/second-source"
cp -a "$SRC" "$SECOND"
printf 'new-revision\n' > "$SECOND/nested/file.txt"
REPO_ROOT="$SECOND"
persist_source_tree
[ "$REPO_ROOT" = "$PERSIST_DIR" ]
[ "$(cat "$PERSIST_DIR/nested/file.txt")" = "new-revision" ]
[ "$(cat "$PERSIST_PREV_DIR/nested/file.txt")" = "ordinary" ]
[ "$(source_tree_digest "$PERSIST_DIR")" = "$(source_tree_digest "$SECOND")" ]
[ -z "$(find "$PERSIST_DIR" -perm /022 -print -quit)" ]
[ "$(cat "$TMP/etc/vpn/users.json")" = "secret-state" ]
echo "ok: a different requested source replaces the persistent copy"

# (3) once the run is accepted the previous tree is dropped.
SOURCE_SWAP_PENDING=1
finalize_source_swap
[ ! -e "$PERSIST_PREV_DIR" ]
echo "ok: the replaced tree is removed after acceptance"

# (4) a swap killed between the two renames is undone on the next run.
mv "$PERSIST_DIR" "$PERSIST_PREV_DIR"
[ ! -e "$PERSIST_DIR" ]
recover_interrupted_source_swap
[ -d "$PERSIST_DIR" ] && [ ! -e "$PERSIST_PREV_DIR" ]
[ "$(cat "$PERSIST_DIR/nested/file.txt")" = "new-revision" ]
echo "ok: an interrupted swap is recovered"

# (5) a stale previous tree next to a complete current one is discarded.
cp -a "$PERSIST_DIR" "$PERSIST_PREV_DIR"
printf 'stale\n' > "$PERSIST_PREV_DIR/nested/file.txt"
recover_interrupted_source_swap
[ ! -e "$PERSIST_PREV_DIR" ]
[ "$(cat "$PERSIST_DIR/nested/file.txt")" = "new-revision" ]
echo "ok: a stale previous tree is discarded, the complete current one kept"

# (6) a failure while switching the staged tree in leaves the old tree intact.
THIRD="$TMP/third-source"
cp -a "$SRC" "$THIRD"
printf 'third-revision\n' > "$THIRD/nested/file.txt"
mv() {
  if [ "${FAIL_SWITCH:-0}" = 1 ] && [ "${2:-}" = "$PERSIST_DIR" ] && [ "${1:-}" != "$PERSIST_PREV_DIR" ]; then return 1; fi
  command mv "$@"
}
FAIL_SWITCH=1
REPO_ROOT="$THIRD"
if persist_source_tree; then echo "FAIL: a failed switch must fail the install"; exit 1; fi
unset FAIL_SWITCH
unset -f mv
[ "$(cat "$PERSIST_DIR/nested/file.txt")" = "new-revision" ]
[ ! -e "$PERSIST_PREV_DIR" ]
[ -z "$(find "$(dirname "$PERSIST_DIR")" -maxdepth 1 -name '.singbox-vpn-source.*' -print -quit)" ]
echo "ok: a failed switch restores the previous tree and leaves no staging debris"

# (7) an untrusted (group-writable) existing tree is still refused, never replaced.
chmod 0775 "$PERSIST_DIR"
REPO_ROOT="$THIRD"
if persist_source_tree 2>/dev/null; then echo "FAIL: untrusted tree was accepted"; exit 1; fi
[ "$(cat "$PERSIST_DIR/nested/file.txt")" = "new-revision" ]
chmod 0755 "$PERSIST_DIR"
echo "ok: an ambiguous existing tree is refused, not replaced"
echo "persist-source-tree regression passed"
