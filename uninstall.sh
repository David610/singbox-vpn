#!/usr/bin/env bash
# vpn1 one-command bootstrap uninstaller.
#
#   curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/uninstall.sh | sudo bash
#
# Mirrors install.sh: this script does NOT assume the repository is
# already cloned or that a persistent copy still exists at /opt/vpn1. It
# never requires the operator to know about (or invoke)
# deploy/almalinux/uninstall.sh directly. It downloads a fresh copy of
# the vpn1 source (the same ref/version this host was installed from,
# when that can be determined, otherwise the requested/default branch)
# into a secure temporary directory, then hands off to the real
# implementation at deploy/almalinux/uninstall.sh, which removes
# EVERYTHING vpn1 created — completely, by default, no extra flags
# needed.
#
# Supported overrides (all optional; a plain `curl | sudo bash` needs
# none of them):
#   VPN1_REPO=owner/repo   uninstall a fork's deployment
#   VPN1_REF=branch-name   source branch to fetch the uninstaller from,
#                          if the installed version can't be determined
#                          (default: main)
set -Eeuo pipefail

VPN1_REPO="${VPN1_REPO:-David610/vpn1}"
VPN1_REF="${VPN1_REF:-main}"

log() { echo "[bootstrap] $*" >&2; }
warn() { echo "[bootstrap] WARNING: $*" >&2; }
die() { echo "[bootstrap] ERROR: $*" >&2; exit 1; }

CURL_NET_FLAGS=(--connect-timeout 10 --max-time 300 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2)

PASSTHROUGH_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) VPN1_REPO="$2"; shift 2 ;;
    --repo=*) VPN1_REPO="${1#*=}"; shift ;;
    --ref) VPN1_REF="$2"; shift 2 ;;
    --ref=*) VPN1_REF="${1#*=}"; shift ;;
    -h|--help)
      cat <<'USAGE'
vpn1 one-command bootstrap uninstaller.

  curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/uninstall.sh | sudo bash

Removes EVERYTHING vpn1 created on this host, completely, by default —
no other flags are needed. Bootstrap-specific options:
  --repo owner/repo   uninstall a fork's deployment
  --ref branch-name   source branch to fetch (default: main)

Everything else (e.g. --yes) is passed through unchanged to the real
uninstaller, deploy/almalinux/uninstall.sh — see its own --help.
USAGE
      exit 0 ;;
    *) PASSTHROUGH_ARGS+=("$1"); shift ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "must run as root — try: curl -fsSL https://raw.githubusercontent.com/$VPN1_REPO/main/uninstall.sh | sudo bash"
command -v curl >/dev/null 2>&1 || die "curl is required but not found. Install curl and re-run."
command -v tar >/dev/null 2>&1 || die "tar is required but not found. Install tar and re-run."

# If a previous install left its own persistent copy at /opt/vpn1, prefer
# it — it is guaranteed self-consistent with what was actually installed
# (never a different version's uninstall logic operating on this
# install's state), and works even with no network access at all. This
# online bootstrap is only a fallback for when that local copy is
# missing — the stable, documented, offline entry point is
# `sudo /opt/vpn1/bin/vpn1-uninstall --yes` directly.
if [ -x /opt/vpn1/bin/vpn1-uninstall ]; then
  log "found the persistent vpn1 install at /opt/vpn1 — using its own uninstaller (no download needed)."
  exec /opt/vpn1/bin/vpn1-uninstall "${PASSTHROUGH_ARGS[@]}"
elif [ -x /opt/vpn1/deploy/almalinux/uninstall.sh ]; then
  log "found the persistent vpn1 install at /opt/vpn1 (older layout, no bin/vpn1-uninstall) — using its own uninstaller (no download needed)."
  exec bash /opt/vpn1/deploy/almalinux/uninstall.sh "${PASSTHROUGH_ARGS[@]}"
fi

log "no persistent install found at /opt/vpn1 — downloading vpn1 source to run its uninstaller (repo=$VPN1_REPO, ref=$VPN1_REF)..."

TMPDIR="$(mktemp -d /tmp/vpn1-uninstall.XXXXXXXX)" || die "mktemp failed"
cleanup() {
  local rc=$?
  rm -rf "$TMPDIR"
  exit "$rc"
}
trap cleanup EXIT
trap 'die "interrupted"' INT TERM

url="https://codeload.github.com/$VPN1_REPO/tar.gz/refs/heads/$VPN1_REF"
tarball="$TMPDIR/vpn1-src.tar.gz"
if ! curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tarball" "$url"; then
  die "could not download source from $url. Check network connectivity and that $VPN1_REPO/$VPN1_REF exists.
If this host has no network access, vpn1 state may still be present at /var/lib/vpn1, /etc/vpn and /opt/vpn1 — remove those directories manually along with any sing-box/vpn1 systemd units under /etc/systemd/system as a last resort."
fi
tar -xzf "$tarball" -C "$TMPDIR" || die "failed to extract downloaded source archive."
SRC_DIR="$(find "$TMPDIR" -mindepth 1 -maxdepth 1 -type d | head -n1)"
[ -n "$SRC_DIR" ] || die "extracted archive did not contain the expected repository directory."
[ -x "$SRC_DIR/deploy/almalinux/uninstall.sh" ] || die "downloaded source is missing deploy/almalinux/uninstall.sh — cannot continue."

log "running uninstaller (removes everything vpn1 created — no further steps needed)..."
echo

set +e
bash "$SRC_DIR/deploy/almalinux/uninstall.sh" "${PASSTHROUGH_ARGS[@]}"
rc=$?
set -e
exit "$rc"
