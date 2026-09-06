#!/usr/bin/env bash
# Regression test for activate_ufw_ssh_safe() (deploy/almalinux/install.sh,
# install_dependencies_debian's Debian/Ubuntu firewall activation).
#
# BUG this guards against: install_dependencies_debian() used to run a bare
# `systemctl enable --now ufw` right after apt-get installs packages, long
# before firewall_stage (stage 14, deploy/almalinux/firewall-ufw.sh) ever
# allows the confirmed SSH port. Starting the ufw systemd unit applies ufw's
# configured policy (default-deny incoming on a stock Ubuntu/Debian image)
# immediately, with no SSH allow rule in place yet — an operator installing
# over SSH (the normal way this installer is run on a VPS) could lose access
# between stage 2 and stage 14, and a custom SSH port was never covered by
# ufw's default rules at all. The RHEL/firewalld path already had this
# guarantee via activate_firewalld_ssh_safe(); the Debian/ufw path did not.
#
# This test extracts activate_ufw_ssh_safe()'s REAL body from install.sh and
# eval's it with `ufw` stubbed — not a reimplementation of the logic.
#
# (SSH_PORT is read by the eval'd activate_ufw_ssh_safe() body below; the
# static analyzer cannot see that dynamic use.)
# shellcheck disable=SC2034
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

ACTIVATE_UFW_SSH_SAFE_BODY="$(sed -n '/^activate_ufw_ssh_safe() {/,/^}/p' "$INSTALL_SH")"
if [ -z "$ACTIVATE_UFW_SSH_SAFE_BODY" ]; then
  fail "could not extract activate_ufw_ssh_safe() from $INSTALL_SH — has it been renamed/moved?"
  echo "$failures test(s) FAILED"
  exit 1
fi

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# Runs activate_ufw_ssh_safe() in a subshell with `ufw` stubbed to just log
# every invocation (order preserved) to a calls file, and reporting whether
# it currently considers itself "active" (so `--force enable` flips it).
run_activate_ufw_ssh_safe() {
  local ssh_port="$1" initially_active="$2" calls_file="$3" break_verification="${4:-no}"
  (
    set -Eeuo pipefail
    log() { :; }
    warn() { :; }
    die() { echo "die: $*" >&2; exit 1; }
    SSH_PORT="$ssh_port"
    UFW_ACTIVE="$initially_active"
    ALLOWED_RULES=""
    BREAK_VERIFICATION="$break_verification"
    ufw() {
      echo "ufw $*" >> "$calls_file"
      if [ "$1" = "status" ]; then
        if [ "$UFW_ACTIVE" = "yes" ]; then
          echo "Status: active"
          # Positive verification (added alongside die()) greps this
          # output for "<rule>  ALLOW", so a real stub must reflect what
          # was actually allowed rather than a fixed canned response —
          # otherwise the test would pass even if the verification step
          # were silently broken (e.g. always true, or checking the
          # wrong pattern).
          if [ "$BREAK_VERIFICATION" != "yes" ]; then
            printf '%s\n' "$ALLOWED_RULES"
          fi
        else
          echo "Status: inactive"
        fi
        return 0
      fi
      if [ "$1" = "allow" ]; then
        case "$2" in
          OpenSSH) ALLOWED_RULES="${ALLOWED_RULES}OpenSSH                    ALLOW       Anywhere
" ;;
          *) ALLOWED_RULES="${ALLOWED_RULES}${2}                    ALLOW       Anywhere
" ;;
        esac
      fi
      if [ "$1" = "--force" ] && [ "$2" = "enable" ]; then
        UFW_ACTIVE="yes"
      fi
      return 0
    }
    eval "$ACTIVATE_UFW_SSH_SAFE_BODY"
    activate_ufw_ssh_safe
  )
}

