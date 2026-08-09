#!/usr/bin/env bash
# firewalld rules for the compatibility stack. Public: SSH, VLESS+REALITY
# (443/tcp), Hysteria2 (443/udp), subscription HTTPS (8443/tcp). Nothing
# else — internal Rust services (rendezvous, subscription's own loopback
# bind) stay off the public zone entirely (spec §33).
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

command -v firewall-cmd >/dev/null 2>&1 || { echo "firewalld not installed" >&2; exit 1; }
systemctl is-active --quiet firewalld || systemctl start firewalld

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
log() { echo "[firewall] $*"; }
warn() { echo "[firewall] WARNING: $*" >&2; }
die() { echo "[firewall] ERROR: $*" >&2; exit 1; }
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

ZONE="$(firewall-cmd --get-default-zone)"

# Never assume SSH is on 22 (docs/FINAL_PRODUCTION_AUDIT.md P0-10):
# `--add-service=ssh` only covers the well-known port 22, so a custom
# sshd port must be added explicitly BEFORE this firewall goes
# effectively default-deny, or the operator's current session gets
# locked out.
SSH_PORT="$(preflight_detect_ssh_port)" || warn "could not positively detect the real SSH port; falling back to 22. If sshd listens on a different port, this firewall change may lock you out — verify before disconnecting."
log "detected SSH port: $SSH_PORT"

firewall-cmd --zone="$ZONE" --permanent --add-service=ssh
if [ "$SSH_PORT" != "22" ]; then
  firewall-cmd --zone="$ZONE" --permanent --add-port="${SSH_PORT}/tcp"
fi
firewall-cmd --zone="$ZONE" --permanent --add-port=443/tcp
firewall-cmd --zone="$ZONE" --permanent --add-port=443/udp
firewall-cmd --zone="$ZONE" --permanent --add-port=8443/tcp

# Explicitly documented as NOT exposed publicly (spec §33): 1080 (any
# local SOCKS proxy), 9000/9100-class internal control-plane ports.
# firewalld default-deny already blocks these; nothing to add.

firewall-cmd --reload
echo "firewall rules applied on zone '$ZONE':"
firewall-cmd --zone="$ZONE" --list-all
