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
# otherwise. What IS verified: once a pinned release (SINGBOX_VPN_VERSION, or
# the latest tag auto-resolved in the default stable channel) is
# selected, its SOURCE ARCHIVE is downloaded from that release's GitHub
# Release assets, checksum-verified against SHA256SUMS, and authenticated
# with GitHub artifact provenance bound to this repository before extraction
# or execution — see download_verified_source_release() below. SHA-256 alone
# is integrity, not publisher authentication: archive and digest share one
# GitHub trust root. Provenance prevents an asset-only replacement, but not a
# compromised trusted workflow legitimately attesting malicious output. See
# docs/SUPPLY_CHAIN_SECURITY.md.
# SINGBOX_VPN_CHANNEL=dev (unpinned branch source, no SINGBOX_VPN_VERSION resolved)
# remains intentionally weaker: it downloads a live branch tarball via
# codeload with NO checksum verification at all, because a branch has no
# stable checksum to verify against. That path is explicitly documented
# as development/testing only, never for a production install.
#
# Supported overrides (all optional):
#   SINGBOX_VPN_VERSION=v1.2.3   pin to a specific tagged release/source ref
#   SINGBOX_VPN_REPO=owner/repo  install from a fork
#   PUBLIC_HOST=vpn.example.com   use your own domain instead of the
#                                 auto-detected-IP + sslip.io default
#   SUBSCRIPTION_PORT=8443        public port for the subscription HTTPS
#                                 endpoint (default 8443); change it if
#                                 something else on the VPS already
#                                 listens on 8443
#   --version v1.2.3 / -s -- --version v1.2.3   same as SINGBOX_VPN_VERSION,
#                                 usable through `curl | sudo bash -s --`
set -Eeuo pipefail

SINGBOX_VPN_REPO="${SINGBOX_VPN_REPO:-David610/singbox-vpn}"
SINGBOX_VPN_VERSION="${SINGBOX_VPN_VERSION:-}"
SINGBOX_VPN_REF="${SINGBOX_VPN_REF:-main}"
SINGBOX_VPN_CHANNEL="${SINGBOX_VPN_CHANNEL:-stable}"
SINGBOX_VPN_ALLOW_UNVERIFIED_DEV="${SINGBOX_VPN_ALLOW_UNVERIFIED_DEV:-0}"

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
# `--retry-connrefused` also treats a transient TCP connection refusal
# as retryable (the lifecycle gate observed exactly this against github.com);
# `--connect-timeout`/`--max-time` bound the rest.
CURL_NET_FLAGS=(--connect-timeout 10 --max-time 300 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2 --retry-connrefused)

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
      SINGBOX_VPN_VERSION="$2"; shift 2 ;;
    --version=*)
      SINGBOX_VPN_VERSION="${1#*=}"; shift ;;
    --repo)
      SINGBOX_VPN_REPO="$2"; shift 2 ;;
    --repo=*)
      SINGBOX_VPN_REPO="${1#*=}"; shift ;;
    --ref)
      SINGBOX_VPN_REF="$2"; shift 2 ;;
    --ref=*)
      SINGBOX_VPN_REF="${1#*=}"; shift ;;
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
once available. Unverified branch installation requires BOTH
SINGBOX_VPN_CHANNEL=dev and SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 to explicitly track '--ref'
(default: main) instead — intended for development/testing only, not
production VPS installs; this downloads UNVERIFIED branch source with no
checksum check — this is the ONLY way to install unpinned branch source.

Environment overrides: SINGBOX_VPN_VERSION, SINGBOX_VPN_REPO, SINGBOX_VPN_REF, SINGBOX_VPN_CHANNEL, SINGBOX_VPN_ALLOW_UNVERIFIED_DEV, PUBLIC_HOST, SUBSCRIPTION_HOST, REALITY_HANDSHAKE_SERVER
USAGE
      exit 0 ;;
    *)
      PASSTHROUGH_ARGS+=("$1"); shift ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "must run as root — try: curl -fsSL https://raw.githubusercontent.com/$SINGBOX_VPN_REPO/main/install.sh | sudo bash"

command -v curl >/dev/null 2>&1 || die "curl is required but not found. Install curl and re-run."
command -v tar >/dev/null 2>&1 || die "tar is required but not found. Install tar and re-run."

[ -f /etc/os-release ] || die "cannot detect OS (/etc/os-release missing) — singbox-vpn requires a modern systemd Linux distribution."

