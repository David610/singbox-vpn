#!/usr/bin/env bash
# Acceptance test for a real AlmaLinux 9 deployment
# (docs/PRODUCTION_HARDENING_PLAN.md #21/#25). Run on a fresh VPS AFTER
# install.sh has completed successfully:
#
#   sudo ./deploy/almalinux/acceptance-test.sh
#
# Distinguishing container smoke-test from real VM/VPS test: this
# script checks SELinux enforcement, firewalld state, systemd unit
# health, and low-port capability behavior — none of which a plain
# Docker/Podman container can prove (no SELinux, often no systemd PID
# 1, no real firewalld). Running this INSIDE a container will fail
# several checks by construction; that is expected and does not mean
# the checks are wrong — it means a container is not a substitute for
# a real host/VM here. CI's `singbox-validate` job (.github/workflows/
# ci.yml) is the container-appropriate subset (config rendering +
# `sing-box check` only).
#
# Never prints secrets. Exits non-zero on any REQUIRED failure.
set -uo pipefail

STATE_DIR="/etc/vpn/compat"
REQUIRED_FAIL=0

pass() { printf "%-46s PASS\n" "$1"; }
fail() { printf "%-46s FAIL %s\n" "$1" "${2:-}"; }
fail_required() { printf "%-46s FAIL (required) %s\n" "$1" "${2:-}"; REQUIRED_FAIL=1; }

section() { echo; echo "== $1 =="; }

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

section "OS"
if [ -f /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}-${VERSION_ID:-}" in
    almalinux-9*) pass "AlmaLinux 9 detected" ;;
    *) fail "AlmaLinux 9 detected" "(found ${PRETTY_NAME:-unknown} — container smoke test only)" ;;
  esac
else
  fail "AlmaLinux 9 detected" "(/etc/os-release missing)"
fi

section "SELinux"
if command -v getenforce >/dev/null 2>&1; then
  mode="$(getenforce)"
  if [ "$mode" = "Enforcing" ]; then
    pass "SELinux enforcing"
  else
    fail "SELinux enforcing" "(mode: $mode — not a container limitation, this must be fixed on a real host)"
  fi
else
  fail "SELinux enforcing" "(getenforce not found — container has no SELinux; not testable here)"
fi

section "firewalld"
if command -v firewall-cmd >/dev/null 2>&1 && systemctl is-active --quiet firewalld 2>/dev/null; then
  pass "firewalld active"
else
  fail "firewalld active" "(not running — container may lack systemd/firewalld entirely)"
fi

section "file ownership / permissions"
check_owner_mode() {
  local path="$1" want_owner="$2" want_mode="$3"
  [ -e "$path" ] || { fail_required "$path exists" ; return; }
  local owner mode
  owner="$(stat -c '%U:%G' "$path")"
  mode="$(stat -c '%a' "$path")"
  if [ "$owner" = "$want_owner" ] && [ "$mode" = "$want_mode" ]; then
    pass "$path is $want_owner $want_mode"
  else
    fail_required "$path is $want_owner $want_mode" "(found $owner $mode)"
  fi
}
check_owner_mode "$STATE_DIR/reality/private.key" "root:sing-box" "640"
check_owner_mode "$STATE_DIR/reality/public.key" "root:vpn-subscription" "640"
check_owner_mode "$STATE_DIR/users/users.json" "root:vpn-subscription" "640"
check_owner_mode "$STATE_DIR/sing-box/config.json" "root:sing-box" "640"
check_owner_mode "$STATE_DIR/hysteria/cert.pem" "root:sing-box" "640"
check_owner_mode "$STATE_DIR/hysteria/key.pem" "root:sing-box" "640"

section "effective access (least privilege proof, not just nominal owner)"
access_check() {
  local user="$1" path="$2" expect="$3" # expect: readable | not-readable
  if [ "$expect" = "readable" ]; then
    if sudo -u "$user" test -r "$path" 2>/dev/null; then
      pass "$user can read $path"
    else
      fail_required "$user can read $path" "(required for the service to function)"
    fi
  else
    if sudo -u "$user" test -r "$path" 2>/dev/null; then
      fail_required "$user cannot read $path" "(but CAN — secret over-exposure)"
    else
      pass "$user cannot read $path"
    fi
  fi
}
access_check sing-box "$STATE_DIR/sing-box/config.json" readable
access_check sing-box "$STATE_DIR/hysteria/key.pem" readable
access_check sing-box "$STATE_DIR/reality/private.key" readable
access_check vpn-subscription "$STATE_DIR/users/users.json" readable
access_check vpn-subscription "$STATE_DIR/reality/public.key" readable
access_check vpn-subscription "$STATE_DIR/reality/private.key" not-readable
access_check vpn-subscription "$STATE_DIR/hysteria/key.pem" not-readable
access_check vpn-subscription "$STATE_DIR/sing-box/config.json" not-readable

section "services"
if systemctl is-active --quiet sing-box; then pass "sing-box service active"; else fail_required "sing-box service active"; fi
if systemctl is-active --quiet vpn-subscription; then pass "subscription service active"; else fail_required "subscription service active"; fi

section "config validity"
if /usr/local/bin/sing-box check -c "$STATE_DIR/sing-box/config.json" >/dev/null 2>&1; then
  pass "sing-box check"
else
  fail_required "sing-box check"
fi

