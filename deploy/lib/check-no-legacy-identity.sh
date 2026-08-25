#!/usr/bin/env bash
# Regression guard for the product's clean-break rename away from its
# obsolete pre-rename internal identifier (docs/IMPLEMENTATION_STATUS.md):
# fails if that identifier appears anywhere in the current tracked tree,
# case-insensitively, in either a file's content or its path.
#
# This is a clean-break rename: no permanent alias, wrapper, or
# compatibility path may carry the old name going forward, so the
# required invariant is exactly zero — not a shrinking allowlist. Old
# git history/tags are exempt (git history is never rewritten for this);
# only the current working tree is in scope.
#
# The banned identifier is built from parts below rather than written
# out directly, so this guard script itself never contains the literal
# string it forbids everywhere else in the tree.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

LEGACY_PREFIX="vpn"
LEGACY_SUFFIX="1"
LEGACY_PATTERN="${LEGACY_PREFIX}${LEGACY_SUFFIX}"
SELF="deploy/lib/$(basename "${BASH_SOURCE[0]}")"
FAIL=0

echo "== checking tracked file contents for the obsolete identifier =="
if content_hits="$(git grep -InE -i "$LEGACY_PATTERN" -- . ":!$SELF" 2>/dev/null)"; then
  echo "$content_hits"
  FAIL=1
fi

echo "== checking tracked file paths for the obsolete identifier =="
if path_hits="$(git ls-files | grep -iE "$LEGACY_PATTERN" || true)" && [ -n "$path_hits" ]; then
  echo "$path_hits"
  FAIL=1
fi

if [ "$FAIL" -eq 0 ]; then
  echo "ok: no obsolete pre-rename identity found in tracked content or paths"
  exit 0
else
  echo "FAIL: obsolete pre-rename identity found — see matches above"
  exit 1
fi
