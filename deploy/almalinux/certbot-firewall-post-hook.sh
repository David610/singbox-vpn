#!/usr/bin/env bash
# certbot post-hook: undoes exactly what certbot-firewall-pre-hook.sh did
# for this renewal attempt (if anything) — the temporary TCP/80 firewall
# rule, and stopping nginx — every time, including when the renewal
# attempt itself failed, so neither is ever left in the wrong state.
# Never undoes something it did not itself do: each marker below is only
# written by the pre-hook when IT was the one that made the change. See
# certbot-firewall-pre-hook.sh for the full rationale
# (docs/FINAL_PRODUCTION_AUDIT.md F-06 for the firewall part; reproduced
# directly on a real VPS for the nginx part — see that file).
set -u

: "${SINGBOX_VPN_CERTBOT_PORT80_MARKER:=/run/singbox-vpn-certbot-port80.opened}"
: "${SINGBOX_VPN_CERTBOT_NGINX_MARKER:=/run/singbox-vpn-certbot-nginx-stopped.opened}"

log() { echo "[certbot-firewall-post-hook] $*"; }

if [ -e "$SINGBOX_VPN_CERTBOT_PORT80_MARKER" ]; then
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
fi

if [ -e "$SINGBOX_VPN_CERTBOT_NGINX_MARKER" ]; then
  systemctl start nginx >/dev/null 2>&1 \
    && log "restarted nginx after the renewal attempt." \
    || log "WARNING: could not restart nginx; run 'systemctl start nginx' by hand."
  rm -f "$SINGBOX_VPN_CERTBOT_NGINX_MARKER"
fi

exit 0