section "listeners"
if command -v ss >/dev/null 2>&1; then
  if ss -tln 2>/dev/null | grep -q ':443 '; then pass "TCP 443 listening"; else fail_required "TCP 443 listening"; fi
  if ss -uln 2>/dev/null | grep -q ':443 '; then pass "UDP 443 listening"; else fail_required "UDP 443 listening"; fi
  for p in 1080 9000 9100; do
    if ss -tln 2>/dev/null | grep -qE "0\.0\.0\.0:$p |\[::\]:$p "; then
      fail_required "port $p not publicly listening" "(bound on a non-loopback address)"
    else
      pass "port $p not publicly listening"
    fi
  done
else
  fail "listener checks" "(ss not found)"
fi

section "Hysteria TLS certificate"
cert="$STATE_DIR/hysteria/cert.pem"
if [ -s "$cert" ]; then
  if openssl x509 -in "$cert" -noout -checkend 0 >/dev/null 2>&1; then
    pass "Hysteria TLS cert currently valid"
  else
    fail_required "Hysteria TLS cert currently valid"
  fi
  # warn (not required-fail) if expiring within 14 days
  if openssl x509 -in "$cert" -noout -checkend $((14 * 86400)) >/dev/null 2>&1; then
    pass "Hysteria TLS cert not expiring within 14 days"
  else
    fail "Hysteria TLS cert not expiring within 14 days" "(renew soon)"
  fi
else
  fail_required "Hysteria TLS certificate present"
fi

section "subscription reverse proxy"
# Real hostname, real trust-chain validation — no `-k` (spec:
# docs/FINAL_PRODUCTION_AUDIT.md P0-13, `curl -k` is not proof production
# TLS works, it's proof TLS verification was turned off).
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
# The backend port is configurable ([subscription] listen_port) and the
# deployment.toml template explicitly invites hand-editing, but this probe
# hardcoded 9100 — so changing the port made a HEALTHY deployment fail here
# (and made install.sh's equivalent probe abort a healthy install).
SUBSCRIPTION_BACKEND_PORT="$(awk '/^\[subscription\]/{s=1;next} /^\[/{s=0} s && /^[[:space:]]*listen_port[[:space:]]*=/{gsub(/[^0-9]/,"",$0); print; exit}' "$DEPLOYMENT_TOML" 2>/dev/null || true)"
: "${SUBSCRIPTION_BACKEND_PORT:=9100}"
SUBSCRIPTION_HOST=""
SUBSCRIPTION_PORT="8443"
if [ -f "$DEPLOYMENT_TOML" ]; then
  SUBSCRIPTION_HOST="$(grep -E '^subscription_host' "$DEPLOYMENT_TOML" | sed -E 's/^subscription_host *= *"([^"]*)".*/\1/')"
  existing_port="$(grep -E '^public_port' "$DEPLOYMENT_TOML" | sed -E 's/^public_port *= *([0-9]+).*/\1/')"
  [ -n "$existing_port" ] && SUBSCRIPTION_PORT="$existing_port"
fi
if [ -n "$SUBSCRIPTION_HOST" ] && curl -fsS -o /dev/null --resolve "${SUBSCRIPTION_HOST}:${SUBSCRIPTION_PORT}:127.0.0.1" "https://${SUBSCRIPTION_HOST}:${SUBSCRIPTION_PORT}/healthz" 2>/dev/null; then
  pass "subscription reachable+trusted over HTTPS (${SUBSCRIPTION_PORT}, hostname=$SUBSCRIPTION_HOST)"
else
  fail "subscription reachable+trusted over HTTPS (${SUBSCRIPTION_PORT})" "(nginx vhost may not be configured yet, or subscription_host unreadable from $DEPLOYMENT_TOML — see install.sh output)"
fi

section "health endpoint"
if curl -fsS --connect-timeout 5 --max-time 10 -o /dev/null http://127.0.0.1:${SUBSCRIPTION_BACKEND_PORT}/healthz; then
  pass "subscription /healthz (loopback)"
else
  fail_required "subscription /healthz (loopback)"
fi

section "network failure-independence (documented, not executed here)"
echo "  Real UDP-block / TCP-block failure-independence tests require root"
echo "  network-namespace + nftables/tc manipulation with guaranteed cleanup"
echo "  traps. Not run automatically by this script against a live host's"
echo "  firewall. Reference commands (run manually on a disposable"
echo "  host/VM, never on a shared production firewall):"
echo "    ip netns add vpn-test-ns"
echo "    ip link add veth-host type veth peer name veth-ns"
echo "    ip link set veth-ns netns vpn-test-ns"
echo "    trap 'ip netns del vpn-test-ns 2>/dev/null; ip link del veth-host 2>/dev/null' EXIT"
echo "    nft add rule inet filter output udp dport 443 drop  # then re-test Hysteria2"
echo "    nft flush ruleset                                    # cleanup"
echo "  See docs/PRODUCTION_HARDENING_PLAN.md #20 for the full rationale."

section "optional real transport test"
echo "  Level-5 (actual VLESS+REALITY / Hysteria2 connection through a"
echo "  local sing-box client config to a controlled destination) is not"
echo "  wired into this script — see vpn-health-check --full (not yet"
echo "  implemented; documented follow-up, requires a controlled"
echo "  destination endpoint that this repo does not stand up)."

echo
if [ "$REQUIRED_FAIL" -ne 0 ]; then
  echo "ACCEPTANCE TEST: FAILED (one or more required checks failed)"
  exit 1
fi
echo "ACCEPTANCE TEST: PASSED (all required checks passed)"
exit 0
