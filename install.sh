#!/usr/bin/env bash
# singbox-vpn one-command bootstrap installer.
#
#   curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh | sudo bash
#
# This script does NOT assume the repository is already cloned — it is
# designed to be piped directly from `curl` into `bash` via stdin, in
# which case `$0` does not point at a real file and `${BASH_SOURCE[0]}`
# is unreliable. It never reads its own path; it downloads a fresh copy
# of the singbox-vpn source (a tagged release if one exists, otherwise the
# requested branch) into a secure temporary directory, then hands off to
# the real implementation at deploy/almalinux/install.sh (which, despite
# the directory name, supports the RHEL and Debian OS families — see
# deploy/lib/os.sh).
#
# TRUST BOUNDARY (read this before assuming more than it says): the
# initial `curl this-file | sudo bash` step itself is fetched over plain
# HTTPS from raw.githubusercontent.com with no additional signature —
# that step's trust is "HTTPS + GitHub account security", the same as
# any curl-pipe-to-shell installer, and this script does not claim
# otherwise. What IS verified: once a pinned release (VPN1_VERSION, or
# the latest tag auto-resolved in the default stable channel) is
# selected, its SOURCE ARCHIVE is downloaded from that release's GitHub
# Release assets and checksum-verified against a SHA256SUMS manifest
# published by `.github/workflows/release.yml` for that exact tag before
# it is extracted or executed — see download_verified_source_release()
# below. This is SHA-256 integrity verification, not cryptographic
# signing: it protects against a corrupted/tampered-in-transit download
# or a codeload mirror serving different bytes than the release, not
# against someone who has compromised the GitHub release itself (who
# could republish both the archive and its checksum together). No
# signing infrastructure is implemented for this checkpoint.
# VPN1_CHANNEL=dev (unpinned branch source, no VPN1_VERSION resolved)
# remains intentionally weaker: it downloads a live branch tarball via
# codeload with NO checksum verification at all, because a branch has no
# stable checksum to verify against. That path is explicitly documented
# as development/testing only, never for a production install.
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

VPN1_REPO="${VPN1_REPO:-David610/singbox-vpn}"
VPN1_VERSION="${VPN1_VERSION:-}"
VPN1_REF="${VPN1_REF:-main}"
VPN1_CHANNEL="${VPN1_CHANNEL:-stable}"

log() { echo "[bootstrap] $*" >&2; }
warn() { echo "[bootstrap] WARNING: $*" >&2; }
die() { echo "[bootstrap] ERROR: $*" >&2; exit 1; }

# Shared curl flags for every network fetch below. `--retry` alone does
# NOT protect against a connection that opens fine but then stalls
# (zero throughput) partway through — curl only retries on a completed
# failure, so a stalled-but-technically-open transfer hangs forever
# with no error and no way for `--retry` to ever kick in. Observed for
# real: this exact installer hanging indefinitely on a flaky VPS
# network mid-download, requiring a manual Ctrl+C every time.
# `--speed-limit`/`--speed-time` makes curl itself detect and abort a
# stalled transfer so `--retry` actually gets a chance to run;
# `--connect-timeout`/`--max-time` bound the rest.
CURL_NET_FLAGS=(--connect-timeout 10 --max-time 300 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2)

# ---------------------------------------------------------------------
# argument parsing (curl ... | sudo bash -s -- --version v1.2.3)
#
# Bootstrap-specific flags are consumed here. Everything else (e.g.
# --domain, --reality-handshake-server, --subscription-port,
# --non-interactive) is passed through UNCHANGED to the real installer
# at deploy/almalinux/install.sh, which is the one that understands
# them — kept in exactly one place rather than duplicated/drifting here.
# ---------------------------------------------------------------------
PASSTHROUGH_ARGS=()
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
singbox-vpn one-command bootstrap installer.

  curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh | sudo bash

Bootstrap options:
  --version, -s -- --version v1.2.3   pin to a specific tagged release
  --repo owner/repo                   install from a fork
  --ref branch-name                   source branch when no tag/version is given (default: main)

