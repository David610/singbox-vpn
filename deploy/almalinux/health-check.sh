#!/usr/bin/env bash
# `vpn-health-check` — spec §42 smoke test. Never prints secrets. Exits
# non-zero if any required component fails.
set -uo pipefail

STATE_DIR="/etc/vpn/compat"
FAIL=0

check() {
  local name="$1" ; shift
  if "$@" >/dev/null 2>&1; then
    printf "%-24s OK\n" "$name"
  else
    printf "%-24s FAIL\n" "$name"
    FAIL=1
  fi
}

check "sing-box service" systemctl is-active --quiet sing-box
check "subscription service" systemctl is-active --quiet vpn-subscription
check "sing-box binary" test -x /usr/local/bin/sing-box
check "sing-box config valid" /usr/local/bin/sing-box check -c "$STATE_DIR/sing-box/config.json"
check "REALITY key present" test -s "$STATE_DIR/reality/private.key"
if command -v firewall-cmd >/dev/null 2>&1; then
  check "firewall TCP/443" bash -c "firewall-cmd --list-ports | grep -qw 443/tcp"
  check "firewall UDP/443" bash -c "firewall-cmd --list-ports | grep -qw 443/udp"
elif command -v ufw >/dev/null 2>&1; then
  check "firewall TCP/443" bash -c "ufw status | grep -qE '443/tcp\s+ALLOW'"
  check "firewall UDP/443" bash -c "ufw status | grep -qE '443/udp\s+ALLOW'"
else
  echo "firewall                     UNKNOWN (neither firewalld nor ufw found)"
fi
check "subscription loopback reachable" curl -fsS -o /dev/null http://127.0.0.1:9100/healthz

if command -v ss >/dev/null 2>&1; then
  check "VLESS+REALITY listening (443/tcp)" bash -c "ss -tlnp 2>/dev/null | grep -q ':443 '"
  check "Hysteria2 listening (443/udp)" bash -c "ss -ulnp 2>/dev/null | grep -q ':443 '"
fi

check "Hysteria TLS cert present" test -s "$STATE_DIR/hysteria/cert.pem"
if [ -s "$STATE_DIR/hysteria/cert.pem" ]; then
  check "Hysteria TLS cert not expired" openssl x509 -in "$STATE_DIR/hysteria/cert.pem" -noout -checkend 0
fi
if [ -f /etc/nginx/conf.d/vpn-subscription.conf ]; then
  check "nginx subscription vhost reachable (8443)" \
    curl -fsSk -o /dev/null "https://127.0.0.1:8443/healthz"
else
  echo "nginx subscription vhost         NOT CONFIGURED (run install.sh again once a subscription TLS cert is present)"
fi

exit $FAIL
