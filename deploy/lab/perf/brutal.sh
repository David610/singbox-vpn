#!/usr/bin/env bash
# brutal.sh DECLARED_DOWN_MBPS [DURATION] -> JSON lines (class C, lab only)
#
# Hysteria2 fixed-rate ("Brutal") behaviour on the shaped access link that
# matrix.sh sets up (netem + htb must already be on for the condition you
# want). The exit must run with bandwidth caps, as the renderer emits them:
#   lab.sh server_up_caps exit exit 1 1000 1000
# A client process that DECLARES DECLARED_DOWN_MBPS gets Brutal at that rate
# (capped by the server's up_mbps); compare with the BBR client (cH, which
# declares nothing) under the same condition.
set -uo pipefail
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
HERE="$(cd "$(dirname "$0")" && pwd)"
SB="${SB:-/usr/local/bin/sing-box}"
LT="$LAB_WORK/bin/labtool"
declared="$1" dur="${2:-15}"
pids=()
cleanup() { local p; for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; }
trap cleanup EXIT

jq --argjson d "$declared" '
  .inbounds = [{type:"mixed", tag:"socks", listen:"127.0.0.1", listen_port:1083},
               {type:"direct", tag:"fwd0", listen:"127.0.0.1", listen_port:15501, network:"tcp",
                override_address:"11.0.3.2", override_port:5201}]
  | (.outbounds[] | select(.type=="hysteria2")) |= (.down_mbps = $d | .up_mbps = 50)' \
  "$RUN/cH.json" >"$RUN/cB.json"
ip netns exec cli taskset -c 2 "$SB" run -c "$RUN/cB.json" >"$RUN/cB.log" 2>&1 &
pids+=("$!")
echo "$!" >"$RUN/cB.pid"
sleep 1.5

drops() { ip netns exec rtr tc -s qdisc show dev r1 | awk '/pfifo/{getline; for(i=1;i<=NF;i++) if($i=="dropped") {gsub(",","",$(i+1)); print $(i+1); exit}}'; }
d0="$(drops)"
(CLIENT_PID_NAME=cB "$HERE/tput.sh" "brutal-$declared" 15501 "$dur" 1 1 >"$RUN/brutal-tput.json") &
tp=$!
sleep 2
ip netns exec cli "$LT" rtt -socks 127.0.0.1:1083 -target 11.0.3.2:7 -duration "$((dur - 4))s" \
  -interval 50ms -name "brutal-$declared" >"$RUN/brutal-rtt.json" 2>/dev/null
wait "$tp"
d1="$(drops)"
jq -c --argjson declared "$declared" --argjson qdrops "$(( ${d1:-0} - ${d0:-0} ))" \
  --argjson lat "$(cat "$RUN/brutal-rtt.json")" \
  '. + {declared_down_mbps:$declared, bottleneck_queue_drops:$qdrops,
        loaded_rtt_p50:$lat.p50_ms, loaded_rtt_p95:$lat.p95_ms, loaded_rtt_p99:$lat.p99_ms}' \
  "$RUN/brutal-tput.json"