Installer options (passed through to deploy/almalinux/install.sh):
  --domain HOST                    use your own domain (accepts Unicode/IDN,
                                    e.g. чёрт.com — converted to punycode
                                    automatically)
  --reality-handshake-server HOST  REALITY decoy TLS 1.3 hostname; required
                                    for a non-interactive install, no unsafe
                                    default is ever chosen automatically
  --subscription-port PORT         public HTTPS port for the subscription
                                    endpoint (default 8443)
  --non-interactive                never prompt; fail fast instead

By default this installer resolves the latest STABLE tagged release and
installs source + binaries from that exact tag together (never a mix of
main-branch source with a different release's binaries); the source
archive is downloaded from that release and checksum-verified against
its published SHA256SUMS before extraction (see the trust-boundary note
at the top of this file — this is SHA-256 integrity verification, not
cryptographic signing). If no tagged release exists yet, the default
channel REFUSES to fall back to branch source — pin one with --version
once available. Set VPN1_CHANNEL=dev to explicitly track '--ref'
(default: main) instead — intended for development/testing only, not
production VPS installs; this downloads UNVERIFIED branch source with no
checksum check — this is the ONLY way to install unpinned branch source.

Environment overrides: VPN1_VERSION, VPN1_REPO, VPN1_REF, VPN1_CHANNEL, PUBLIC_HOST, SUBSCRIPTION_HOST, REALITY_HANDSHAKE_SERVER
USAGE
      exit 0 ;;
    *)
      PASSTHROUGH_ARGS+=("$1"); shift ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "must run as root — try: curl -fsSL https://raw.githubusercontent.com/$VPN1_REPO/main/install.sh | sudo bash"

command -v curl >/dev/null 2>&1 || die "curl is required but not found. Install curl and re-run."
command -v tar >/dev/null 2>&1 || die "tar is required but not found. Install tar and re-run."

[ -f /etc/os-release ] || die "cannot detect OS (/etc/os-release missing) — singbox-vpn requires a modern systemd Linux distribution."

log "singbox-vpn bootstrap installer starting (repo=$VPN1_REPO)"

# ---------------------------------------------------------------------
# secure temp workspace, always cleaned up
# ---------------------------------------------------------------------
TMPDIR="$(mktemp -d /tmp/singbox-vpn-install.XXXXXXXX)" || die "mktemp failed"
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
# Download and SHA-256-verify the source archive published for a pinned
# release tag by .github/workflows/release.yml (asset "vpn1-src.tar.gz"
# + a "SHA256SUMS" manifest covering it, alongside the prebuilt-binary
# archives). Fails closed (die) on: missing asset, missing/unreadable
# SHA256SUMS, a SHA256SUMS with no well-formed entry for this asset, or
# a checksum mismatch — never falls through to extracting/executing an
# unverified download. A version mismatch (wrong tag entirely) cannot
# silently occur because the URL itself is scoped to $version: GitHub
# 404s if that tag has no matching release/asset, which curl -f turns
# into a hard failure below.
# ---------------------------------------------------------------------
download_verified_source_release() {
  local version="$1" tarball="$2"
  local base_url="https://github.com/$VPN1_REPO/releases/download/$version"
  local sums="$TMPDIR/SHA256SUMS"
  log "downloading singbox-vpn $version release source archive + checksum manifest..."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tarball" "$base_url/vpn1-src.tar.gz" \
    || die "could not download release source archive 'vpn1-src.tar.gz' for $version from $VPN1_REPO. Check that this release exists and was published by .github/workflows/release.yml (which includes this asset)."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$sums" "$base_url/SHA256SUMS" \
    || die "release $version was found but its SHA256SUMS checksum manifest could not be downloaded — refusing to install an unverified source archive."
  grep -qE '^[0-9a-f]{64}  vpn1-src\.tar\.gz$' "$sums" \
    || die "SHA256SUMS for $version has no well-formed entry for vpn1-src.tar.gz (malformed or unexpected checksum manifest) — refusing to install an unverified source archive."
  # Keep sha256sum's success line off stdout. download_source used to be
  # called inside command substitution, so `vpn1-src.tar.gz: OK` was silently
  # prepended to the returned directory path and every stable install failed
  # the handoff check even though the verified archive was correct.
  ( cd "$TMPDIR" && grep -E '  vpn1-src\.tar\.gz$' SHA256SUMS | sha256sum -c - ) >&2 \
    || die "checksum verification failed for vpn1-src.tar.gz against $version's published SHA256SUMS — refusing to extract/execute an unverified source archive."
  log "source archive checksum verified against release SHA256SUMS."
}

