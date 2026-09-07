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
: "${SINGBOX_VPN_CERTBOT_RENEWAL_LOCK:=/run/lock/singbox-vpn-certbot-renewal.lock}"

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

# Same short local critical section (and same lock file) the pre-hook
# guards its nginx marker write with — see that file's concurrency note.
# Runs regardless of whether certbot's renewal attempt itself succeeded:
# a failed renewal must never leave nginx (and therefore the subscription
# HTTPS vhost) down.
(
  flock -w 30 200 || { log "WARNING: could not acquire $SINGBOX_VPN_CERTBOT_RENEWAL_LOCK within 30s; restoring nginx anyway (best-effort, matches the marker file regardless)."; }
  if [ -e "$SINGBOX_VPN_CERTBOT_NGINX_MARKER" ]; then
    if systemctl start nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then
      log "restarted nginx after the renewal attempt."
    else
      # This is the one failure this hook pair cannot silently absorb:
      # the subscription HTTPS vhost (and any other site sharing this
      # nginx) is now down and will STAY down until an operator notices.
      # Loud, specific, and impossible to miss in `journalctl -u
      # certbot.timer` / cron mail — never just a generic "WARNING".
      log "ERROR: nginx did NOT come back up after this renewal attempt — the subscription HTTPS vhost (and any other site on this nginx) is currently DOWN. Run 'systemctl status nginx' and 'systemctl start nginx' by hand immediately."
    fi
    rm -f "$SINGBOX_VPN_CERTBOT_NGINX_MARKER"
  fi
) 200>"$SINGBOX_VPN_CERTBOT_RENEWAL_LOCK"

exit 0
