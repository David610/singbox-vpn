#!/usr/bin/env bash
# certbot pre-hook: temporarily opens TCP/80 at the HOST firewall
# (firewalld/ufw) before EVERY `certbot renew` attempt, and
# certbot-firewall-post-hook.sh closes it again afterward. Installed by
# install.sh into /etc/letsencrypt/renewal-hooks/pre/, which certbot runs
# unconditionally before any renewal attempt on the host (regardless of
# which lineage(s) are due) — normally driven by the distro's
# certbot.timer / certbot-renew.timer.
#
# Why this exists (docs/FINAL_PRODUCTION_AUDIT.md F-06): install.sh's own
# firewall_open_port_80_temp/firewall_close_port_80_temp only run once,
# during the initial install, to let the FIRST certbot certonly
# --standalone HTTP-01 challenge through. Both certificates this project
# issues use HTTP-01, and certbot's renewal timer re-runs that same
# standalone challenge roughly every 60 days for the life of the
# deployment — but nothing reopened TCP/80 at the host firewall for those
# later attempts, so renewal would silently keep failing (the challenge
# never reaches the standalone listener) until the certificate actually
# expired and Hysteria2/the subscription vhost broke. This hook pair
# closes that gap by reopening TCP/80 for the few seconds the challenge
# actually needs, on every renewal attempt, then closing it again — the
# same transient-exposure approach install.sh already uses at install
# time, rather than leaving TCP/80 open to 0.0.0.0/0 permanently.
#
# This does not (and cannot) touch a separate cloud-provider firewall
# layer (AWS security groups, GCP firewall rules, etc.) — see
# docs/ALMALINUX_DEPLOYMENT.md "Cloud provider firewalls / security
# groups" for why that layer still needs its own permanent TCP/80 allow
# rule if it exists.
#
# Deliberately never fails the renewal attempt over firewall-management
# trouble: if this host has no managed firewall backend, or the backend
# tool misbehaves, the certbot HTTP-01 challenge itself will fail with
# its own clear, actionable error — this hook does not manufacture a
# second, less informative failure on top of that.
set -u

: "${SINGBOX_VPN_CERTBOT_PORT80_MARKER:=/run/singbox-vpn-certbot-port80.opened}"

log() { echo "[certbot-firewall-pre-hook] $*"; }

firewall_backend() {
  if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld 2>/dev/null; then
    echo firewalld
  elif command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -qi '^Status: active'; then
    echo ufw
  fi
}

backend="$(firewall_backend)"
case "$backend" in
  firewalld)
    if firewall-cmd --query-port=80/tcp >/dev/null 2>&1; then
      log "TCP/80 already allowed in firewalld; nothing to do."
    else
      # Runtime-only, not --permanent: gone on the next reload/reboot
      # even if the post-hook below is somehow skipped.
      if firewall-cmd --add-port=80/tcp >/dev/null 2>&1; then
        : > "$SINGBOX_VPN_CERTBOT_PORT80_MARKER" 2>/dev/null
        log "temporarily allowed inbound TCP/80 in firewalld for the ACME HTTP-01 renewal challenge."
      else
        log "WARNING: firewall-cmd --add-port=80/tcp failed; the renewal challenge may not be reachable."
      fi
    fi
    ;;
  ufw)
    if ufw status 2>/dev/null | grep -Eq '^80/tcp[[:space:]]+ALLOW'; then
      log "TCP/80 already allowed in ufw; nothing to do."
    else
      if ufw allow 80/tcp >/dev/null 2>&1; then
        : > "$SINGBOX_VPN_CERTBOT_PORT80_MARKER" 2>/dev/null
        log "temporarily allowed inbound TCP/80 in ufw for the ACME HTTP-01 renewal challenge."
      else
        log "WARNING: ufw allow 80/tcp failed; the renewal challenge may not be reachable."
      fi
    fi
    ;;
  *)
    log "no managed firewalld/ufw backend detected active; leaving the firewall untouched."
    ;;
esac

exit 0
