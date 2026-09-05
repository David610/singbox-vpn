#!/usr/bin/env bash
# Fail closed if any workspace member crate declares its own literal
# `version = "..."` instead of delegating to the single authoritative
# `[workspace.package].version` in the root Cargo.toml. This is what
# actually prevents version drift between crates day-to-day; the release
# workflow's check-release-version.sh only catches drift at tag time.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

failures=0
checked=0

while IFS= read -r manifest; do
  checked=$((checked + 1))
  if ! awk '
    /^\[package\][[:space:]]*$/ { in_package=1; next }
    /^\[/ { in_package=0 }
    in_package && /^version\.workspace[[:space:]]*=[[:space:]]*true/ { found=1 }
    END { exit !found }
  ' "$manifest"; then
    echo "FAIL: $manifest does not declare version.workspace = true" >&2
    failures=$((failures + 1))
  fi
done < <(find apps crates services tests -type f -name Cargo.toml | sort)

if [ "$checked" -eq 0 ]; then
  echo "no workspace member Cargo.toml files found; refusing to pass" >&2
  exit 1
fi

if [ "$failures" -eq 0 ]; then
  echo "workspace version consistency: PASS ($checked crates all use version.workspace = true)"
  exit 0
else
  echo "workspace version consistency: FAIL ($failures crate(s) with a divergent literal version)"
  exit 1
fi
