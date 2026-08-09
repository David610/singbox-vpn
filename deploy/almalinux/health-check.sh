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
check "firewall TCP/443" bash -c "firewall-cmd --list-ports | grep -qw 443/tcp"
check "firewall UDP/443" bash -c "firewall-cmd --list-ports | grep -qw 443/udp"
check "subscription loopback reachable" curl -fsS -o /dev/null http://127.0.0.1:9100/healthz

if command -v ss >/dev/null 2>&1; then
  check "VLESS+REALITY listening (443/tcp)" bash -c "ss -tlnp 2>/dev/null | grep -q ':443 '"
  check "Hysteria2 listening (443/udp)" bash -c "ss -ulnp 2>/dev/null | grep -q ':443 '"
fi

exit $FAIL
