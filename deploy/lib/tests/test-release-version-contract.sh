#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CHECK="$REPO_ROOT/deploy/lib/check-release-version.sh"

bash "$CHECK" v0.1.2
bash "$CHECK" v0.1.2-rc.1

for invalid in v1.0.0 v0.1.1 0.1.2 v0.1; do
  if bash "$CHECK" "$invalid" >/dev/null 2>&1; then
    echo "FAIL: invalid or mismatched release tag was accepted: $invalid" >&2
    exit 1
  fi
done

echo "release version regression checks: PASS"
