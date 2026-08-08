#!/usr/bin/env bash
# Sets up two network namespaces joined by a veth pair, applies netem/
# iptables hostile-network conditions, and runs the relevant integration
# tests against the constrained link. Requires root + iproute2 + iptables.
# See README.md in this directory for why this isn't run automatically.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "must run as root (needs CAP_NET_ADMIN for netns/tc/iptables)" >&2
  exit 1
fi
for bin in ip tc iptables; do
  command -v "$bin" >/dev/null || { echo "missing required binary: $bin" >&2; exit 1; }
done

cleanup() {
  ip netns del vpn-client 2>/dev/null || true
  ip netns del vpn-relay 2>/dev/null || true
}
trap cleanup EXIT

cleanup
ip netns add vpn-client
ip netns add vpn-relay
ip link add veth-client type veth peer name veth-relay
ip link set veth-client netns vpn-client
ip link set veth-relay netns vpn-relay
ip netns exec vpn-client ip addr add 10.99.0.1/24 dev veth-client
ip netns exec vpn-relay ip addr add 10.99.0.2/24 dev veth-relay
ip netns exec vpn-client ip link set veth-client up
ip netns exec vpn-relay ip link set veth-relay up
ip netns exec vpn-client ip link set lo up
ip netns exec vpn-relay ip link set lo up

run_scenario() {
  local name="$1"
  shift
  echo "=== scenario: $name ==="
  "$@"
  # The relay-side binaries run inside vpn-relay; the test binary runs
  # inside vpn-client so it experiences whatever netem/iptables rules were
  # just applied to veth-client.
  ip netns exec vpn-relay \
    env RELAY_BIND_TLS=10.99.0.2:9443 RELAY_BIND_QUIC=10.99.0.2:9444 \
    ./target/debug/relay-agent --role combined --bind-tls 10.99.0.2:9443 --bind-quic 10.99.0.2:9444 &
  local relay_pid=$!
  sleep 1
  ip netns exec vpn-client \
    env RELAY_TLS_ADDR=10.99.0.2:9443 RELAY_QUIC_ADDR=10.99.0.2:9444 \
    cargo test -p tests --test hostile_network_scenario -- --ignored --nocapture || true
  kill "$relay_pid" 2>/dev/null || true
  ip netns exec vpn-client tc qdisc del dev veth-client root 2>/dev/null || true
  ip netns exec vpn-client iptables -F 2>/dev/null || true
}

run_scenario "packet-loss-30pct" \
  ip netns exec vpn-client tc qdisc add dev veth-client root netem loss 30%

run_scenario "latency-300ms-jitter-100ms" \
  ip netns exec vpn-client tc qdisc add dev veth-client root netem delay 300ms 100ms

run_scenario "udp-blocked" \
  ip netns exec vpn-client iptables -A OUTPUT -p udp --dport 9444 -j DROP

run_scenario "tcp-blocked" \
  ip netns exec vpn-client iptables -A OUTPUT -p tcp --dport 9443 -j DROP

echo "done"
