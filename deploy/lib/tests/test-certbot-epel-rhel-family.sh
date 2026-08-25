#!/usr/bin/env bash
# Regression test for install_certbot_repo_rhel() (deploy/almalinux/install.sh).
#
# BUG this guards against: `certbot` is not shipped in BaseOS/AppStream on
# AlmaLinux/Rocky/RHEL/CentOS Stream 9 — only via EPEL. install.sh's
# install_dependencies_rhel() ran `dnf install ... certbot` with no EPEL
# enablement anywhere in the codebase, so a fresh, stock AlmaLinux/Rocky/
# RHEL 9 image (with EPEL not already enabled by some other means) failed
# outright with "No package certbot available" — for the flagship
# "supported" OS target. Amazon Linux 2023 is the opposite case: it ships
# `certbot` directly in its own repos and does not support EPEL at all, so
# enabling EPEL there would be wrong, not just unnecessary.
#
# This test extracts install_certbot_repo_rhel()'s REAL body from
# install.sh and eval's it with `dnf`/`rpm` stubbed — not a
# reimplementation of the logic.
#
# (OS_ID/OS_PRETTY_NAME below are read by the eval'd install_certbot_repo_rhel()
# body; the static analyzer cannot see that dynamic use.)
# shellcheck disable=SC2034
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

CERTBOT_REPO_BODY="$(sed -n '/^install_certbot_repo_rhel() {/,/^}/p' "$INSTALL_SH")"
if [ -z "$CERTBOT_REPO_BODY" ]; then
  fail "could not extract install_certbot_repo_rhel() from $INSTALL_SH — has it been renamed/moved?"
  echo "$failures test(s) FAILED"
  exit 1
fi
RECORD_PKG_OWNERSHIP_RHEL_BODY="$(sed -n '/^record_package_ownership_rhel() {/,/^}/p' "$INSTALL_SH")"
if [ -z "$RECORD_PKG_OWNERSHIP_RHEL_BODY" ]; then
  fail "could not extract record_package_ownership_rhel() from $INSTALL_SH — has it been renamed/moved?"
  echo "$failures test(s) FAILED"
  exit 1
fi

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# Runs install_certbot_repo_rhel() in a subshell for a given OS_ID, with
# `epel-release` reported as either already installed or missing, and `dnf`
# stubbed to just log every `install` invocation.
run_install_certbot_repo_rhel() {
  local os_id="$1" epel_present="$2" dnf_calls="$3"
  (
    set -Eeuo pipefail
    log() { :; }
    warn() { :; }
    die() { echo "DIE: $*" >&2; exit 1; }
    OS_ID="$os_id"
    OS_PRETTY_NAME="fixture-$os_id"
    OWNERSHIP_DIR="$TMPDIR_TEST/var-lib-singbox-vpn-$os_id-$epel_present-$$"
    # shellcheck disable=SC2034 # read by deploy/lib/ownership.sh on source
    OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"
    # shellcheck source=/dev/null
    . "$REPO_ROOT/deploy/lib/ownership.sh"
    rpm() {
      if [ "$1" = "-q" ] && [ "$2" = "epel-release" ]; then
        [ "$epel_present" = "yes" ] && return 0 || return 1
      fi
      return 1
    }
    dnf() {
      if [ "$1" = "install" ]; then
        echo "$*" >> "$dnf_calls"
      fi
      return 0
    }
    eval "$RECORD_PKG_OWNERSHIP_RHEL_BODY"
    eval "$CERTBOT_REPO_BODY"
    install_certbot_repo_rhel
  )
}

echo "--- install_certbot_repo_rhel(): AlmaLinux 9, EPEL not yet present ---"
calls="$TMPDIR_TEST/dnf-calls-alma-no-epel"
: > "$calls"
run_install_certbot_repo_rhel "almalinux" "no" "$calls"
if grep -qw 'epel-release' "$calls"; then
  ok "AlmaLinux 9 without EPEL: install_certbot_repo_rhel() installs epel-release before certbot is ever requested"
else
  fail "AlmaLinux 9 without EPEL: epel-release was never installed — 'dnf install certbot' would fail with 'No package certbot available'"
fi

echo
echo "--- install_certbot_repo_rhel(): Rocky Linux 9, EPEL not yet present ---"
calls="$TMPDIR_TEST/dnf-calls-rocky-no-epel"
: > "$calls"
run_install_certbot_repo_rhel "rocky" "no" "$calls"
if grep -qw 'epel-release' "$calls"; then
  ok "Rocky Linux 9 without EPEL: epel-release is installed"
else
  fail "Rocky Linux 9 without EPEL: epel-release was never installed"
fi

echo
echo "--- install_certbot_repo_rhel(): EPEL already present — must not reinstall ---"
calls="$TMPDIR_TEST/dnf-calls-alma-has-epel"
: > "$calls"
run_install_certbot_repo_rhel "almalinux" "yes" "$calls"
if [ -s "$calls" ]; then
  fail "epel-release was already present but install_certbot_repo_rhel() still ran dnf install:"; sed 's/^/    /' "$calls"
else
  ok "an already-present epel-release is left alone (no redundant dnf install call)"
fi

echo
echo "--- install_certbot_repo_rhel(): Amazon Linux 2023 — must NOT enable EPEL (unsupported there) ---"
calls="$TMPDIR_TEST/dnf-calls-amzn"
: > "$calls"
run_install_certbot_repo_rhel "amzn" "no" "$calls"
if [ -s "$calls" ]; then
  fail "install_certbot_repo_rhel() tried to install something on Amazon Linux 2023 (EPEL is unsupported there — AL2023 ships certbot directly):"; sed 's/^/    /' "$calls"
else
  ok "Amazon Linux 2023 is left alone — no EPEL enablement attempted (AL2023 ships certbot directly and does not support EPEL)"
fi

echo
echo "--- static: install_dependencies_rhel() calls install_certbot_repo_rhel() before installing the package list ---"
DEPS_BODY="$(sed -n '/^install_dependencies_rhel() {/,/^}/p' "$REPO_ROOT/deploy/almalinux/install.sh")"
if echo "$DEPS_BODY" | grep -q 'install_certbot_repo_rhel'; then
  ok "install_dependencies_rhel() calls install_certbot_repo_rhel()"
else
  fail "install_dependencies_rhel() no longer calls install_certbot_repo_rhel()"
fi
call_line="$(echo "$DEPS_BODY" | grep -n 'install_certbot_repo_rhel' | head -1 | cut -d: -f1)"
dnf_install_line="$(echo "$DEPS_BODY" | grep -n 'dnf install -y --setopt=install_weak_deps=False' | head -1 | cut -d: -f1)"
if [ -n "$call_line" ] && [ -n "$dnf_install_line" ] && [ "$call_line" -lt "$dnf_install_line" ]; then
  ok "install_certbot_repo_rhel() runs before the main package list is installed (EPEL is available in time for 'certbot')"
else
  fail "install_certbot_repo_rhel() does not clearly run before the main dnf install of the package list"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