# ---------------------------------------------------------------------
# resolve what to download: a checksum-verified release source archive
# if VPN1_VERSION is pinned (see download_verified_source_release()
# above), otherwise an UNVERIFIED branch source tarball (VPN1_CHANNEL=dev
# only — see the top-of-file trust-boundary note). GitHub's codeload
# tarballs work for any public branch without needing git installed.
# ---------------------------------------------------------------------
DOWNLOADED_SOURCE_DIR=""
download_source() {
  local ref="$VPN1_REF" url tarball="$TMPDIR/vpn1-src.tar.gz"
  if [ -n "$VPN1_VERSION" ]; then
    download_verified_source_release "$VPN1_VERSION" "$tarball"
  else
    url="https://codeload.github.com/$VPN1_REPO/tar.gz/refs/heads/$ref"
    log "downloading UNVERIFIED singbox-vpn branch source (ref=$ref, VPN1_CHANNEL=dev — no checksum verification, development/testing only)..."
    curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tarball" "$url" \
      || die "could not download source from $url. Check network connectivity and that $VPN1_REPO/$ref exists."
  fi
  log "extracting..."
  tar -xzf "$tarball" -C "$TMPDIR" || die "failed to extract downloaded source archive."
  # Both archive shapes (codeload's "<repo>-<ref>", and release.yml's
  # "vpn1-src/") extract to a single top-level directory — find it
  # rather than hardcoding either name.
  local extracted
  extracted="$(find "$TMPDIR" -mindepth 1 -maxdepth 1 -type d | head -n1)"
  [ -n "$extracted" ] || die "extracted archive did not contain the expected repository directory."
  # Return through a shell variable, not stdout. This deliberately makes the
  # handoff immune to informational output from curl/tar/checksum tools.
  DOWNLOADED_SOURCE_DIR="$extracted"
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
  latest_tag="$(curl -fsSL "${CURL_NET_FLAGS[@]}" \
      "https://api.github.com/repos/$VPN1_REPO/releases/latest" 2>/dev/null \
      | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name" *: *"([^"]*)".*/\1/')" || true
  if [ -n "$latest_tag" ]; then
    VPN1_VERSION="$latest_tag"
    log "resolved stable release: $VPN1_VERSION — source and binaries will both come from this exact tag."
    return
  fi
  # No tagged release exists yet: the default (stable) channel must NOT
  # silently install unpinned, mutable branch source in production — that
  # would make "curl | sudo bash" non-reproducible with no indication to
  # the operator. Refuse and require an explicit choice instead.
  die "no tagged release found for $VPN1_REPO — refusing to install unpinned '$VPN1_REF' branch source in the default (stable) channel. Either pin an exact release once one exists (--version vX.Y.Z), or explicitly opt into development/test mode with VPN1_CHANNEL=dev (tracks '$VPN1_REF' directly — NOT a reproducible/immutable install, do not use this for a real deployment)."
}

resolve_version
download_source
SRC_DIR="$DOWNLOADED_SOURCE_DIR"
[ -x "$SRC_DIR/deploy/almalinux/install.sh" ] || die "downloaded source is missing deploy/almalinux/install.sh — cannot continue."

log "handing off to deploy/almalinux/install.sh (repo checked out at $SRC_DIR)"
echo

# Propagate the caller's environment (PUBLIC_HOST, VPN1_VERSION, etc.)
# and the exact exit code of the real installer — never mask a failure.
set +e
VPN1_RELEASE_REPO="$VPN1_REPO" VPN1_VERSION="$VPN1_VERSION" \
  bash "$SRC_DIR/deploy/almalinux/install.sh" "${PASSTHROUGH_ARGS[@]}"
rc=$?
set -e
exit "$rc"
