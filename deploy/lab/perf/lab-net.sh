#!/usr/bin/env bash
# Performance-lab network: leaf namespaces around one router namespace whose
# forwarding path can be diverted through netem-tun (userspace delay/loss),
# for kernels that ship without sch_netem.
#   cli   11.0.1.2   client side (sing-box client + probes)
#   exit  11.0.2.2   VPN exit (sing-box server under test)
#   inet  11.0.3.2   "internet": iperf3, bulk HTTP, echo, decoy TLS, DNS
#   relay 11.0.4.2   first hop for two-hop tests
#   pair  11.0.5.2   spare node (hairpin experiments)
# Addresses are deliberately outside every C-16 deny range (RFC1918, CGNAT,
# TEST-NET, benchmarking), so the production exit policy stays intact.
# DISPOSABLE MACHINES ONLY: creates and deletes network namespaces and
# /etc/netns/<name>/resolv.conf. Never run it on a VPN server.
set -euo pipefail
# Refuse to run on a VPN node: the installer persists this tree to
# /opt/singbox-vpn, so these scripts can exist on a production host.
if [ -e /etc/vpn/deployment.toml ] || systemctl is-active --quiet sing-box 2>/dev/null; then
  if [ "${LAB_I_KNOW_THIS_IS_NOT_A_VPN_SERVER:-}" != 1 ]; then
    echo "refusing: this host looks like a singbox-vpn node (/etc/vpn/deployment.toml or an active sing-box.service). The perf lab is for disposable machines only." >&2
    exit 1
  fi
fi
NS=(cli exit inet relay pair)
idx() { case "$1" in cli) echo 1;; exit) echo 2;; inet) echo 3;; relay) echo 4;; pair) echo 5;; esac; }

up() {
  ip netns add rtr
  # Mirror deploy/lib/perf-tuning.sh's socket-buffer ceilings (the 2026-10-14
  # runs used 16 MiB; some kernels keep these per namespace, others globally).
  sysctl -qw net.core.rmem_max=16777216 net.core.wmem_max=16777216
  ip netns exec rtr sysctl -qw net.ipv4.ip_forward=1
  ip netns exec rtr sysctl -qw net.ipv4.conf.all.rp_filter=0
  ip netns exec rtr sysctl -qw net.ipv4.conf.default.rp_filter=0
  ip -n rtr link set lo up
  for ns in "${NS[@]}"; do
    i="$(idx "$ns")"
    ip netns add "$ns"
    ip link add "r$i" type veth peer name "${ns}0"
    ip link set "r$i" netns rtr
    ip link set "${ns}0" netns "$ns"
    ip -n rtr addr add "11.0.$i.1/24" dev "r$i"
    ip -n rtr link set "r$i" up
    ip -n "$ns" addr add "11.0.$i.2/24" dev "${ns}0"
    ip -n "$ns" link set "${ns}0" up
    ip -n "$ns" link set lo up
    ip -n "$ns" route add default via "11.0.$i.1"
    ip netns exec rtr sysctl -qw "net.ipv4.conf.r$i.rp_filter=0"
    ip netns exec "$ns" sysctl -qw net.core.rmem_max=16777216 net.core.wmem_max=16777216 2>/dev/null || true
    mkdir -p "/etc/netns/$ns"
    echo "nameserver 11.0.3.2" > "/etc/netns/$ns/resolv.conf"
  done
}

# Divert all forwarded traffic through tun0 (netem-tun must already own tun0
# inside rtr). Without this, rtr forwards in-kernel with no impairment.
netem_on() {
  ip -n rtr link set tun0 up
  ip -n rtr link set tun0 mtu 1500
  ip netns exec rtr sysctl -qw net.ipv4.conf.tun0.rp_filter=0
  ip -n rtr route replace default dev tun0 table 100
  for ns in "${NS[@]}"; do
    i="$(idx "$ns")"
    ip -n rtr rule add iif "r$i" lookup 100 pref $((100 + i)) 2>/dev/null || true
  done
}
netem_off() {
  for ns in "${NS[@]}"; do
    i="$(idx "$ns")"
    ip -n rtr rule del iif "r$i" lookup 100 2>/dev/null || true
  done
}

down() {
  for ns in "${NS[@]}" rtr; do ip netns del "$ns" 2>/dev/null || true; rm -rf "/etc/netns/$ns"; done
}

"$@"
