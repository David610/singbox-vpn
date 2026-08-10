#!/usr/bin/env bash
# Reverses install.sh. Never deletes secrets/state by default — only
# stops/disables services and removes binaries/units. Pass
# --purge-state to also remove /etc/vpn/compat (irreversible: destroys
# the REALITY private key and every user's credentials) and
# --purge-firewall to close the ports again.
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

PURGE_STATE=0
PURGE_FIREWALL=0
for arg in "$@"; do
  case "$arg" in
    --purge-state) PURGE_STATE=1 ;;
    --purge-firewall) PURGE_FIREWALL=1 ;;
    *) echo "unknown flag: $arg" >&2; exit 1 ;;
  esac
done

systemctl disable --now sing-box.service 2>/dev/null || true
systemctl disable --now vpn-subscription.service 2>/dev/null || true
systemctl disable --now vpn-expiry-reconcile.timer 2>/dev/null || true
rm -f /etc/systemd/system/sing-box.service /etc/systemd/system/vpn-subscription.service \
  /etc/systemd/system/vpn-expiry-reconcile.service /etc/systemd/system/vpn-expiry-reconcile.timer
systemctl daemon-reload

rm -f /usr/local/bin/vpn-admin /usr/local/bin/vpn /usr/local/bin/vpn-subscription-svc /usr/local/bin/vpn-health-check
echo "sing-box binary left at /usr/local/bin/sing-box (remove manually if desired)."

# Leaving the deploy hook installed breaks EVERY future `certbot renew` on
# this host: the hook's own guards still pass (deployment.toml and the
# sing-box binary survive an uninstall by default), so it runs and then fails
# trying to reload a unit this script just deleted — and certbot reports the
# whole renewal as failed, including for unrelated certificates.
rm -f /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh
rm -f /var/lib/vpn1/install-state.json
if [ -f /etc/nginx/conf.d/vpn-subscription.conf ]; then
rm -f /etc/nginx/conf.d/vpn-subscription.conf
  if nginx -t >/dev/null 2>&1; then systemctl reload nginx 2>/dev/null || true; fi
  echo "removed /etc/nginx/conf.d/vpn-subscription.conf (nginx itself left installed)."
fi

if [ "$PURGE_FIREWALL" -eq 1 ]; then
  FIREWALL_OWNERSHIP=/var/lib/vpn1/firewall-owned.env
  if [ -f "$FIREWALL_OWNERSHIP" ]; then
    # Root-only installer-owned file; records exactly which rules were absent
    # before vpn1 added them, plus the install-time backend/zone.
    firewall_backend=""
    firewall_zone=""
    subscription_port=""
    owned_443_tcp=0
    owned_443_udp=0
    owned_subscription_tcp=0
    # shellcheck disable=SC1090
    . "$FIREWALL_OWNERSHIP"
    case "$owned_443_tcp:$owned_443_udp:$owned_subscription_tcp" in
      0:0:0|0:0:1|0:1:0|0:1:1|1:0:0|1:0:1|1:1:0|1:1:1) ;;
      *) echo "invalid vpn1 firewall ownership record" >&2; exit 1 ;;
    esac
    if [ "$owned_subscription_tcp" -eq 1 ] &&
       ! [[ "$subscription_port" =~ ^[0-9]+$ ]] ; then
      echo "invalid subscription port in vpn1 firewall ownership record" >&2
      exit 1
    fi
    if [ "${firewall_backend:-}" = "firewalld" ] && command -v firewall-cmd >/dev/null 2>&1; then
      [[ "$firewall_zone" =~ ^[A-Za-z0-9_.-]+$ ]] || {
        echo "invalid firewalld zone in vpn1 firewall ownership record" >&2
        exit 1
      }
      [ "${owned_443_tcp:-0}" -eq 1 ] && firewall-cmd --zone="$firewall_zone" --permanent --remove-port=443/tcp || true
      [ "${owned_443_udp:-0}" -eq 1 ] && firewall-cmd --zone="$firewall_zone" --permanent --remove-port=443/udp || true
      [ "${owned_subscription_tcp:-0}" -eq 1 ] && firewall-cmd --zone="$firewall_zone" --permanent --remove-port="${subscription_port}/tcp" || true
      firewall-cmd --reload
    elif [ "${firewall_backend:-}" = "ufw" ] && command -v ufw >/dev/null 2>&1; then
      [ "${owned_443_tcp:-0}" -eq 1 ] && ufw delete allow 443/tcp || true
      [ "${owned_443_udp:-0}" -eq 1 ] && ufw delete allow 443/udp || true
      [ "${owned_subscription_tcp:-0}" -eq 1 ] && ufw delete allow "${subscription_port}/tcp" || true
    fi
    rm -f "$FIREWALL_OWNERSHIP"
  else
    echo "no vpn1 firewall ownership record found; refusing to remove possibly pre-existing operator rules." >&2
  fi
fi

if [ "$PURGE_STATE" -eq 1 ]; then
  echo "purging /etc/vpn (REALITY private key, all user credentials) — this cannot be undone."
  rm -rf /etc/vpn
else
  echo "state kept at /etc/vpn (REALITY key, users.json, deployment.toml). Pass --purge-state to remove it."
fi

echo "uninstall complete."
