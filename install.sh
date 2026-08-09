#!/usr/bin/env bash
# vpn1 one-command bootstrap installer.
#
#   curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash
#
# This script does NOT assume the repository is already cloned — it is
# designed to be piped directly from `curl` into `bash` via stdin, in
# which case `$0` does not point at a real file and `${BASH_SOURCE[0]}`
# is unreliable. It never reads its own path; it downloads a fresh copy
# of the vpn1 source (a tagged release if one exists, otherwise the
# requested branch) into a secure temporary directory, then hands off to
# the real implementation at deploy/almalinux/install.sh (which, despite
# the directory name, supports the RHEL and Debian OS families — see
# deploy/lib/os.sh).
#
# Supported overrides (all optional):
#   VPN1_VERSION=v1.2.3   pin to a specific tagged release/source ref
#   VPN1_REPO=owner/repo  install from a fork
#   PUBLIC_HOST=vpn.example.com   use your own domain instead of the
#                                 auto-detected-IP + sslip.io default
#   --version v1.2.3 / -s -- --version v1.2.3   same as VPN1_VERSION,
#                                 usable through `curl | sudo bash -s --`
set -Eeuo pipefail

VPN1_REPO="${VPN1_REPO:-David610/vpn1}"
VPN1_VERSION="${VPN1_VERSION:-}"
VPN1_REF="${VPN1_REF:-main}"

log() { echo "[bootstrap] $*" >&2; }
warn() { echo "[bootstrap] WARNING: $*" >&2; }
die() { echo "[bootstrap] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------------
# argument parsing (curl ... | sudo bash -s -- --version v1.2.3)
# ---------------------------------------------------------------------
while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      VPN1_VERSION="$2"; shift 2 ;;
    --version=*)
      VPN1_VERSION="${1#*=}"; shift ;;
    --repo)
      VPN1_REPO="$2"; shift 2 ;;
    --repo=*)
      VPN1_REPO="${1#*=}"; shift ;;
    --ref)
      VPN1_REF="$2"; shift 2 ;;
    --ref=*)
      VPN1_REF="${1#*=}"; shift ;;
    -h|--help)
      cat <<'USAGE'
vpn1 one-command bootstrap installer.

  curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash

Options:
  --version, -s -- --version v1.2.3   pin to a specific tagged release
  --repo owner/repo                   install from a fork
  --ref branch-name                   source branch when no tag/version is given (default: main)

Environment overrides: VPN1_VERSION, VPN1_REPO, VPN1_REF, PUBLIC_HOST, SUBSCRIPTION_HOST
USAGE
      exit 0 ;;
    *)
      die "unknown argument: $1" ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "must run as root — try: curl -fsSL https://raw.githubusercontent.com/$VPN1_REPO/main/install.sh | sudo bash"

command -v curl >/dev/null 2>&1 || die "curl is required but not found. Install curl and re-run."
command -v tar >/dev/null 2>&1 || die "tar is required but not found. Install tar and re-run."

[ -f /etc/os-release ] || die "cannot detect OS (/etc/os-release missing) — vpn1 requires a modern systemd Linux distribution."

log "vpn1 bootstrap installer starting (repo=$VPN1_REPO)"

# ---------------------------------------------------------------------
# secure temp workspace, always cleaned up
# ---------------------------------------------------------------------
TMPDIR="$(mktemp -d /tmp/vpn1-install.XXXXXXXX)" || die "mktemp failed"
cleanup() {
  local rc=$?
  rm -rf "$TMPDIR"
  if [ "$rc" -ne 0 ]; then
    echo "[bootstrap] install failed (exit $rc). Temporary files cleaned up. See errors above." >&2
    echo "[bootstrap] For diagnostics after a partial install: vpn doctor  /  journalctl -u sing-box -u vpn-subscription --no-pager -n 100" >&2
  fi
  exit "$rc"
}
trap cleanup EXIT
trap 'die "interrupted"' INT TERM

# ---------------------------------------------------------------------
# resolve what to download: a tagged release archive if VPN1_VERSION is
# set (or a real release exists), otherwise the branch source archive.
# GitHub's codeload tarballs work for any public branch/tag without
# needing git installed, and without needing to clone.
# ---------------------------------------------------------------------
download_source() {
  local ref="$VPN1_REF" url tarball="$TMPDIR/vpn1-src.tar.gz"
  if [ -n "$VPN1_VERSION" ]; then
    ref="$VPN1_VERSION"
  fi
  url="https://codeload.github.com/$VPN1_REPO/tar.gz/refs/heads/$ref"
  if [ -n "$VPN1_VERSION" ]; then
    url="https://codeload.github.com/$VPN1_REPO/tar.gz/refs/tags/$VPN1_VERSION"
  fi
  log "downloading vpn1 source (ref=$ref)..."
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$tarball" "$url"; then
    if [ -n "$VPN1_VERSION" ]; then
      die "could not download source for ref '$VPN1_VERSION' from $VPN1_REPO. Check the version exists and try again."
    fi
    die "could not download source from $url. Check network connectivity and that $VPN1_REPO/$ref exists."
  fi
  log "extracting..."
  tar -xzf "$tarball" -C "$TMPDIR" || die "failed to extract downloaded source archive."
  # codeload archives extract to a single top-level dir named
  # "<repo>-<ref-without-slashes>" — find it rather than hardcoding.
  local extracted
  extracted="$(find "$TMPDIR" -mindepth 1 -maxdepth 1 -type d | head -n1)"
  [ -n "$extracted" ] || die "extracted archive did not contain the expected repository directory."
  echo "$extracted"
}

SRC_DIR="$(download_source)"
[ -x "$SRC_DIR/deploy/almalinux/install.sh" ] || die "downloaded source is missing deploy/almalinux/install.sh — cannot continue."

log "handing off to deploy/almalinux/install.sh (repo checked out at $SRC_DIR)"
echo

# Propagate the caller's environment (PUBLIC_HOST, VPN1_VERSION, etc.)
# and the exact exit code of the real installer — never mask a failure.
set +e
VPN1_RELEASE_REPO="$VPN1_REPO" VPN1_VERSION="$VPN1_VERSION" \
  bash "$SRC_DIR/deploy/almalinux/install.sh"
rc=$?
set -e
exit "$rc"
