#!/usr/bin/env bash
# netem.sh on RTT_MS LOSS_PCT | off — set one network condition in the lab:
# userspace delay/loss (netem-tun) per leg + an htb-shaped access link toward
# the client (RATE, default 100mbit, QLEN-packet pfifo, default 300).
#   client<->exit   RTT_MS, LOSS_PCT
#   client<->inet   RTT_MS + 2 ms, LOSS_PCT     (no-tunnel baseline)
#   client<->relay  RTT_MS - RELAY_EXIT_RTT (min 2 ms), LOSS_PCT
#   relay<->exit    RELAY_EXIT_RTT (default 40 ms)
#   exit<->inet     2 ms                         (exit to destination/decoy/DNS)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
RATE="${RATE:-100mbit}"; QLEN="${QLEN:-300}"; RELAY_EXIT_RTT="${RELAY_EXIT_RTT:-40}"
mkdir -p "$RUN"

on() { # $1 rtt ms, $2 loss pct
  local d l rt; d="$(echo "$1 / 2" | bc -l)"; l="$2"; rt="$(echo "$RELAY_EXIT_RTT / 2" | bc -l)"
  # two-hop: client<->relay gets the impaired access leg (rtt minus relay-exit leg, min 2ms)
  local cr; cr="$(echo "x=($1 - $RELAY_EXIT_RTT)/2; if (x<1) x=1; x" | bc -l)"
  cat >"$RUN/rules.json" <<J
[{"src":"11.0.1.0/24","dst":"11.0.2.0/24","delay_ms":$d,"loss_pct":$l},
 {"src":"11.0.2.0/24","dst":"11.0.1.0/24","delay_ms":$d,"loss_pct":$l},
 {"src":"11.0.1.0/24","dst":"11.0.3.0/24","delay_ms":$(echo "$d + 1" | bc -l),"loss_pct":$l},
 {"src":"11.0.3.0/24","dst":"11.0.1.0/24","delay_ms":$(echo "$d + 1" | bc -l),"loss_pct":$l},
 {"src":"11.0.1.0/24","dst":"11.0.4.0/24","delay_ms":$cr,"loss_pct":$l},
 {"src":"11.0.4.0/24","dst":"11.0.1.0/24","delay_ms":$cr,"loss_pct":$l},
 {"src":"11.0.4.0/24","dst":"11.0.2.0/24","delay_ms":$rt},
 {"src":"11.0.2.0/24","dst":"11.0.4.0/24","delay_ms":$rt},
 {"src":"11.0.2.0/24","dst":"11.0.3.0/24","delay_ms":1},
 {"src":"11.0.3.0/24","dst":"11.0.2.0/24","delay_ms":1}]
J
  if [ -f "$RUN/netem.pid" ]; then kill "$(cat "$RUN/netem.pid")" 2>/dev/null || true; fi
  sleep 0.3
  ip netns exec rtr taskset -c 3 "$LAB_WORK/bin/netem-tun" -dev tun0 -rules "$RUN/rules.json" >"$RUN/netem.log" 2>&1 &
  echo $! >"$RUN/netem.pid"; sleep 0.4
  "$HERE/lab-net.sh" netem_on
  # access-link bottleneck toward the client (download direction)
  ip netns exec rtr tc qdisc del dev r1 root 2>/dev/null || true
  ip netns exec rtr tc qdisc add dev r1 root handle 1: htb default 10 r2q 100
  ip netns exec rtr tc class add dev r1 parent 1: classid 1:10 htb rate "$RATE" ceil "$RATE"
  ip netns exec rtr tc qdisc add dev r1 parent 1:10 handle 10: pfifo limit "$QLEN"
}

off() {
  "$HERE/lab-net.sh" netem_off
  ip netns exec rtr tc qdisc del dev r1 root 2>/dev/null || true
  if [ -f "$RUN/netem.pid" ]; then kill "$(cat "$RUN/netem.pid")" 2>/dev/null || true; fi
  rm -f "$RUN/netem.pid"
}

"$@"
