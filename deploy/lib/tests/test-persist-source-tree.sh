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
printf '#!/usr/bin/env bash\nexit 0\n' > "$SRC/nested/executable.sh"
printf 'ordinary\n' > "$SRC/nested/file.txt"
chmod 0775 "$SRC/deploy" "$SRC/deploy/almalinux"
chmod 0775 "$SRC/deploy/almalinux/uninstall.sh" "$SRC/nested/executable.sh"
chmod 0664 "$SRC/nested/file.txt"

# Source production functions without executing main, then redirect the
# canonical destination to this root-owned throwaway path.
REPO_ROOT="$SRC"
# shellcheck source=/dev/null
. "$REPO_ROOT_ACTUAL/deploy/almalinux/install.sh"
REPO_ROOT="$SRC"
PERSIST_DIR="$TMP/opt/vpn1"
OPT_VPN1_PRE_EXISTED=0
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
echo "persist-source-tree regression passed"
