#!/usr/bin/env bash
# ufw rules for the compatibility stack (Debian/Ubuntu). Mirrors
# firewall.sh (firewalld/RHEL family): SSH, VLESS+REALITY (443/tcp),
# Hysteria2 (443/udp), subscription HTTPS (8443/tcp). Nothing else —
# internal Rust services stay off the public interface entirely
# (spec §33). Never runs `ufw --force reset` or otherwise flushes
# pre-existing rules; only adds vpn1's own allow rules.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

command -v ufw >/dev/null 2>&1 || { echo "ufw not installed" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
log() { echo "[firewall] $*"; }
warn() { echo "[firewall] WARNING: $*" >&2; }
die() { echo "[firewall] ERROR: $*" >&2; exit 1; }
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

# Never let this script be the reason SSH access is lost. Don't assume
# port 22 (docs/FINAL_PRODUCTION_AUDIT.md P0-10): `ufw allow OpenSSH`
# only covers the well-known port 22.
SSH_PORT="$(preflight_detect_ssh_port)" || warn "could not positively detect the real SSH port; falling back to 22. If sshd listens on a different port, this firewall change may lock you out — verify before disconnecting."
log "detected SSH port: $SSH_PORT"
ufw allow OpenSSH >/dev/null 2>&1 || ufw allow 22/tcp >/dev/null
if [ "$SSH_PORT" != "22" ]; then
  ufw allow "${SSH_PORT}/tcp" >/dev/null
fi
ufw allow 443/tcp >/dev/null
ufw allow 443/udp >/dev/null
ufw allow 8443/tcp >/dev/null

# `ufw enable` is only safe to run non-interactively once SSH is
# explicitly allowed above; -y skips ufw's "will disrupt existing ssh
# connections" confirmation prompt.
ufw --force enable >/dev/null

echo "firewall rules applied (ufw):"
ufw status verbose
