#!/usr/bin/env bash
# ufw rules for the compatibility stack (Debian/Ubuntu). Mirrors
# firewall.sh (firewalld/RHEL family): SSH, VLESS+REALITY (443/tcp),
# Hysteria2 (443/udp), subscription HTTPS (SUBSCRIPTION_PORT, default
# 8443/tcp). Nothing else —
# internal Rust services stay off the public interface entirely
# (spec §33). Never runs `ufw --force reset` or otherwise flushes
# pre-existing rules; only adds singbox-vpn's own allow rules.
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
OWNERSHIP_STATE="/var/lib/singbox-vpn/firewall-owned.env"
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
# only covers the well-known port 22. preflight_resolve_ssh_port()
# (deploy/lib/preflight.sh) is the single canonical implementation
# shared with install.sh/firewall.sh: it honours an explicit
# SSH_PORT/--ssh-port override and otherwise auto-detects. FAILS CLOSED
# (checkpoint-1 requirement): if detection is inconclusive and no
# override was supplied, this refuses to touch the firewall at all
# rather than falling back to 22.
SSH_PORT="$(preflight_resolve_ssh_port)" || die "could not positively determine the real SSH port (checked sshd -T, sshd_config, and live listeners — all inconclusive), and no SSH_PORT override was supplied. Refusing to enable/reload the firewall — re-run with SSH_PORT=<port> set to sshd's real listening port (install.sh's --ssh-port does this for you)."
log "confirmed SSH port: $SSH_PORT"
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

install -d -m 0755 /var/lib/singbox-vpn
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

# Positively verify the resulting rule/state rather than assume the
# `ufw allow` calls above actually took effect.
ufw status 2>/dev/null | grep -Eq '^(22/tcp|OpenSSH)[[:space:]]+ALLOW' \
  || die "ufw is now active but port 22/OpenSSH does not show as ALLOW in 'ufw status' afterward. This means SSH may be blocked. ufw is running — do not disconnect this session; investigate immediately (e.g. 'ufw allow 22/tcp') before assuming safety."
if [ "$SSH_PORT" != "22" ]; then
  ufw status 2>/dev/null | grep -Eq "^${SSH_PORT}/tcp[[:space:]]+ALLOW" \
    || die "ufw is now active but ${SSH_PORT}/tcp does not show as ALLOW in 'ufw status' afterward. This means SSH on that port may be blocked. ufw is running — do not disconnect this session; investigate immediately (e.g. 'ufw allow ${SSH_PORT}/tcp') before assuming safety."
fi

echo "firewall rules applied (ufw):"
ufw status verbose
