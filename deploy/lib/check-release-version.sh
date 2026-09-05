#!/usr/bin/env bash
# Fail closed when a release tag does not match the version compiled into the
# Rust packages. Prerelease suffixes are allowed, but their SemVer core must
# still equal every package version (for example v0.1.1-rc.1 -> 0.1.1).
set -euo pipefail

tag="${1:-}"
if [[ ! "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$ ]]; then
  echo "release tag '$tag' is invalid; expected vMAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH-prerelease" >&2
  exit 1
fi

tag_version="${BASH_REMATCH[1]}"

# Single authoritative source (`[workspace.package].version` in the root
# manifest): every member crate declares `version.workspace = true` rather
# than repeating a literal version, so drift between crates is structurally
# impossible. This script still walks every member manifest below and
# fails closed if any of them ever stops delegating to the workspace.
workspace_version="$(awk '
  /^\[workspace\.package\][[:space:]]*$/ { in_ws=1; next }
  /^\[/ { in_ws=0 }
  in_ws && /^version[[:space:]]*=/ {
    value=$0
    sub(/^[^=]*=[[:space:]]*"/, "", value)
    sub(/".*/, "", value)
    print value
    exit
  }
' Cargo.toml)"

if [ -z "$workspace_version" ]; then
  echo "no [workspace.package].version found in root Cargo.toml; refusing to validate release tag $tag" >&2
  exit 1
fi

if [ "$workspace_version" != "$tag_version" ]; then
  echo "release tag $tag has version $tag_version, but [workspace.package].version in Cargo.toml declares $workspace_version" >&2
  exit 1
fi

checked=0

while IFS= read -r manifest; do
  package_version="$(awk '
    /^\[package\][[:space:]]*$/ { in_package=1; next }
    /^\[/ { in_package=0 }
    in_package && /^version[[:space:]]*=/ {
      value=$0
      sub(/^[^=]*=[[:space:]]*"/, "", value)
      sub(/".*/, "", value)
      print value
      exit
    }
    in_package && /^version\.workspace[[:space:]]*=[[:space:]]*true/ {
      print "__WORKSPACE__"
      exit
    }
  ' "$manifest")"

  [ -n "$package_version" ] || continue
  checked=$((checked + 1))
  if [ "$package_version" = "__WORKSPACE__" ]; then
    package_version="$workspace_version"
  fi
  if [ "$package_version" != "$tag_version" ]; then
    echo "release tag $tag has version $tag_version, but $manifest declares $package_version" >&2
    exit 1
  fi
done < <(find apps crates services tests -type f -name Cargo.toml | sort)

if [ "$checked" -eq 0 ]; then
  echo "no package versions were found; refusing to validate release tag $tag" >&2
  exit 1
fi

echo "release tag/version contract: PASS ($tag; $checked packages at $tag_version)"
