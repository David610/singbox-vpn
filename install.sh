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
#   SUBSCRIPTION_PORT=8443        public port for the subscription HTTPS
#                                 endpoint (default 8443); change it if
#                                 something else on the VPS already
#                                 listens on 8443
#   --version v1.2.3 / -s -- --version v1.2.3   same as VPN1_VERSION,
#                                 usable through `curl | sudo bash -s --`
set -Eeuo pipefail

VPN1_REPO="${VPN1_REPO:-David610/vpn1}"
VPN1_VERSION="${VPN1_VERSION:-}"
VPN1_REF="${VPN1_REF:-main}"
VPN1_CHANNEL="${VPN1_CHANNEL:-stable}"

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

By default this installer resolves the latest STABLE tagged release and
installs source + binaries from that exact tag together (never a mix of
main-branch source with a different release's binaries). Set
VPN1_CHANNEL=dev to explicitly track '--ref' (default: main) instead —
intended for development/testing only, not production VPS installs.

Environment overrides: VPN1_VERSION, VPN1_REPO, VPN1_REF, VPN1_CHANNEL, PUBLIC_HOST, SUBSCRIPTION_HOST
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

# ---------------------------------------------------------------------
# resolve a SINGLE version up front so source and binaries never mix
# (docs/FINAL_PRODUCTION_AUDIT.md P0-7): a normal install must correspond
# to exactly one immutable, self-consistent version — never
# main-branch source/templates paired with a different release's
# binaries. VPN1_CHANNEL=dev is the explicit, documented developer
# opt-out for tracking main directly (still self-consistent: both source
# and binaries come from main in that mode, since
# deploy/almalinux/install.sh falls back to building from source when it
# can't find a release matching VPN1_VERSION).
# ---------------------------------------------------------------------
resolve_version() {
  if [ -n "$VPN1_VERSION" ]; then
    log "using pinned version $VPN1_VERSION (explicitly requested)"
    return
  fi
  if [ "$VPN1_CHANNEL" = "dev" ]; then
    log "VPN1_CHANNEL=dev — tracking '$VPN1_REF' directly (source and binaries both built from the same ref; no release-version guarantee)."
    return
  fi
  log "resolving latest stable release tag for $VPN1_REPO..."
  local latest_tag
  # `|| true`: a 404 (no release published yet) makes `curl -f` fail,
  # which under `set -e`/pipefail would otherwise abort the whole
  # installer right here instead of reaching the intended "no release
  # found, fall back to $VPN1_REF" branch below.
  latest_tag="$(curl -fsSL --retry 3 --retry-delay 2 \
      "https://api.github.com/repos/$VPN1_REPO/releases/latest" 2>/dev/null \
      | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name" *: *"([^"]*)".*/\1/')" || true
  if [ -n "$latest_tag" ]; then
    VPN1_VERSION="$latest_tag"
    log "resolved stable release: $VPN1_VERSION — source and binaries will both come from this exact tag."
  else
    warn "no tagged release found for $VPN1_REPO yet — falling back to '$VPN1_REF' (dev channel behavior). Once a release is tagged, plain 'curl | sudo bash' will automatically start using it; pass VPN1_CHANNEL=dev explicitly to silence this warning and always track main."
  fi
}

resolve_version
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
