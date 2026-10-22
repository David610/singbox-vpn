#!/usr/bin/env bash
# I-49: local-tree installs record the git commit (and whether the tree is
# dirty); release installs keep the commit they were given; a different tree
# never inherits an old commit. Hermetic: temp repos only, no host state.
set -Eeuo pipefail

ACTUAL="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
command -v git >/dev/null 2>&1 || { echo "local-tree provenance: SKIP (needs git)"; exit 0; }
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# shellcheck disable=SC2034  # consumed by the sourced installer
REPO_ROOT="$ACTUAL"
# shellcheck source=/dev/null
. "$ACTUAL/deploy/almalinux/install.sh"
unset SINGBOX_VPN_SOURCE_COMMIT

SRC="$TMP/tree"
mkdir -p "$SRC"
printf 'one\n' >"$SRC/file"
git -C "$SRC" init -q
git -C "$SRC" -c user.email=t@example.test -c user.name=t add -A
git -C "$SRC" -c user.email=t@example.test -c user.name=t commit -q -m fixture
HEAD_SHA="$(git -C "$SRC" rev-parse HEAD)"

capture_local_tree_provenance "$SRC"
[ "$LOCAL_TREE_GIT_COMMIT" = "$HEAD_SHA" ] && [ "$LOCAL_TREE_GIT_DIRTY" = "false" ] \
  && ok "clean git tree records its HEAD and dirty=false" \
  || fail "clean tree: commit=$LOCAL_TREE_GIT_COMMIT dirty=$LOCAL_TREE_GIT_DIRTY"

printf 'two\n' >"$SRC/file"
capture_local_tree_provenance "$SRC"
[ "$LOCAL_TREE_GIT_COMMIT" = "$HEAD_SHA" ] && [ "$LOCAL_TREE_GIT_DIRTY" = "true" ] \
  && ok "modified tree is flagged dirty (HEAD is not what is installed)" \
  || fail "dirty tree: dirty=$LOCAL_TREE_GIT_DIRTY"
git -C "$SRC" checkout -q -- file
printf 'x\n' >"$SRC/untracked"
capture_local_tree_provenance "$SRC"
[ "$LOCAL_TREE_GIT_DIRTY" = "true" ] && ok "untracked file makes the tree dirty" || fail "untracked file not flagged"
rm -f "$SRC/untracked"

NOGIT="$TMP/nogit"
mkdir -p "$NOGIT"
capture_local_tree_provenance "$NOGIT"
[ -z "$LOCAL_TREE_GIT_COMMIT" ] && [ -z "$LOCAL_TREE_GIT_DIRTY" ] \
  && ok "a tree without .git records nothing" || fail "non-git tree recorded a commit"

SINGBOX_VPN_SOURCE_COMMIT="$HEAD_SHA" capture_local_tree_provenance "$SRC"
[ -z "$LOCAL_TREE_GIT_COMMIT" ] && ok "a provided SINGBOX_VPN_SOURCE_COMMIT is never overridden" \
  || fail "provided commit was overridden"

# Repair from the persisted copy (no .git): same content keeps the commit,
# different content does not.
DIGEST="$(printf 'a%.0s' $(seq 1 64))"
OTHER="$(printf 'b%.0s' $(seq 1 64))"
MANIFEST="$TMP/install-state.json"
cat >"$MANIFEST" <<JSON
{
  "source_commit": "$HEAD_SHA",
  "source_tree_sha256": "$DIGEST",
  "source_tree_dirty": "false",
  "acceptance": "accepted"
}
JSON
LOCAL_TREE_GIT_COMMIT=""; LOCAL_TREE_GIT_DIRTY=""
resolve_local_source_provenance "$MANIFEST" "$DIGEST"
[ "$RESOLVED_SOURCE_COMMIT" = "$HEAD_SHA" ] && [ "$RESOLVED_SOURCE_DIRTY" = "false" ] \
  && ok "repair of identical content keeps the recorded commit" || fail "same-content repair lost the commit"
resolve_local_source_provenance "$MANIFEST" "$OTHER"
[ -z "$RESOLVED_SOURCE_COMMIT" ] && ok "different content never inherits the old commit" \
  || fail "different tree inherited commit $RESOLVED_SOURCE_COMMIT"
resolve_local_source_provenance "$TMP/missing.json" "$DIGEST"
[ -z "$RESOLVED_SOURCE_COMMIT" ] && ok "no manifest -> nothing recorded" || fail "invented a commit"

# The writer must use the resolver (guards against the call being dropped).
body="$(sed -n '/^write_install_state_manifest() {/,/^}/p' "$ACTUAL/deploy/almalinux/install.sh")"
grep -q 'resolve_local_source_provenance' <<<"$body" && grep -q '"source_tree_dirty"' <<<"$body" \
  && ok "install-state writer records resolved provenance" || fail "writer does not record provenance"
grep -q 'capture_local_tree_provenance "\$REPO_ROOT"' "$ACTUAL/deploy/almalinux/install.sh" \
  && ok "provenance is captured before persist_source_tree" || fail "capture call missing"

[ "$failures" -eq 0 ] || { echo "local-tree provenance: $failures failure(s)"; exit 1; }
echo "local-tree provenance: PASS"
