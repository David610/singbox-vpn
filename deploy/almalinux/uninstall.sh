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
rm -f /etc/systemd/system/sing-box.service /etc/systemd/system/vpn-subscription.service
systemctl daemon-reload

rm -f /usr/local/bin/vpn-admin /usr/local/bin/vpn-subscription-svc /usr/local/bin/vpn-health-check
echo "sing-box binary left at /usr/local/bin/sing-box (remove manually if desired)."

if [ "$PURGE_FIREWALL" -eq 1 ]; then
  ZONE="$(firewall-cmd --get-default-zone)"
  firewall-cmd --zone="$ZONE" --permanent --remove-port=443/tcp || true
  firewall-cmd --zone="$ZONE" --permanent --remove-port=443/udp || true
  firewall-cmd --zone="$ZONE" --permanent --remove-port=8443/tcp || true
  firewall-cmd --reload
fi

if [ "$PURGE_STATE" -eq 1 ]; then
  echo "purging /etc/vpn (REALITY private key, all user credentials) — this cannot be undone."
  rm -rf /etc/vpn
else
  echo "state kept at /etc/vpn (REALITY key, users.json, deployment.toml). Pass --purge-state to remove it."
fi

echo "uninstall complete."