echo "--- activate_ufw_ssh_safe(): ufw initially inactive, default SSH port 22 ---"
calls="$TMPDIR_TEST/calls-inactive-22"
run_activate_ufw_ssh_safe 22 no "$calls"
if grep -qE '^ufw allow (OpenSSH|22/tcp)' "$calls" && grep -q '^ufw --force enable' "$calls"; then
  ok "SSH (port 22) is allowed before ufw is force-enabled"
else
  fail "expected an SSH allow rule before '--force enable', got:"; sed 's/^/    /' "$calls"
fi
allow_line="$(grep -n 'allow' "$calls" | head -1 | cut -d: -f1)"
enable_line="$(grep -n -- '--force enable' "$calls" | head -1 | cut -d: -f1)"
if [ -n "$allow_line" ] && [ -n "$enable_line" ] && [ "$allow_line" -lt "$enable_line" ]; then
  ok "SSH allow rule is added strictly before the firewall goes active (no lockout window)"
else
  fail "SSH allow rule was not confirmed to run before '--force enable' (allow_line=$allow_line, enable_line=$enable_line)"
fi

echo
echo "--- activate_ufw_ssh_safe(): ufw initially inactive, custom SSH port 2222 ---"
calls="$TMPDIR_TEST/calls-inactive-2222"
run_activate_ufw_ssh_safe 2222 no "$calls"
if grep -q '^ufw allow 2222/tcp' "$calls"; then
  ok "custom SSH port 2222 is explicitly allowed (not just the well-known OpenSSH/22 rule)"
else
  fail "custom SSH port 2222 was never allowed:"; sed 's/^/    /' "$calls"
fi

echo
echo "--- activate_ufw_ssh_safe(): ufw already active — must not re-enable or flush ---"
calls="$TMPDIR_TEST/calls-already-active"
run_activate_ufw_ssh_safe 22 yes "$calls"
if grep -q -- '--force enable' "$calls"; then
  fail "ufw was already active but activate_ufw_ssh_safe() re-enabled it anyway (risks disrupting existing rules/state):"; sed 's/^/    /' "$calls"
else
  ok "an already-active ufw is left completely alone (existing rules/state preserved)"
fi

echo
echo "--- activate_ufw_ssh_safe(): ufw enable succeeds but the resulting rule never shows ALLOW — must fail closed, not report success ---"
calls="$TMPDIR_TEST/calls-broken-verification"
if run_activate_ufw_ssh_safe 22 no "$calls" yes 2>"$TMPDIR_TEST/stderr-broken"; then
  fail "activate_ufw_ssh_safe() returned success even though 'ufw status' never showed the SSH rule as ALLOW — a fail-open verification bug"
else
  if grep -q 'does not show as ALLOW' "$TMPDIR_TEST/stderr-broken"; then
    ok "activate_ufw_ssh_safe() fails closed (die) when the post-enable verification can't confirm SSH is allowed"
  else
    fail "activate_ufw_ssh_safe() failed, but not with the expected verification error:"; cat "$TMPDIR_TEST/stderr-broken"
  fi
fi

echo
echo "--- static: install_dependencies_debian() calls activate_ufw_ssh_safe(), not a bare 'systemctl enable --now ufw' ---"
DEBIAN_DEPS_BODY="$(sed -n '/^install_dependencies_debian() {/,/^}/p' "$INSTALL_SH")"
if echo "$DEBIAN_DEPS_BODY" | grep -q 'activate_ufw_ssh_safe'; then
  ok "install_dependencies_debian() calls activate_ufw_ssh_safe()"
else
  fail "install_dependencies_debian() no longer calls activate_ufw_ssh_safe()"
fi
if echo "$DEBIAN_DEPS_BODY" | grep -qE 'systemctl enable --now ufw'; then
  fail "install_dependencies_debian() still activates ufw with a bare 'systemctl enable --now ufw' (no SSH-safety)"
else
  ok "install_dependencies_debian() no longer activates ufw with a bare 'systemctl enable --now ufw'"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