log "singbox-vpn bootstrap installer starting (repo=$SINGBOX_VPN_REPO)"
if [ "$SINGBOX_VPN_REPO" != "David610/singbox-vpn" ]; then
  # --repo/SINGBOX_VPN_REPO is intended fork support, but every later
  # "artifact attestation verified" log line is only ever an assurance
  # relative to whatever repository is named HERE — it reads as an
  # absolute assurance if this substitution isn't obvious up front. Make
  # it impossible to miss rather than blending into an ordinary log line.
  warn "installing from a NON-DEFAULT repository: $SINGBOX_VPN_REPO (the canonical repository is David610/singbox-vpn). Every subsequent checksum/attestation check in this run verifies artifacts against $SINGBOX_VPN_REPO, NOT the canonical project — that repository, not David610/singbox-vpn, is who you are trusting."
fi

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

# Pinned sigstore/cosign build used to verify GitHub artifact attestations —
# see verify_release_attestation() below for why this replaced `gh
# attestation verify`. This script runs before any repo checkout exists (it
# is fetched standalone via raw.githubusercontent.com), so it cannot source
# deploy/lib/versions.env like the other two consumers of this same pin do;
# these three values are kept in sync with that file by
# deploy/lib/tests/test-release-reproducibility.sh.
COSIGN_VERSION=3.1.3
COSIGN_SHA256_AMD64=4629c757b7618056f8ddd7e2625ae9fdd94c0372a65049520bc7d9df9efc7f71
COSIGN_SHA256_ARM64=c5d324e091826b0d7a78eb16fef316450b4eb9aaec045611c08ba06f5e73220a

# GitHub artifact attestations bind an artifact digest to this repository and
# an Actions OIDC identity. This is authentication/provenance in addition to
# SHA256SUMS integrity; it does not make a compromised release workflow safe.
#
# Verified with cosign against the public Sigstore Rekor transparency log,
# NOT `gh attestation verify`: gh refuses to run ANY command that touches
# the GitHub API without `gh auth login`/GH_TOKEN configured first, even
# read-only lookups against a public repository (reproduced directly: `gh
# repo view` on this exact public repo fails identically unauthenticated).
# A fresh VPS has no such credential, and one cannot be pre-provisioned into
# a public installer without baking in a shared secret. GitHub's own
# artifact-attestation service is itself built on Sigstore's public-good
# Fulcio/Rekor instance, so cosign can verify the SAME attestation entirely
# anonymously once it has the signed bundle bytes — which
# .github/workflows/release.yml publishes as a plain "<asset>.sigstore.json"
# release asset, downloaded here the same unauthenticated way as the
# tarball/SHA256SUMS. See docs/SUPPLY_CHAIN_SECURITY.md.
COSIGN_BIN=""
ensure_cosign() {
  [ -n "$COSIGN_BIN" ] && return 0
  if command -v cosign >/dev/null 2>&1; then
    COSIGN_BIN="$(command -v cosign)"
    return 0
  fi
  local cosign_arch expected_sha256
  case "$(uname -m)" in
    x86_64) cosign_arch=amd64; expected_sha256="$COSIGN_SHA256_AMD64" ;;
    aarch64|arm64) cosign_arch=arm64; expected_sha256="$COSIGN_SHA256_ARM64" ;;
    *) die "unsupported CPU architecture $(uname -m) — cannot verify release attestations." ;;
  esac
  local asset="cosign-linux-${cosign_arch}"
  local dest="$TMPDIR/$asset"
  log "downloading pinned cosign v${COSIGN_VERSION} (${cosign_arch}) to verify release attestations..."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$dest" "https://github.com/sigstore/cosign/releases/download/v${COSIGN_VERSION}/${asset}" \
    || die "could not download cosign — required to verify release attestations."
  local actual_sha256
  actual_sha256="$(sha256sum "$dest" | awk '{print $1}')"
  [ "$actual_sha256" = "$expected_sha256" ] \
    || die "checksum verification failed for $asset: expected $expected_sha256, got $actual_sha256 — refusing to run an unverified cosign binary."
  chmod +x "$dest"
  COSIGN_BIN="$dest"
}

# Every stable release, with no exception, must carry a verified GitHub
# artifact attestation before its source is extracted or executed as root.
# An earlier version of this function exempted any release whose *version
# string* claimed to predate v0.1.3 from attestation entirely, falling back
# to checksum-only verification. That exemption was gated purely on
# attacker-suppliable release metadata (the tag name / embedded version):
# anyone with release-publish access to the repository — precisely the
# actor artifact attestation exists to contain — could delete and
# republish an old-numbered tag (or push a non-numeric/malformed one that
# fell through the comparison) with malicious content and skip attestation
# entirely, while every other gate (SHA256SUMS, embedded-version check)
# passed because the attacker controlled both the archive and its
# checksum manifest. There is no way to keep a version-gated fallback that
# closes that hole, so there no longer is one: every version, including
# every historical pre-v0.1.3 tag, requires attestation to install through
# this script. See docs/SUPPLY_CHAIN_SECURITY.md.
verify_release_attestation() {
  local artifact="$1" version="$2" repo="$3"
  ensure_cosign
  local bundle
  bundle="$(mktemp)"
  if ! curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$bundle" "https://github.com/$repo/releases/download/$version/$(basename "$artifact").sigstore.json"; then
    rm -f "$bundle"
    die "could not download the attestation bundle for $(basename "$artifact")/$version — refusing stable installation. There is no checksum-only fallback for any release, historical or otherwise."
  fi
  if ! "$COSIGN_BIN" verify-blob-attestation \
      --bundle "$bundle" \
      --certificate-identity "https://github.com/$repo/.github/workflows/release.yml@refs/tags/$version" \
      --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
      --type "https://slsa.dev/provenance/v1" \
      "$artifact" >/dev/null; then
    rm -f "$bundle"
    die "artifact attestation verification failed or is missing for $version/$repo — refusing stable installation. There is no checksum-only fallback for any release, historical or otherwise."
  fi
  rm -f "$bundle"
  log "artifact attestation verified for repository $repo."
}

