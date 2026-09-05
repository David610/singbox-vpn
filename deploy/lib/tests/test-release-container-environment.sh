#!/usr/bin/env bash
# Regression coverage for the v1.0.0-rc.4 release-build failure: the
# AlmaLinux release container mounted CARGO_HOME read-only, so `cargo
# build` failed with "Read-only file system (os error 30)" the moment it
# needed to update the crates.io index or download a crate — after
# fmt/clippy/test/gate all passed, and only once a real tag had already
# been cut (release.yml only triggers on tags/dispatch, so PR #53's own
# CI never exercised this container invocation at all).
#
# This does NOT grep the workflow YAML for the literal string ":ro" —
# that would only prove the rc.4 mount flag's spelling is absent, not
# that CARGO_HOME is actually usable. Instead it sources
# deploy/lib/build-release-x86_64.sh for its real
# assert_cargo_home_writable() function (the exact check that runs
# inside the production build container, embedded there verbatim via
# `declare -f` — see that script's header) and calls it against a REAL
# read-only bind mount, reproducing the exact rc.4 CARGO_HOME
# configuration mechanically rather than by inspecting mount-flag text.
#
# Requires root (bind mount + remount ro) — deploy/lib/tests/*.sh are
# invoked via `sudo bash "$test_script"` in ci.yml's `shell` job, same as
# every other test in this directory that needs root for a filesystem
# fixture.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BUILD_SH="$REPO_ROOT/deploy/lib/build-release-x86_64.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# shellcheck disable=SC1090
. "$BUILD_SH"

echo "--- functional: assert_cargo_home_writable() rejects the exact rc.4 configuration (CARGO_HOME on a real read-only mount) ---"
RO_DIR="$(mktemp -d)"
cleanup_ro() { mountpoint -q "$RO_DIR" 2>/dev/null && umount "$RO_DIR"; rm -rf "$RO_DIR"; }
trap cleanup_ro EXIT

if [ "$(id -u)" -ne 0 ]; then
  echo "SKIP: this test requires root to create a real read-only bind mount (run via sudo, as ci.yml's shell job does)"
else
  mount --bind "$RO_DIR" "$RO_DIR"
  mount -o remount,bind,ro "$RO_DIR"

  if assert_cargo_home_writable "$RO_DIR" >/tmp/rc4-writable-check-fail.out 2>&1; then
    fail "assert_cargo_home_writable() incorrectly accepted a CARGO_HOME on a read-only mount — this is exactly the v1.0.0-rc.4 configuration and must be rejected"
  else
    ok "a CARGO_HOME on a real read-only mount is rejected"
    grep -q '::error::' /tmp/rc4-writable-check-fail.out && ok "failure output includes an ::error:: diagnostic line" || fail "failure output missing ::error:: line"
    grep -qi 'not writable' /tmp/rc4-writable-check-fail.out && ok "failure output explains that CARGO_HOME is not writable" || fail "failure output does not explain that CARGO_HOME is not writable"
  fi

  umount "$RO_DIR"
  trap - EXIT
  rm -rf "$RO_DIR"

  echo
  echo "--- functional: assert_cargo_home_writable() rejects a CARGO_HOME whose registry/cache subdirectory is blocked even though the top-level dir itself is writable ---"
  # A more subtle variant of the same contract: cargo needs to CREATE
  # registry/cache under CARGO_HOME, not merely have the top-level
  # directory itself be writable. A read-only mount one level down
  # reproduces that without needing a full container.
  PARTIAL_DIR="$(mktemp -d)"
  mkdir -p "$PARTIAL_DIR/registry"
  mount --bind "$PARTIAL_DIR/registry" "$PARTIAL_DIR/registry"
  mount -o remount,bind,ro "$PARTIAL_DIR/registry"
  if assert_cargo_home_writable "$PARTIAL_DIR" >/tmp/rc4-writable-check-partial.out 2>&1; then
    fail "assert_cargo_home_writable() incorrectly accepted a CARGO_HOME whose registry/cache subdirectory cannot be created"
  else
    ok "a CARGO_HOME with a read-only registry/ subdirectory is rejected"
  fi
  umount "$PARTIAL_DIR/registry"
  rm -rf "$PARTIAL_DIR"
fi

echo
echo "--- functional: assert_cargo_home_writable() accepts a genuinely writable CARGO_HOME ---"
WRITABLE_DIR="$(mktemp -d)"
if assert_cargo_home_writable "$WRITABLE_DIR" >/tmp/rc4-writable-check-pass.out 2>&1; then
  ok "a genuinely writable CARGO_HOME is accepted"
  grep -q "registry/cache" <<<"$(ls -R "$WRITABLE_DIR")" && ok "registry/cache was actually created under it (matches what cargo build requires)" || fail "registry/cache was not created"
else
  fail "a genuinely writable CARGO_HOME was incorrectly rejected: $(cat /tmp/rc4-writable-check-pass.out)"
fi
rm -rf "$WRITABLE_DIR"

echo
echo "--- static: release.yml's build job and ci.yml's pre-tag smoke job both call this single-sourced script, not a hand-copied Docker invocation ---"
RELEASE_YML="$REPO_ROOT/.github/workflows/release.yml"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
if grep -q 'bash deploy/lib/build-release-x86_64.sh$' "$RELEASE_YML"; then
  ok "release.yml's build job invokes deploy/lib/build-release-x86_64.sh"
else
  fail "release.yml no longer invokes deploy/lib/build-release-x86_64.sh for its production build — has the container invocation been re-inlined into YAML?"
fi
if grep -q 'bash deploy/lib/build-release-x86_64.sh --environment-check' "$CI_YML"; then
  ok "ci.yml runs the same script in --environment-check mode as a pre-tag smoke job"
else
  fail "ci.yml has no pre-tag --environment-check smoke job for the release build container — this is the exact process gap that let PR #53 ship the rc.4 bug undetected before a tag was cut"
fi

echo
echo "--- static: RELEASE_BUILD_IMAGE/RELEASE_GLIBC_BASELINE are single-sourced (deploy/lib/release-build.env), not duplicated between the two workflows ---"
ENV_FILE="$REPO_ROOT/deploy/lib/release-build.env"
if [ -f "$ENV_FILE" ] && grep -q '^RELEASE_BUILD_IMAGE=' "$ENV_FILE" && grep -q '^RELEASE_GLIBC_BASELINE=' "$ENV_FILE"; then
  ok "deploy/lib/release-build.env defines both pinned values"
else
  fail "deploy/lib/release-build.env is missing or does not define RELEASE_BUILD_IMAGE/RELEASE_GLIBC_BASELINE"
fi
if grep -q 'deploy/lib/release-build.env' "$RELEASE_YML" && grep -q 'deploy/lib/release-build.env' "$CI_YML"; then
  ok "both release.yml and ci.yml load deploy/lib/release-build.env"
else
  fail "release.yml and ci.yml do not both load deploy/lib/release-build.env — the pinned image/baseline could drift between the real build and the pre-tag smoke check"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all release-container-environment regression tests passed"
