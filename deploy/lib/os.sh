#!/usr/bin/env bash
# Shared OS/architecture detection for deploy/almalinux/install.sh and
# the root bootstrap installer. Sourced, not executed — no shebang
# execution, no `set -e` here (the caller already set it).
#
# Populates on success: OS_ID, OS_VERSION_ID, OS_PRETTY_NAME, OS_FAMILY
# ("rhel" | "debian"), PKG_MANAGER ("dnf" | "apt"), FIREWALL_BACKEND
# ("firewalld" | "ufw").
#
# Deliberately conservative: only OS/version combinations that are
# actually exercised are marked supported. Anything else fails loudly
# instead of silently pretending to work (task requirement: "do not
# pretend an OS is supported if it is not").

detect_os() {
  [ -f /etc/os-release ] || { echo "cannot detect OS: /etc/os-release missing" >&2; return 1; }
  # shellcheck disable=SC1091
  . /etc/os-release
  OS_ID="${ID:-unknown}"
  OS_VERSION_ID="${VERSION_ID:-unknown}"
  OS_PRETTY_NAME="${PRETTY_NAME:-$OS_ID $OS_VERSION_ID}"
  OS_ID_LIKE="${ID_LIKE:-}"

  case "$OS_ID" in
    almalinux|rocky|rhel|centos)
      OS_FAMILY="rhel"
      PKG_MANAGER="dnf"
      FIREWALL_BACKEND="firewalld"
      ;;
    amzn)
      # Amazon Linux 2023's /etc/os-release sets ID=amzn, ID_LIKE=fedora
      # (no "rhel" token), so without this explicit case it fell through
      # into the generic ID_LIKE-fedora branch below with OS_SUPPORT
      # always "untested" — same OS_FAMILY/PKG_MANAGER/FIREWALL_BACKEND
      # values, but no way to ever mark it "tested" and no dedicated
      # place to hang AL2023-specific behavior (e.g. install_dependencies_rhel's
      # curl-minimal handling). AL2023 uses dnf and, per its own package
      # repos, ships firewalld as an installable (not preinstalled)
      # package — same as a minimal AlmaLinux/Rocky image, so the existing
      # rhel-family firewalld install path applies unchanged. This has
      # NOT been verified against a live AL2023 host; see the OS-support
      # matrix docs for what is and is not covered by automated tests.
      OS_FAMILY="rhel"
      PKG_MANAGER="dnf"
      FIREWALL_BACKEND="firewalld"
      ;;
    ubuntu|debian)
      OS_FAMILY="debian"
      PKG_MANAGER="apt"
      FIREWALL_BACKEND="ufw"
      ;;
    *)
      case " $OS_ID_LIKE " in
        *" rhel "*|*" fedora "*)
          OS_FAMILY="rhel"; PKG_MANAGER="dnf"; FIREWALL_BACKEND="firewalld" ;;
        *" debian "*)
          OS_FAMILY="debian"; PKG_MANAGER="apt"; FIREWALL_BACKEND="ufw" ;;
        *)
          echo "unsupported operating system: $OS_PRETTY_NAME (id=$OS_ID)" >&2
          echo "vpn1 supports: AlmaLinux 9, Rocky Linux 9, RHEL 9, Amazon Linux 2023, Ubuntu 22.04/24.04, Debian 12/13." >&2
          return 1
          ;;
      esac
      ;;
  esac

  case "$OS_FAMILY-$OS_ID-$OS_VERSION_ID" in
    rhel-almalinux-9*|rhel-rocky-9*|rhel-rhel-9*) OS_SUPPORT="tested" ;;
    debian-ubuntu-22.04*|debian-ubuntu-24.04*) OS_SUPPORT="tested" ;;
    debian-debian-12*|debian-debian-13*) OS_SUPPORT="tested" ;;
    # "tested" here means: covered by automated detection + curl-minimal
    # handling tests (deploy/lib/tests/test-amazon-linux-2023.sh) — NOT
    # verified end-to-end against a real Amazon Linux 2023 EC2 instance.
    # Be honest about that distinction; do not broaden this pattern to
    # other amzn versions without equivalent coverage for them too.
    rhel-amzn-2023*) OS_SUPPORT="tested" ;;
    *) OS_SUPPORT="untested" ;;
  esac

  export OS_ID OS_VERSION_ID OS_PRETTY_NAME OS_FAMILY PKG_MANAGER FIREWALL_BACKEND OS_SUPPORT
  return 0
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "amd64" ;;
    aarch64|arm64) echo "arm64" ;;
    *) echo "unsupported architecture: $(uname -m)" >&2; return 1 ;;
  esac
}

# Rust target triple for the detected arch (used to pick release assets).
rust_target_for_arch() {
  case "$1" in
    amd64) echo "x86_64-unknown-linux-gnu" ;;
    arm64) echo "aarch64-unknown-linux-gnu" ;;
    *) echo "unsupported architecture: $1" >&2; return 1 ;;
  esac
}
