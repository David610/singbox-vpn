#!/usr/bin/env bash
# `vpn-health-check` — spec §42 smoke test. Never prints secrets. Exits
# non-zero if any required component fails.
#
# Three explicitly separate categories (docs/FINAL_PRODUCTION_AUDIT.md
# P0-13) — a passing "local health" section does NOT prove "external
# reachability", and neither proves a real client can actually complete a
# VLESS/Hysteria2 handshake:
#   - local health:    services active, config valid, key/cert present
#   - TLS validity:    the subscription HTTPS endpoint is checked with a
#                       REAL hostname and REAL trust-chain validation, not
#                       `curl -k` against a bare IP (which proves nothing
#                       about what an actual client would accept)
#   - external/protocol: explicitly NOT attempted by this script — see the
#                       final section.
#
# What this script does NOT check, and where that lives instead: whether
# the REALITY key material sing-box is actually enforcing agrees with
# what vpn-subscription is currently advertising to new clients (`vpn-
# admin doctor`, tagged [L4]), and whether a real client can actually
# complete a REALITY handshake (`vpn-admin doctor --protocol`, tagged
# [L5-6], best-effort). Passing every check in this script is L1-L3 only
# — it is not proof a real client can connect.
set -uo pipefail

STATE_DIR="/etc/vpn/compat"
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
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

echo "== local health =="
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

echo
echo "== TLS validity (real hostname, real trust chain — no -k) =="
SUBSCRIPTION_HOST=""
SUBSCRIPTION_PORT="8443"
if [ -f "$DEPLOYMENT_TOML" ]; then
  SUBSCRIPTION_HOST="$(grep -E '^subscription_host' "$DEPLOYMENT_TOML" | sed -E 's/^subscription_host *= *"([^"]*)".*/\1/')"
  existing_port="$(grep -E '^public_port' "$DEPLOYMENT_TOML" | sed -E 's/^public_port *= *([0-9]+).*/\1/')"
  [ -n "$existing_port" ] && SUBSCRIPTION_PORT="$existing_port"
fi
if [ -f /etc/nginx/conf.d/vpn-subscription.conf ]; then
  if [ -z "$SUBSCRIPTION_HOST" ]; then
    printf "%-24s FAIL %s\n" "nginx subscription vhost" "(could not read subscription_host from $DEPLOYMENT_TOML)"
    FAIL=1
  else
    # --resolve pins the hostname to the loopback socket WITHOUT
    # disabling verification: the cert's CN/SAN and trust chain are
    # still checked exactly as a real client resolving that hostname
    # over the internet would check them. This is what actually
    # exercises whether a client's TLS stack would accept the
    # certificate — `curl -k` never did.
    check "nginx subscription vhost reachable+trusted (${SUBSCRIPTION_PORT}, hostname=$SUBSCRIPTION_HOST)" \
      curl -fsS --resolve "${SUBSCRIPTION_HOST}:${SUBSCRIPTION_PORT}:127.0.0.1" -o /dev/null \
        "https://${SUBSCRIPTION_HOST}:${SUBSCRIPTION_PORT}/healthz"
    if [ -f "/etc/letsencrypt/live/$SUBSCRIPTION_HOST/fullchain.pem" ]; then
      check "subscription TLS cert not expiring within 14 days" \
        openssl x509 -in "/etc/letsencrypt/live/$SUBSCRIPTION_HOST/fullchain.pem" -noout -checkend $((14 * 86400))
    fi
  fi
else
  echo "nginx subscription vhost         NOT CONFIGURED (run install.sh again once a subscription TLS cert is present)"
fi

echo
echo "== subscription/server key coherence (run 'vpn-admin doctor') =="
echo "  This script does not compare the REALITY key material sing-box is"
echo "  actually enforcing against what vpn-subscription would hand a new"
echo "  client right now — that class of drift (server and subscription"
echo "  disagreeing about REALITY keys) passed every check above in a"
echo "  real incident while still failing a real client's handshake."
echo "  'sudo vpn-admin doctor' adds that check, tagged [L4], from file"
echo "  contents alone (no network needed); 'sudo vpn-admin doctor"
echo "  --protocol' goes further and dials this server's own REALITY"
echo "  listener with a throwaway sing-box client, tagged [L5-6]."

echo
echo "== external reachability / real protocol test (NOT performed here) =="
echo "  Everything above runs from localhost against localhost sockets."
echo "  It proves the services are up and that TLS is configured"
echo "  correctly for the hostname this host thinks it has — it does NOT"
echo "  prove this VPS is reachable from the public internet (that"
echo "  depends on upstream routing/provider firewall/NAT, none of which"
echo "  this script can see from inside the host), and it does NOT prove"
echo "  a real VLESS+REALITY or Hysteria2 client can complete a full"
echo "  handshake end-to-end. See 'vpn-admin doctor --protocol' for a"
echo "  best-effort loopback self-test, deploy/almalinux/acceptance-test.sh"
echo "  for the listener/access-matrix checks, and"
echo "  docs/DEVICE_ACCEPTANCE_TESTS.md for what real-client verification"
echo "  requires and whether it has actually been performed."

exit $FAIL