# ---------------------------------------------------------------------
# Download and SHA-256-verify the source archive published for a pinned
# release tag by .github/workflows/release.yml (asset "singbox-vpn-src.tar.gz"
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
  local base_url="https://github.com/$SINGBOX_VPN_REPO/releases/download/$version"
  local sums="$TMPDIR/SHA256SUMS"
  log "downloading singbox-vpn $version release source archive + checksum manifest..."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tarball" "$base_url/singbox-vpn-src.tar.gz" \
    || die "could not download release source archive 'singbox-vpn-src.tar.gz' for $version from $SINGBOX_VPN_REPO. Check that this release exists and was published by .github/workflows/release.yml (which includes this asset)."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$sums" "$base_url/SHA256SUMS" \
    || die "release $version was found but its SHA256SUMS checksum manifest could not be downloaded — refusing to install an unverified source archive."
  grep -qE '^[0-9a-f]{64}  singbox-vpn-src\.tar\.gz$' "$sums" \
    || die "SHA256SUMS for $version has no well-formed entry for singbox-vpn-src.tar.gz (malformed or unexpected checksum manifest) — refusing to install an unverified source archive."
  # Keep sha256sum's success line off stdout. download_source used to be
  # called inside command substitution, so `singbox-vpn-src.tar.gz: OK` was silently
  # prepended to the returned directory path and every stable install failed
  # the handoff check even though the verified archive was correct.
  ( cd "$TMPDIR" && grep -E '  singbox-vpn-src\.tar\.gz$' SHA256SUMS | sha256sum -c - ) >&2 \
    || die "checksum verification failed for singbox-vpn-src.tar.gz against $version's published SHA256SUMS — refusing to extract/execute an unverified source archive."
  log "source archive checksum verified against release SHA256SUMS."
  verify_release_attestation "$tarball" "$version" "$SINGBOX_VPN_REPO"
}

