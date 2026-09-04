#!/usr/bin/env bash
# certbot post-hook: undoes exactly the TCP/80 firewall rule
# certbot-firewall-pre-hook.sh added for this renewal attempt (if any),
# every time — including when the renewal attempt itself failed, so a
# failed HTTP-01 challenge never leaves TCP/80 open. Never removes a
# pre-existing rule it did not add itself: the pre-hook only writes the
# marker this script acts on when IT was the one that opened the port.
# See certbot-firewall-pre-hook.sh for the full rationale
# (docs/FINAL_PRODUCTION_AUDIT.md F-06).
set -u

: "${SINGBOX_VPN_CERTBOT_PORT80_MARKER:=/run/singbox-vpn-certbot-port80.opened}"

log() { echo "[certbot-firewall-post-hook] $*"; }

[ -e "$SINGBOX_VPN_CERTBOT_PORT80_MARKER" ] || exit 0

if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld 2>/dev/null; then
  firewall-cmd --remove-port=80/tcp >/dev/null 2>&1 \
    && log "removed the temporary TCP/80 firewalld rule added for this renewal attempt." \
    || log "WARNING: could not remove the temporary TCP/80 firewalld rule; remove it by hand with 'firewall-cmd --remove-port=80/tcp'."
elif command -v ufw >/dev/null 2>&1; then
  ufw delete allow 80/tcp >/dev/null 2>&1 \
    && log "removed the temporary TCP/80 ufw rule added for this renewal attempt." \
    || log "WARNING: could not remove the temporary TCP/80 ufw rule; remove it by hand with 'ufw delete allow 80/tcp'."
fi

rm -f "$SINGBOX_VPN_CERTBOT_PORT80_MARKER"
exit 0
