#!/usr/bin/env bash
# firewalld rules for the compatibility stack. Public: SSH, VLESS+REALITY
# (443/tcp), Hysteria2 (443/udp), subscription HTTPS (SUBSCRIPTION_PORT,
# default 8443/tcp). Nothing
# else — internal Rust services (rendezvous, subscription's own loopback
# bind) stay off the public zone entirely (spec §33).
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

command -v firewall-cmd >/dev/null 2>&1 || { echo "firewalld not installed" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
log() { echo "[firewall] $*"; }
warn() { echo "[firewall] WARNING: $*" >&2; }
die() { echo "[firewall] ERROR: $*" >&2; exit 1; }
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

SUBSCRIPTION_PORT="${SUBSCRIPTION_PORT:-8443}"
preflight_validate_port "$SUBSCRIPTION_PORT" "SUBSCRIPTION_PORT" || die "invalid SUBSCRIPTION_PORT."

# Never assume SSH is on 22 (docs/FINAL_PRODUCTION_AUDIT.md P0-10):
# `--add-service=ssh` only covers the well-known port 22, so a custom
# sshd port must be added explicitly BEFORE this firewall goes
# effectively default-deny, or the operator's current session gets
# locked out. preflight_resolve_ssh_port() (deploy/lib/preflight.sh) is
# the single canonical implementation shared with install.sh/
# firewall-ufw.sh: it honours an explicit SSH_PORT/--ssh-port override
# and otherwise auto-detects. FAILS CLOSED (checkpoint-1 requirement):
# if detection is inconclusive and no override was supplied, this
# refuses to touch the firewall at all rather than falling back to 22.
# Resolved BEFORE firewalld is ever started below — this script is also
# callable standalone (not only via install.sh, which already activates
# firewalld SSH-safely in packages_stage), so the same ordering
# guarantee has to hold here too.
SSH_PORT="$(preflight_resolve_ssh_port)" || die "could not positively determine the real SSH port (checked sshd -T, sshd_config, and live listeners — all inconclusive), and no SSH_PORT override was supplied. Refusing to activate/reload the firewall — re-run with SSH_PORT=<port> set to sshd's real listening port (install.sh's --ssh-port does this for you)."
log "confirmed SSH port: $SSH_PORT"

FIREWALLD_WAS_INACTIVE=0
systemctl is-active --quiet firewalld || FIREWALLD_WAS_INACTIVE=1
if [ "$FIREWALLD_WAS_INACTIVE" -eq 1 ]; then
  systemctl start firewalld
fi

ZONE="$(firewall-cmd --get-default-zone)"
OWNERSHIP_STATE="/var/lib/singbox-vpn/firewall-owned.env"
owned_443_tcp=0
owned_443_udp=0
owned_subscription_tcp=0
if [ -f "$OWNERSHIP_STATE" ]; then
  # Installer-owned root-only file containing only numeric flags/validated
  # backend metadata written below.
  # shellcheck disable=SC1090
  . "$OWNERSHIP_STATE"
  if [ "${firewall_backend:-}" != "firewalld" ] || [ "${firewall_zone:-}" != "$ZONE" ]; then
    owned_443_tcp=0; owned_443_udp=0; owned_subscription_tcp=0
  fi
fi

# The very first firewall-cmd calls after a fresh activation above must
# be the SSH-allow rules — before anything else touches this zone.
firewall-cmd --zone="$ZONE" --add-service=ssh >/dev/null 2>&1 || true
if [ "$SSH_PORT" != "22" ]; then
  firewall-cmd --zone="$ZONE" --add-port="${SSH_PORT}/tcp" >/dev/null 2>&1 || true
fi
firewall-cmd --zone="$ZONE" --permanent --add-service=ssh
if [ "$SSH_PORT" != "22" ]; then
  firewall-cmd --zone="$ZONE" --permanent --add-port="${SSH_PORT}/tcp"
fi
firewall-cmd --zone="$ZONE" --permanent --query-port=443/tcp >/dev/null 2>&1 \
  || { firewall-cmd --zone="$ZONE" --permanent --add-port=443/tcp; owned_443_tcp=1; }
firewall-cmd --zone="$ZONE" --permanent --query-port=443/udp >/dev/null 2>&1 \
  || { firewall-cmd --zone="$ZONE" --permanent --add-port=443/udp; owned_443_udp=1; }
firewall-cmd --zone="$ZONE" --permanent --query-port="${SUBSCRIPTION_PORT}/tcp" >/dev/null 2>&1 \
  || { firewall-cmd --zone="$ZONE" --permanent --add-port="${SUBSCRIPTION_PORT}/tcp"; owned_subscription_tcp=1; }

install -d -m 0755 /var/lib/singbox-vpn
umask 077
cat >"$OWNERSHIP_STATE.tmp" <<EOF
firewall_backend=firewalld
firewall_zone=$ZONE
subscription_port=$SUBSCRIPTION_PORT
owned_443_tcp=$owned_443_tcp
owned_443_udp=$owned_443_udp
owned_subscription_tcp=$owned_subscription_tcp
EOF
mv -f "$OWNERSHIP_STATE.tmp" "$OWNERSHIP_STATE"

# Explicitly documented as NOT exposed publicly (spec §33): 1080 (any
# local SOCKS proxy), 9000/9100-class internal control-plane ports.
# firewalld default-deny already blocks these; nothing to add.

firewall-cmd --reload
echo "firewall rules applied on zone '$ZONE':"
firewall-cmd --zone="$ZONE" --list-all
