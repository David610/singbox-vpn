#!/usr/bin/env bash
# vpn1 one-command bootstrap uninstaller.
#
#   curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh | sudo bash
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

VPN1_REPO="${VPN1_REPO:-David610/singbox-vpn}"
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
    --ref) VPN1_REF="$2"; VPN1_REF_EXPLICIT=1; shift 2 ;;
    --ref=*) VPN1_REF="${1#*=}"; VPN1_REF_EXPLICIT=1; shift ;;
    -h|--help)
      cat <<'USAGE'
vpn1 one-command bootstrap uninstaller.

  curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh | sudo bash

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

# Runs a persistent local deploy/almalinux/uninstall.sh, translating
# modern flags to whatever interface THAT copy actually understands.
# Historical layouts that matter (docs/IMPLEMENTATION_STATUS.md has the
# exact commit range for each):
#   - current (since commit 07f8b72): understands --yes; forward as-is.
#   - pre-07f8b72 (predates bin/vpn1-uninstall entirely): only ever
#     understood --purge-state/--purge-firewall, had NO --yes flag and
#     NO interactive confirmation prompt at all (it ran immediately,
#     unconditionally) — `case "$arg" in ... *) die "unknown flag" ;;`
#     means a modern `--yes` makes it abort before removing anything, a
#     real failure mode this was filed to fix. --yes is meaningless to a
#     script that never prompts, so it is dropped; --purge-state
#     --purge-firewall are added so this same bootstrap command still
#     performs a COMPLETE removal against that historical layout too,
#     matching what the modern default promises.
# Any other/unrecognized interface: forwarded as-is (best effort) rather
# than refused outright — every actual historical layout in this
# repository's git history is one of the two above, so this path is a
# safety net rather than a targeted case; if that script would ever
# block on an interactive prompt with no /dev/tty available, this cannot
# detect that in advance, but no version in this repo's history does.
run_legacy_uninstaller() {
  local script="$1"
  shift
  if grep -q -- '--yes) ASSUME_YES=1' "$script" 2>/dev/null; then
    exec bash "$script" "$@"
  fi
  if grep -q -- '--purge-state' "$script" 2>/dev/null && ! grep -q -- '\-\-yes' "$script" 2>/dev/null; then
    log "found an older persistent install (predates the --yes/bin/vpn1-uninstall layout) — translating to its actual interface (--purge-state --purge-firewall; it never prompts, so --yes is dropped as meaningless to it) for a complete, non-interactive removal."
    local filtered=() a
    for a in "$@"; do
      case "$a" in
        --yes) ;; # this version has no such flag and never prompts; dropping it is a no-op, not a behavior change
        *) filtered+=("$a") ;;
      esac
    done
    exec bash "$script" --purge-state --purge-firewall "${filtered[@]}"
  fi
  warn "found a persistent install at /opt/vpn1 with an unrecognized uninstaller interface — forwarding arguments as-is. If it rejects them, remove state manually: /var/lib/vpn1, /etc/vpn, /opt/vpn1, and any sing-box/vpn1 systemd units under /etc/systemd/system."
  exec bash "$script" "$@"
}

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
  run_legacy_uninstaller /opt/vpn1/deploy/almalinux/uninstall.sh "${PASSTHROUGH_ARGS[@]}"
fi

# No usable local copy — this is a damaged/incomplete /opt/vpn1 (or it
# never existed). Prefer the EXACT version this host was actually
# installed with over today's mutable main branch: install-state.json
# (written by install.sh, survives independently of /opt/vpn1/bin being
# damaged as long as /var/lib/vpn1 itself is intact) records the pinned
# release tag and repo this host was installed from. Running arbitrary
# newer destructive uninstall logic against old installed state is
# exactly the kind of mismatch this checkpoint is meant to avoid — only
# fall back to the conservative $VPN1_REPO/$VPN1_REF default (main, by
# default) when no exact version can be determined at all (a dev/
# unreleased install, or install-state.json itself is missing/damaged).
INSTALL_STATE_MANIFEST="/var/lib/vpn1/install-state.json"
if [ -z "${VPN1_REF_EXPLICIT:-}" ] && [ -f "$INSTALL_STATE_MANIFEST" ]; then
  manifest_repo="$(grep -o '"vpn1_repo"[[:space:]]*:[[:space:]]*"[^"]*"' "$INSTALL_STATE_MANIFEST" 2>/dev/null | sed -E 's/.*"([^"]*)"$/\1/')"
  manifest_version="$(grep -o '"vpn1_version"[[:space:]]*:[[:space:]]*"[^"]*"' "$INSTALL_STATE_MANIFEST" 2>/dev/null | sed -E 's/.*"([^"]*)"$/\1/')"
  if [ -n "$manifest_repo" ] && [ -n "$manifest_version" ] && [ "$manifest_version" != "main" ]; then
    log "found $INSTALL_STATE_MANIFEST — this host was installed from $manifest_repo@$manifest_version; using that EXACT pinned version instead of the mutable '$VPN1_REF' default."
    VPN1_REPO="$manifest_repo"
    VPN1_REF="$manifest_version"
    VPN1_REF_IS_TAG=1
  else
    warn "found $INSTALL_STATE_MANIFEST but it does not record a pinned release version (vpn1_version='${manifest_version:-<missing>}', likely a dev/unreleased install) — falling back to the conservative default ($VPN1_REPO/$VPN1_REF). This may not exactly match what was installed; prefer restoring bin/vpn1-uninstall from a backup if precision matters here."
  fi
fi
VPN1_REF_IS_TAG="${VPN1_REF_IS_TAG:-0}"

log "no persistent install found at /opt/vpn1 — downloading vpn1 source to run its uninstaller (repo=$VPN1_REPO, ref=$VPN1_REF)..."

TMPDIR="$(mktemp -d /tmp/vpn1-uninstall.XXXXXXXX)" || die "mktemp failed"
cleanup() {
  local rc=$?
  rm -rf "$TMPDIR"
  exit "$rc"
}
trap cleanup EXIT
trap 'die "interrupted"' INT TERM

if [ "$VPN1_REF_IS_TAG" -eq 1 ]; then
  url="https://codeload.github.com/$VPN1_REPO/tar.gz/refs/tags/$VPN1_REF"
else
  url="https://codeload.github.com/$VPN1_REPO/tar.gz/refs/heads/$VPN1_REF"
fi
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
