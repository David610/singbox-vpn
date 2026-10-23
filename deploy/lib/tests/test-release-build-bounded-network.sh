#!/usr/bin/env bash
# Regression coverage for the release-container-smoke hang (PR #192, CI run 39998210724, 2026-10-23):
# deploy/lib/build-release-x86_64.sh --environment-check normally takes ~50 s but one run produced NO output for
# 9m57s after the image pull and was cancelled at the job's 10-minute timeout. The silent step was
# `docker start -a <container>` running `dnf install ... >/dev/null`: no timeout, no retry, output discarded.
# Contract enforced here: every network-dependent stage of the release build is wall-clock bounded, retried a
# bounded number of times, and its failure output is visible; the worst-case total stays below the CI job timeout.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/deploy/lib/build-release-x86_64.sh"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# shellcheck disable=SC1090
. "$SCRIPT"

echo "--- behavior: run_bounded_retry() ---"
if ! declare -F run_bounded_retry >/dev/null; then
  fail "run_bounded_retry() is not defined: network stages are unbounded"
else
  start=$SECONDS
  rc=0
  out="$(run_bounded_retry 2 2 -- sleep 30 2>&1)" || rc=$?
  elapsed=$((SECONDS - start))
  if [ "$rc" -ne 0 ] && [ "$elapsed" -le 12 ] && grep -q 'timed out' <<<"$out"; then
    ok "a hanging command is killed per attempt, retried, and the stage fails within ${elapsed}s (not minutes)"
  else
    fail "hanging command: rc=$rc elapsed=${elapsed}s out=[$out]"
  fi

  marker="$(mktemp)"; rm -f "$marker"
  flaky() { if [ -e "$marker" ]; then return 0; fi; : >"$marker"; return 1; }
  rc=0
  out="$(run_bounded_retry 3 5 -- bash -c "$(declare -f flaky); marker='$marker'; flaky" 2>&1)" || rc=$?
  if [ "$rc" -eq 0 ] && grep -q 'attempt 1/3' <<<"$out"; then
    ok "a transient failure is retried and then succeeds"
  else
    fail "transient failure: rc=$rc out=[$out]"
  fi
  rm -f "$marker"

  rc=0
  out="$(run_bounded_retry 3 5 -- true 2>&1)" || rc=$?
  [ "$rc" -eq 0 ] && ok "a healthy command passes first time" || fail "healthy command rc=$rc"
fi

echo "--- contract: the build script bounds its network stages ---"
grep -q 'run_bounded_retry .*docker pull' "$SCRIPT" && ok "image pull is bounded" || fail "docker pull is not run via run_bounded_retry"
grep -q 'run_bounded_retry .*docker start -a' "$SCRIPT" && ok "dependency-install container start is bounded" || fail "docker start -a (dnf install) is not run via run_bounded_retry"
grep -q -- '--setopt=timeout=' "$SCRIPT" && ok "dnf has an explicit per-request timeout" || fail "dnf install has no --setopt=timeout"
if grep -nE 'dnf install[^|]*>/dev/null' "$SCRIPT" >/dev/null; then
  fail "dnf output is discarded to /dev/null: a stall leaves no evidence"
else
  ok "dnf output is not silently discarded"
fi

echo "--- contract: worst-case stage budget is below the CI job timeout ---"
job_to="$(awk '/^  release-container-smoke:/{f=1} f&&/timeout-minutes:/{print $2; exit}' "$CI_YML")"
budget_all="$(sed -nE 's/^RELEASE_BUILD_NETWORK_BUDGET_SECONDS=([0-9]+).*/\1/p' "$SCRIPT")"
budget="${budget_all%%$'\n'*}"
if [ -n "$job_to" ] && [ -n "$budget" ] && [ "$budget" -lt $((job_to * 60 - 60)) ]; then
  ok "declared worst-case network budget ${budget}s < job timeout ${job_to} min"
else
  fail "no declared RELEASE_BUILD_NETWORK_BUDGET_SECONDS below the ${job_to:-?}-minute job timeout (budget=${budget:-unset})"
fi

[ "$failures" -eq 0 ] && { echo "all checks passed"; exit 0; } || { echo "$failures check(s) FAILED"; exit 1; }
