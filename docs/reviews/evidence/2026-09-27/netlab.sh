#!/usr/bin/env bash
# Lab topology: router ns "inet" connects client, relay, exit, target.
set -euo pipefail
for ns in inet client relay exit target; do ip netns del $ns 2>/dev/null || true; done
for ns in inet client relay exit target; do ip netns add $ns; ip -n $ns link set lo up; done
i=1
for ns in client relay exit target; do
  ip link add "v-$ns" type veth peer name "r-$ns"
  ip link set "v-$ns" netns $ns
  ip link set "r-$ns" netns inet
  ip -n $ns addr add 10.0.$i.2/24 dev "v-$ns"
  ip -n inet addr add 10.0.$i.1/24 dev "r-$ns"
  ip -n $ns link set "v-$ns" up
  ip -n inet link set "r-$ns" up
  ip -n $ns route add default via 10.0.$i.1
  i=$((i+1))
done
ip netns exec inet sysctl -qw net.ipv4.ip_forward=1
# simulated cloud metadata service reachable from exit via its gateway
ip -n inet addr add 169.254.169.254/32 dev lo
mkdir -p /etc/netns/exit /etc/netns/relay /etc/netns/client /etc/netns/target
echo "nameserver 10.0.4.2" > /etc/netns/exit/resolv.conf
echo "nameserver 10.0.4.2" > /etc/netns/relay/resolv.conf
echo "nameserver 10.0.4.2" > /etc/netns/client/resolv.conf
echo "nameserver 127.0.0.1" > /etc/netns/target/resolv.conf
for f in exit relay client target; do printf '127.0.0.1 localhost\n' > /etc/netns/$f/hosts; done
echo lab-up
