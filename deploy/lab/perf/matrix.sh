#!/usr/bin/env bash
# Controlled-network matrix (category C: simulated, not real Internet).
# For each condition: netem.sh sets delay/loss per leg and shapes the
# client access link, then this script measures no-tunnel / REALITY / Hysteria2 /
# two-hop REALITY. Results: one JSON object per line on stdout.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; LT="$LAB_WORK/bin/labtool"
REPS="${REPS:-3}"; DUR="${DUR:-15}"; RATE="${RATE:-100mbit}"
CONDS="${CONDS:-20:0 80:0 160:0 80:1 160:1}"   # rttms:losspct (client<->exit leg)
TRANSPORTS="${TRANSPORTS:-direct reality hy2 twohop}"
# RATE (htb), QLEN and RELAY_EXIT_RTT are read by netem.sh from the environment.

port_of() { case "$1" in direct) echo 5201;; reality) echo 15201;; hy2) echo 15301;; twohop) echo 15401;; esac; }
socks_of() { case "$1" in reality) echo 1080;; hy2) echo 1081;; twohop) echo 1082;; esac; }
client_of() { case "$1" in reality) echo cR;; hy2) echo cH;; twohop) echo c2;; *) echo none;; esac; }

for cond in $CONDS; do
  rtt="${cond%%:*}"; loss="${cond##*:}"
  "$HERE/netem.sh" on "$rtt" "$loss"
  for t in $TRANSPORTS; do
    host=127.0.0.1; [ "$t" = direct ] && host=11.0.3.2
    for r in $(seq 1 "$REPS"); do
      HOST=$host CLIENT_PID_NAME="$(client_of "$t")" "$HERE/tput.sh" "$t" "$(port_of "$t")" "$DUR" 1 1 \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" --arg rate "$RATE" --argjson rep "$r" \
            '. + {test:"tput1", rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber), rate:$rate, rep:$rep}'
    done
    if [ "$t" != direct ]; then
      s="$(socks_of "$t")"
      ip netns exec cli "$LT" connlat -socks "127.0.0.1:$s" -target 11.0.3.2:7 -n 30 -name "$t" \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" '. + {test:"connlat", rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
      ip netns exec cli "$LT" rtt -socks "127.0.0.1:$s" -target 11.0.3.2:7 -n 100 -interval 20ms -name "$t" \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" '. + {test:"rtt_idle", rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
      # loaded latency: ping-pong on its own connection while a download saturates the link
      (HOST=127.0.0.1 CLIENT_PID_NAME="$(client_of "$t")" "$HERE/tput.sh" "$t" "$(port_of "$t")" 14 1 1 >/dev/null 2>&1 &)
      sleep 2
      ip netns exec cli "$LT" rtt -socks "127.0.0.1:$s" -target 11.0.3.2:7 -duration 10s -interval 50ms -name "$t" \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" '. + {test:"rtt_loaded", rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
      sleep 3
      if [ "$t" = twohop ]; then
        # relay routes are TCP-only by design; each UDP probe would only
        # time out (5 s). One record states it instead of 70 timeouts.
        jq -cn --arg rtt "$rtt" --arg loss "$loss" '{name:"twohop", test:"udp_supported", value:false, rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
        continue
      fi
      ip netns exec cli "$LT" udprtt -socks "127.0.0.1:$s" -target 11.0.3.2:7 -n 20 -fresh -name "$t" \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" '. + {test:"udp_fresh_session", rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
      ip netns exec cli "$LT" udprtt -socks "127.0.0.1:$s" -target 11.0.3.2:7 -n 50 -name "$t" \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" '. + {test:"udp_established", rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
    else
      ip netns exec cli ping -c 20 -i 0.05 -q 11.0.3.2 | awk -F'[/ ]' '/rtt/{printf "{\"name\":\"direct\",\"test\":\"icmp\",\"p50_ms\":%s}\n", $8}' \
        | jq -c --arg rtt "$rtt" --arg loss "$loss" '. + {rtt_ms:($rtt|tonumber), loss_pct:($loss|tonumber)}'
    fi
  done
done
"$HERE/netem.sh" off
