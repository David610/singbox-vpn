#!/usr/bin/env bash
# certbot pre-hook: makes TCP/80 actually available for the ACME HTTP-01
# challenge before EVERY `certbot renew` attempt — both at the HOST
# firewall (firewalld/ufw) AND locally, by stopping nginx if it is
# running. certbot-firewall-post-hook.sh undoes both afterward. Installed
# by install.sh into /etc/letsencrypt/renewal-hooks/pre/, which certbot
# runs unconditionally before any renewal attempt on the host (regardless
# of which lineage(s) are due) — normally driven by the distro's
# certbot.timer / certbot-renew.timer.
#
# Why the firewall part exists (docs/FINAL_PRODUCTION_AUDIT.md F-06):
# install.sh's own firewall_open_port_80_temp/firewall_close_port_80_temp
# only run once, during the initial install, to let the FIRST certbot
# certonly --standalone HTTP-01 challenge through. Both certificates this
# project issues use HTTP-01, and certbot's renewal timer re-runs that
# same standalone challenge roughly every 60 days for the life of the
# deployment — but nothing reopened TCP/80 at the host firewall for those
# later attempts, so renewal would silently keep failing (the challenge
# never reaches the standalone listener) until the certificate actually
# expired and Hysteria2/the subscription vhost broke. This hook pair
# closes that gap by reopening TCP/80 for the few seconds the challenge
# actually needs, on every renewal attempt, then closing it again — the
# same transient-exposure approach install.sh already uses at install
# time, rather than leaving TCP/80 open to 0.0.0.0/0 permanently.
#
# Why the nginx part exists: reproduced directly on a real VPS —
# `certbot renew --dry-run` failed every time with "Could not bind TCP
# port 80 because it is already in use by another process" even with the
# firewall correctly opened above. nginx (installed alongside singbox-vpn
# for the subscription HTTPS vhost, which deliberately never listens on
# :80 itself — see nginx-vpn-subscription.conf.template) still binds :80
# via its OS-default vhost, and certbot's --standalone authenticator
# needs to bind that same port itself. install.sh's own
# attempt_automatic_certbot() already stops nginx for exactly this reason
# before the INITIAL issuance; this hook applies the identical fix to
# every later renewal, which was the actual gap.
#
# TRADE-OFF, chosen deliberately for the v1.0 release path (not hidden):
# nginx is stopped as a WHOLE SERVICE, not just its default :80 vhost —
# this project's own subscription vhost never listens on :80 (see
# nginx-vpn-subscription.conf.template), so the ONLY thing actually
# affected is nginx's OS-default vhost and any OTHER site an operator may
# have added to this same nginx. If this host also serves unrelated
# HTTP/HTTPS sites through this nginx, ALL of them go briefly offline for
# the few seconds a renewal takes (typically well under 10s). singbox-vpn's
# own VPN data plane (sing-box: REALITY/Hysteria2) and vpn-subscription
# backend are never touched by this — only nginx. Switching authenticators
# (webroot/nginx plugin) to avoid this was considered and deliberately
# deferred past v1.0 (see the PR that introduced this comment for the
# full option comparison) to keep the already-working --standalone
# issuance model unchanged this close to release.
#
# This does not (and cannot) touch a separate cloud-provider firewall
# layer (AWS security groups, GCP firewall rules, etc.) — see
# docs/ALMALINUX_DEPLOYMENT.md "Cloud provider firewalls / security
# groups" for why that layer still needs its own permanent TCP/80 allow
# rule if it exists.
#
# Concurrency: certbot itself locks /etc/letsencrypt for the duration of
# any certonly/renew invocation (a second concurrent `certbot renew`
# fails fast with its own "Another instance of Certbot is already
# running" error) — this is documented, long-standing certbot behavior,
# not something this project needs to re-implement. What that lock does
# NOT cover is a completely separate code path also touching nginx/TCP80
# around a certificate operation: install.sh's own
# attempt_automatic_certbot() (used for the very first issuance) stops
# nginx independently, with its own local bookkeeping, not these marker
# files. An install.sh/update.sh run happening to overlap with a renewal
# timer firing is a narrow, low-probability window, but the failure mode
# (two independent nginx stop/start sequences interleaving) is exactly
# the kind of thing that corrupts state silently — so this hook still
# guards its own marker-file critical section with a short, local flock.
# That is defense-in-depth around OUR bookkeeping, not a claim that it
# alone prevents every possible overlap with install.sh's separate path.
#
# Deliberately never fails the renewal attempt over firewall management
# trouble (missing firewalld/ufw, or the tool misbehaving): the certbot
# HTTP-01 challenge itself will fail with its own clear, actionable error
# in that case, and manufacturing a second, less informative failure on
# top of that helps no one. Stopping nginx and freeing TCP/80 is
# different — if THAT fails, continuing would only let certbot fail with
# a confusing EADDRINUSE we could have diagnosed better ourselves, so
# this hook fails closed (see check_port_80_free below) with a specific,
# actionable error instead, and never continues with port 80 in a state
# it did not itself verify.
set -u

