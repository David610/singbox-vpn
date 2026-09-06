#!/usr/bin/env bash
# Real-package-manager OS regression check for the server support matrix
# (docs/SUPPORTED_PRODUCT.md). Run inside an official container image for
# each target distribution by .github/workflows/ci.yml's `os-matrix` job.
#
# This is NOT part of the deploy/lib/tests/*.sh fixture suite (those never
# touch a real package manager, a real network, or a real distribution
# image) — it is the L1/L2 layer described in docs/SUPPORTED_PRODUCT.md /
# docs/INSTALLATION.md:
#   L1 — the real detect_os() (deploy/lib/os.sh) classifies this
#        container's real /etc/os-release into the expected
#        OS_FAMILY/PKG_MANAGER/FIREWALL_BACKEND.
#   L2 — the exact package list install_dependencies_rhel()/
#        install_dependencies_debian() (deploy/almalinux/install.sh) would
#        install in production actually resolves and installs, for real,
#        via the real dnf/apt against that distro's real live repositories
#        today. This is the check that would have caught certbot needing
#        EPEL on AlmaLinux/Rocky/RHEL 9 (not present in BaseOS/AppStream).
#
# Deliberately does NOT prove: firewalld/ufw activation, systemd unit
# behavior, SELinux enforcement, or anything else needing a real init
# system and live kernel network stack (L3/L4 — see
# deploy/almalinux/acceptance-test.sh and docs/DEVICE_ACCEPTANCE_TESTS.md
# for what an actual VPS smoke pass covers). systemctl/firewall-cmd/ufw
# are stubbed to harmless no-ops below for exactly that reason — most
# container runtimes provide neither a running init system nor real
# firewall/netfilter access. A green run here is real evidence that
# package names resolve on this distribution today; it is NOT an
# end-to-end VPS installation pass and must never be reported as one.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_SH="$REPO_ROOT/almalinux/install.sh"

echo "--- L1: detect_os() against this container's real /etc/os-release ---"
# shellcheck source=/dev/null
. "$REPO_ROOT/lib/os.sh"
detect_os || { echo "FAIL: detect_os() failed" >&2; exit 1; }
echo "OS_PRETTY_NAME=$OS_PRETTY_NAME OS_ID=$OS_ID OS_FAMILY=$OS_FAMILY PKG_MANAGER=$PKG_MANAGER FIREWALL_BACKEND=$FIREWALL_BACKEND OS_SUPPORT=$OS_SUPPORT"

case "$OS_FAMILY" in
  rhel|debian) ;;
  *) echo "FAIL: unexpected OS_FAMILY '$OS_FAMILY' for this image" >&2; exit 1 ;;
esac

if [ -n "${EXPECT_OS_ID:-}" ] && [ "$OS_ID" != "$EXPECT_OS_ID" ]; then
  echo "FAIL: expected OS_ID='$EXPECT_OS_ID', detect_os() reported '$OS_ID'" >&2
  exit 1
fi

echo
echo "--- L2: real dependency installation via install_dependencies_${OS_FAMILY}() (production function, real dnf/apt, firewall/systemd stubbed) ---"

# systemctl/firewall-cmd/firewall-offline-cmd/ufw: stubbed — no container
# runtime here has a real init system or real firewall/netfilter access,
# this is the documented L1/L2 boundary above, not an attempt to fake
# L3/L4. But activate_firewalld_ssh_safe()/activate_ufw_ssh_safe()
# (deploy/almalinux/install.sh) now positively verify their own SSH-allow
# rule before/after activation (a real P0 fix: never trust a firewall
# activation without checking it), so these stubs have to track just
# enough in-memory state for that self-verification to see a consistent
# answer — a blanket no-op response now makes install_packages() itself
# fail here, not because activation is broken, but because the stub
# can't distinguish "rule requested" from "rule not requested". Still not
# real netfilter/systemd — just enough state to round-trip the same
# add/query calls the real tools would.
_fw_zone_ssh_service=0
_fw_zone_ssh_port=0
_fw_active=0
_ufw_active=0
_ufw_rules=""
systemctl() {
  case "$1" in
    is-active) [ "$_fw_active" -eq 1 ]; return ;;
    start) [ "${2:-}" = "firewalld" ] && _fw_active=1; return 0 ;;
    *) return 0 ;;
  esac
}
firewall-cmd() {
  case "$1" in
    --get-default-zone) echo "public"; return 0 ;;
  esac
  case "$2" in
    --add-service=ssh) _fw_zone_ssh_service=1; return 0 ;;
    --add-port=*/tcp) _fw_zone_ssh_port=1; return 0 ;;
    --query-service=ssh) [ "$_fw_zone_ssh_service" -eq 1 ]; return ;;
    --query-port=*/tcp) [ "$_fw_zone_ssh_port" -eq 1 ]; return ;;
  esac
  return 0
}
firewall-offline-cmd() {
  case "$1" in
    --get-default-zone) echo "public"; return 0 ;;
  esac
  case "$2" in
    --add-service=ssh) _fw_zone_ssh_service=1; return 0 ;;
    --add-port=*/tcp) _fw_zone_ssh_port=1; return 0 ;;
    --query-service=ssh) [ "$_fw_zone_ssh_service" -eq 1 ]; return ;;
    --query-port=*/tcp) [ "$_fw_zone_ssh_port" -eq 1 ]; return ;;
  esac
  return 0
}
ufw() {
  case "$1" in
    status)
      if [ "$_ufw_active" -eq 1 ]; then
        echo "Status: active"
        [ -n "$_ufw_rules" ] && printf '%s' "$_ufw_rules"
      else
        echo "Status: inactive"
      fi
      return 0 ;;
    allow)
      _ufw_rules="${_ufw_rules}${2}                    ALLOW       Anywhere
"
      return 0 ;;
    --force)
      [ "${2:-}" = "enable" ] && _ufw_active=1
      return 0 ;;
    *) return 0 ;;
  esac
}
# shellcheck disable=SC2034 # read by activate_firewalld_ssh_safe()/activate_ufw_ssh_safe() after install.sh is sourced below
SSH_PORT=22

OWNERSHIP_DIR="$(mktemp -d)"
# shellcheck disable=SC2034 # read by deploy/lib/ownership.sh, sourced by install.sh below
OWNERSHIP_FILE="$OWNERSHIP_DIR/ownership.env"

# install.sh guards its own main() behind a `[[ "${BASH_SOURCE[0]}" ==
# "${0}" ]]` check specifically so it can be sourced like this — see the
# comment at the bottom of that file. Sourcing it defines every function
# (including install_dependencies_rhel/debian and install_packages) without
# running the production install.
# shellcheck source=/dev/null
. "$INSTALL_SH"

install_packages

echo
echo "L1+L2 passed for $OS_PRETTY_NAME (OS_ID=$OS_ID)."
