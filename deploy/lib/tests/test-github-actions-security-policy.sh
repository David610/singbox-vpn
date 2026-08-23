#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$repo_root"

failures=0
workflow_count=0
action_count=0

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

shopt -s nullglob
workflows=(.github/workflows/*.yml .github/workflows/*.yaml)
shopt -u nullglob

if (( ${#workflows[@]} == 0 )); then
  fail "no GitHub Actions workflows found"
fi

for workflow in "${workflows[@]}"; do
  workflow_count=$((workflow_count + 1))

  # pull_request_target executes base-branch workflow code with a token that
  # may have elevated privileges. This repository has no use case that
  # justifies that trust boundary; normal pull_request is sufficient.
  if grep -Eq '^[[:space:]]*pull_request_target[[:space:]]*:' "$workflow"; then
    fail "$workflow uses pull_request_target"
  fi

  # Every workflow must set an explicit top-level permission baseline before
  # jobs:. Individual jobs may narrow or selectively add the one write scope
  # they need (for example release publishing or CodeQL SARIF upload).
  pre_jobs="$(sed -n '1,/^jobs:/p' "$workflow")"
  if ! grep -Eq '^permissions[[:space:]]*:' <<<"$pre_jobs"; then
    fail "$workflow has no explicit top-level permissions baseline"
  fi
  if grep -Eq '^permissions[[:space:]]*:[[:space:]]*write-all([[:space:]]|$)' <<<"$pre_jobs"; then
    fail "$workflow grants top-level write-all"
  fi
  if grep -Eq '^[[:space:]]{2}contents[[:space:]]*:[[:space:]]*write([[:space:]]|$)' <<<"$pre_jobs"; then
    fail "$workflow grants top-level contents: write instead of narrowing it to a job"
  fi

  # External actions are executable third-party code. A version tag such as
  # @v4 is mutable; require an immutable 40-character Git commit SHA. Local
  # reusable workflows are repository-controlled and are allowed via ./...
  while IFS= read -r ref; do
    [[ -n "$ref" ]] || continue
    action_count=$((action_count + 1))
    if [[ "$ref" == ./* ]]; then
      continue
    fi
    if [[ ! "$ref" =~ ^[^[:space:]@]+@[0-9a-fA-F]{40}$ ]]; then
      fail "$workflow contains an external action not pinned to a full commit SHA: $ref"
    fi
  done < <(
    sed -nE \
      -e 's/^[[:space:]]*-[[:space:]]*uses:[[:space:]]*([^#[:space:]]+).*/\1/p' \
      -e 's/^[[:space:]]*uses:[[:space:]]*([^#[:space:]]+).*/\1/p' \
      "$workflow"
  )
done

if (( action_count == 0 )); then
  fail "no action uses were inspected; parser likely stopped matching workflow syntax"
fi

if (( failures != 0 )); then
  echo "$failures GitHub Actions security policy violation(s) found across $workflow_count workflow(s)." >&2
  exit 1
fi

echo "OK: inspected $workflow_count workflow(s) and $action_count action/reusable-workflow reference(s); external actions are SHA-pinned, pull_request_target is absent, and top-level permissions are constrained."