: "${SINGBOX_VPN_CERTBOT_PORT80_MARKER:=/run/singbox-vpn-certbot-port80.opened}"
: "${SINGBOX_VPN_CERTBOT_NGINX_MARKER:=/run/singbox-vpn-certbot-nginx-stopped.opened}"
: "${SINGBOX_VPN_CERTBOT_RENEWAL_LOCK:=/run/lock/singbox-vpn-certbot-renewal.lock}"

log() { echo "[certbot-firewall-pre-hook] $*"; }
err() { echo "[certbot-firewall-pre-hook] ERROR: $*" >&2; }

# Names the actual TCP/80 occupant when possible (never assumes it's
# nginx — something else, an operator's own service, or a leftover
# process could hold it) without ever killing anything. Mirrors
# deploy/lib/preflight.sh's preflight_check_port_free(), duplicated
# locally rather than sourced: these renewal hooks are installed as
# self-contained copies (see certbot-deploy-hook.sh for the same
# convention) so they keep working even if /opt/singbox-vpn is ever
# absent when certbot's timer fires.
port_80_owner() {
  command -v ss >/dev/null 2>&1 || return 0
  ss -H -lntp 2>/dev/null | awk '$4 ~ /:80$/ {print; exit}'
}

nginx_was_active=0
if command -v nginx >/dev/null 2>&1 && systemctl is-active --quiet nginx 2>/dev/null; then
  nginx_was_active=1
fi

(
  # Short, local critical section around the nginx stop + marker write —
  # see the concurrency note above for exactly what this does and does
  # not guarantee. -w 30: never hang the renewal indefinitely on a stuck
  # lock; a held lock this old is itself a real problem worth surfacing
  # via certbot's own failure rather than blocking forever.
  flock -w 30 200 || { err "could not acquire $SINGBOX_VPN_CERTBOT_RENEWAL_LOCK within 30s — another singbox-vpn certbot transition appears stuck."; exit 1; }

  if [ "$nginx_was_active" -eq 1 ]; then
    if systemctl stop nginx 2>/dev/null && ! systemctl is-active --quiet nginx 2>/dev/null; then
      # The marker means exactly one thing: nginx WAS active AND
      # singbox-vpn itself just confirmed it is now stopped — never
      # "an attempt was made." Only this confirmed state authorizes the
      # post-hook to restart it later.
      : > "$SINGBOX_VPN_CERTBOT_NGINX_MARKER" 2>/dev/null
      log "temporarily stopped nginx (it has no role in the ACME HTTP-01 challenge but was occupying TCP/80)."
    else
      err "nginx did not actually stop; refusing to proceed with a renewal attempt that would fail with a confusing EADDRINUSE."
      systemctl start nginx >/dev/null 2>&1 || true
      exit 1
    fi
  fi

  owner="$(port_80_owner)"
  if [ -n "$owner" ]; then
    err "TCP/80 is still occupied after stopping nginx; Certbot standalone HTTP-01 cannot start."
    err "occupant: $owner"
    if [ "$nginx_was_active" -eq 1 ]; then
      systemctl start nginx >/dev/null 2>&1 || true
      rm -f "$SINGBOX_VPN_CERTBOT_NGINX_MARKER"
    fi
    exit 1
  fi
  exit 0
) 200>"$SINGBOX_VPN_CERTBOT_RENEWAL_LOCK" || exit 1

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
