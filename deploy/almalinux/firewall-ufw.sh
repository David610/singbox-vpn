#!/usr/bin/env bash
# ufw rules for the compatibility stack (Debian/Ubuntu). Mirrors
# firewall.sh (firewalld/RHEL family): SSH, VLESS+REALITY (443/tcp),
# Hysteria2 (443/udp), subscription HTTPS (SUBSCRIPTION_PORT, default
# 8443/tcp). Nothing else —
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

SUBSCRIPTION_PORT="${SUBSCRIPTION_PORT:-8443}"
preflight_validate_port "$SUBSCRIPTION_PORT" "SUBSCRIPTION_PORT" || die "invalid SUBSCRIPTION_PORT."
OWNERSHIP_STATE="/var/lib/vpn1/firewall-owned.env"
owned_443_tcp=0
owned_443_udp=0
owned_subscription_tcp=0
if [ -f "$OWNERSHIP_STATE" ]; then
  # shellcheck disable=SC1090
  . "$OWNERSHIP_STATE"
  if [ "${firewall_backend:-}" != "ufw" ]; then
    owned_443_tcp=0; owned_443_udp=0; owned_subscription_tcp=0
  fi
fi

# Never let this script be the reason SSH access is lost. Don't assume
# port 22 (docs/FINAL_PRODUCTION_AUDIT.md P0-10): `ufw allow OpenSSH`
# only covers the well-known port 22.
SSH_PORT="$(preflight_detect_ssh_port)" || warn "could not positively detect the real SSH port; falling back to 22. If sshd listens on a different port, this firewall change may lock you out — verify before disconnecting."
log "detected SSH port: $SSH_PORT"
ufw allow OpenSSH >/dev/null 2>&1 || ufw allow 22/tcp >/dev/null
if [ "$SSH_PORT" != "22" ]; then
  ufw allow "${SSH_PORT}/tcp" >/dev/null
fi
ufw status 2>/dev/null | grep -Eq '^443/tcp[[:space:]]+ALLOW' \
  || { ufw allow 443/tcp >/dev/null; owned_443_tcp=1; }
ufw status 2>/dev/null | grep -Eq '^443/udp[[:space:]]+ALLOW' \
  || { ufw allow 443/udp >/dev/null; owned_443_udp=1; }
ufw status 2>/dev/null | grep -Eq "^${SUBSCRIPTION_PORT}/tcp[[:space:]]+ALLOW" \
  || { ufw allow "${SUBSCRIPTION_PORT}/tcp" >/dev/null; owned_subscription_tcp=1; }

install -d -m 0755 /var/lib/vpn1
umask 077
cat >"$OWNERSHIP_STATE.tmp" <<EOF
firewall_backend=ufw
firewall_zone=
subscription_port=$SUBSCRIPTION_PORT
owned_443_tcp=$owned_443_tcp
owned_443_udp=$owned_443_udp
owned_subscription_tcp=$owned_subscription_tcp
EOF
mv -f "$OWNERSHIP_STATE.tmp" "$OWNERSHIP_STATE"

# `ufw enable` is only safe to run non-interactively once SSH is
# explicitly allowed above; -y skips ufw's "will disrupt existing ssh
# connections" confirmation prompt.
ufw --force enable >/dev/null

echo "firewall rules applied (ufw):"
ufw status verbose