# ---------------------------------------------------------------------
# resolve what to download: a checksum-verified release source archive
# if SINGBOX_VPN_VERSION is pinned (see download_verified_source_release()
# above), otherwise an UNVERIFIED branch source tarball (SINGBOX_VPN_CHANNEL=dev
# only — see the top-of-file trust-boundary note). GitHub's codeload
# tarballs work for any public branch without needing git installed.
# ---------------------------------------------------------------------
DOWNLOADED_SOURCE_DIR=""
download_source() {
  local ref="$SINGBOX_VPN_REF" url tarball="$TMPDIR/singbox-vpn-src.tar.gz"
  if [ -n "$SINGBOX_VPN_VERSION" ]; then
    download_verified_source_release "$SINGBOX_VPN_VERSION" "$tarball"
  else
    url="https://codeload.github.com/$SINGBOX_VPN_REPO/tar.gz/refs/heads/$ref"
    log "downloading UNVERIFIED singbox-vpn branch source (ref=$ref, SINGBOX_VPN_CHANNEL=dev — no checksum verification, development/testing only)..."
    curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tarball" "$url" \
      || die "could not download source from $url. Check network connectivity and that $SINGBOX_VPN_REPO/$ref exists."
  fi
  log "extracting..."
  tar -xzf "$tarball" -C "$TMPDIR" || die "failed to extract downloaded source archive."
  # Both archive shapes (codeload's "<repo>-<ref>", and release.yml's
  # "singbox-vpn-src/") extract to a single top-level directory — find it
  # rather than hardcoding either name.
  local extracted
  extracted="$(find "$TMPDIR" -mindepth 1 -maxdepth 1 -type d | head -n1)"
  [ -n "$extracted" ] || die "extracted archive did not contain the expected repository directory."
  if [ -n "$SINGBOX_VPN_VERSION" ]; then
    local expected_package_version="${SINGBOX_VPN_VERSION#v}"
    expected_package_version="${expected_package_version%%-*}"
    # apps/admin/Cargo.toml declares version.workspace = true (no literal
    # version string of its own — see deploy/lib/check-workspace-version-
    # consistency.sh), so the authoritative version lives only in the root
    # Cargo.toml's [workspace.package] section.
    local source_package_version
    source_package_version="$(awk '
      /^\[workspace\.package\]/ { insec=1; next }
      /^\[/ { insec=0 }
      insec && /^version[[:space:]]*=/ { print; exit }
    ' "$extracted/Cargo.toml" 2>/dev/null | sed -nE 's/^version[[:space:]]*=[[:space:]]*"([0-9]+\.[0-9]+\.[0-9]+)"[[:space:]]*$/\1/p')"
    [ "$source_package_version" = "$expected_package_version" ] \
      || die "authenticated source archive version '$source_package_version' does not match requested release '$SINGBOX_VPN_VERSION' — refusing a wrong-version release asset."
  fi
  # Return through a shell variable, not stdout. This deliberately makes the
  # handoff immune to informational output from curl/tar/checksum tools.
  DOWNLOADED_SOURCE_DIR="$extracted"
}

# ---------------------------------------------------------------------
# resolve a SINGLE version up front so source and binaries never mix
# (docs/FINAL_PRODUCTION_AUDIT.md P0-7): a normal install must correspond
# to exactly one immutable, self-consistent version — never
# main-branch source/templates paired with a different release's
# binaries. SINGBOX_VPN_CHANNEL=dev is the explicit, documented developer
# opt-out for tracking main directly (still self-consistent: both source
# and binaries come from main in that mode, since
# deploy/almalinux/install.sh falls back to building from source when it
# can't find a release matching SINGBOX_VPN_VERSION).
# ---------------------------------------------------------------------
resolve_version() {
  if [ -n "$SINGBOX_VPN_VERSION" ]; then
    log "using pinned version $SINGBOX_VPN_VERSION (explicitly requested)"
    return
  fi
  if [ "$SINGBOX_VPN_CHANNEL" = "dev" ]; then
    [ "$SINGBOX_VPN_ALLOW_UNVERIFIED_DEV" = "1" ] \
      || die "SINGBOX_VPN_CHANNEL=dev downloads mutable, checksum-unverified code for root execution. Re-run with SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 as a second deliberate opt-in (development/testing only)."
    log "SINGBOX_VPN_CHANNEL=dev — tracking '$SINGBOX_VPN_REF' directly (source and binaries both built from the same ref; no release-version guarantee)."
    return
  fi
  log "resolving latest stable release tag for $SINGBOX_VPN_REPO..."
  local latest_tag
  # `|| true`: a 404 (no release published yet) makes `curl -f` fail,
  # which under `set -e`/pipefail would otherwise abort the whole
  # installer right here instead of reaching the intended "no release
  # found, fall back to $SINGBOX_VPN_REF" branch below.
  latest_tag="$(curl -fsSL "${CURL_NET_FLAGS[@]}" \
      "https://api.github.com/repos/$SINGBOX_VPN_REPO/releases/latest" 2>/dev/null \
      | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name" *: *"([^"]*)".*/\1/')" || true
  if [ -n "$latest_tag" ]; then
    SINGBOX_VPN_VERSION="$latest_tag"
    log "resolved stable release: $SINGBOX_VPN_VERSION — source and binaries will both come from this exact tag."
    return
  fi
  # No tagged release exists yet: the default (stable) channel must NOT
  # silently install unpinned, mutable branch source in production — that
  # would make "curl | sudo bash" non-reproducible with no indication to
  # the operator. Refuse and require an explicit choice instead.
  die "no tagged release found for $SINGBOX_VPN_REPO — refusing to install unpinned '$SINGBOX_VPN_REF' branch source in the default (stable) channel. Either pin an exact release once one exists (--version vX.Y.Z), or explicitly opt into development/test mode with SINGBOX_VPN_CHANNEL=dev (tracks '$SINGBOX_VPN_REF' directly — NOT a reproducible/immutable install, do not use this for a real deployment)."
}

resolve_version
download_source
SRC_DIR="$DOWNLOADED_SOURCE_DIR"
[ -x "$SRC_DIR/deploy/almalinux/install.sh" ] || die "downloaded source is missing deploy/almalinux/install.sh — cannot continue."

log "handing off to deploy/almalinux/install.sh (repo checked out at $SRC_DIR)"
echo

# Propagate the caller's environment (PUBLIC_HOST, SINGBOX_VPN_VERSION, etc.)
# and the exact exit code of the real installer — never mask a failure.
set +e
SINGBOX_VPN_RELEASE_REPO="$SINGBOX_VPN_REPO" SINGBOX_VPN_VERSION="$SINGBOX_VPN_VERSION" \
  bash "$SRC_DIR/deploy/almalinux/install.sh" "${PASSTHROUGH_ARGS[@]}"
rc=$?
set -e
exit "$rc"
