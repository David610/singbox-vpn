#!/usr/bin/env bash
# Regression test for check-release-version.sh. Deliberately reads the
# CURRENT [workspace.package].version from Cargo.toml rather than
# hardcoding a version literal here -- a hardcoded expected version is
# itself exactly the kind of duplicated-constant drift hazard this
# repository's completion program set out to reduce, and this test
# previously broke every time the real version was bumped (caught when
# v1.0.0 was prepared: this file still asserted v0.1.3 and additionally
# asserted v1.0.0 must be REJECTED).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CHECK="$REPO_ROOT/deploy/lib/check-release-version.sh"

current_version="$(awk '
  /^\[workspace\.package\][[:space:]]*$/ { in_ws=1; next }
  /^\[/ { in_ws=0 }
  in_ws && /^version[[:space:]]*=/ {
    value=$0
    sub(/^[^=]*=[[:space:]]*"/, "", value)
    sub(/".*/, "", value)
    print value
    exit
  }
' "$REPO_ROOT/Cargo.toml")"
[ -n "$current_version" ] || { echo "FAIL: could not read [workspace.package].version from Cargo.toml" >&2; exit 1; }

bash "$CHECK" "v${current_version}"
bash "$CHECK" "v${current_version}-rc.1"

# An invalid/mismatched tag must always be rejected, whatever the
# current version happens to be. `wrong_patch` bumps the last numeric
# component so it is guaranteed different from current_version without
# assuming anything about what that version's digits are.
IFS='.' read -r major minor patch _ <<<"${current_version//-*/}"
wrong_patch="${major}.${minor}.$((patch + 1))"

for invalid in "v${wrong_patch}" "${current_version}" "v${major}.${minor}"; do
  if bash "$CHECK" "$invalid" >/dev/null 2>&1; then
    echo "FAIL: invalid or mismatched release tag was accepted: $invalid" >&2
    exit 1
  fi
done

echo "release version regression checks: PASS (current version: $current_version)"
