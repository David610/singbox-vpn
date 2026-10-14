#!/usr/bin/env bash
# loaded-latency.sh CASE [DURATION] -> one JSON line (class C, lab only)
#
# Where does latency go while a download saturates the access link?
# A 64-byte ping-pong runs on one proxied connection while iperf3 downloads
# through another, under the condition netem.sh set. Cases:
#   hy2-same      download via cH, ping via cH  (same QUIC connection)
#   hy2-separate  download via cH, ping via a second Hysteria2 client process
#                 (its own QUIC connection, same server)
#   reality       download via cR, ping via cR  (separate TCP connections)
#   cross         download via cR, ping via cH  (bottleneck shared only)
#   idle-hy2      no download, ping via cH       (reference)
# Also samples the access-link queue (htb/pfifo backlog on rtr r1) during load.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
LT="$LAB_WORK/bin/labtool"; SB="${SB:-/usr/local/bin/sing-box}"
case_="$1"; dur="${2:-15}"
pids=()
cleanup() { local p; for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; }
trap cleanup EXIT

dl_port=""; dl_client=""; ping_socks=""
case "$case_" in
  hy2-same) dl_port=15301; dl_client=cH; ping_socks=1081 ;;
  hy2-separate)
    dl_port=15301; dl_client=cH; ping_socks=1084
    jq '.inbounds = [{type:"mixed", tag:"socks", listen:"127.0.0.1", listen_port:1084}]' "$RUN/cH.json" >"$RUN/cH2.json"
    ip netns exec cli taskset -c 2 "$SB" run -c "$RUN/cH2.json" >"$RUN/cH2.log" 2>&1 & pids+=("$!")
    sleep 1.5 ;;
  reality) dl_port=15201; dl_client=cR; ping_socks=1080 ;;
  cross) dl_port=15201; dl_client=cR; ping_socks=1081 ;;
  idle-hy2) ping_socks=1081 ;;
  *) echo "unknown case $case_" >&2; exit 2 ;;
esac

if [ -n "$dl_port" ]; then
  (CLIENT_PID_NAME="$dl_client" "$HERE/tput.sh" "$case_" "$dl_port" "$dur" 1 1 >"$RUN/ll-tput.json") &
  pids+=("$!")
  sleep 2
fi
backlog="$RUN/ll-backlog.txt"; : >"$backlog"
(
  end=$(( $(date +%s) + dur - 4 ))
  while [ "$(date +%s)" -lt "$end" ]; do
    ip netns exec rtr tc -s qdisc show dev r1 2>/dev/null \
      | awk '/pfifo/{getline; getline; for(i=1;i<=NF;i++) if($i=="backlog") {sub("p","",$(i+2)); print $(i+2); exit}}' >>"$backlog"
    sleep 0.5
  done
) & pids+=("$!")
ip netns exec cli "$LT" rtt -socks "127.0.0.1:$ping_socks" -target 11.0.3.2:7 -duration "$((dur - 4))s" \
  -interval 50ms -name "$case_" >"$RUN/ll-rtt.json" 2>/dev/null
sleep 3
tput="$(cat "$RUN/ll-tput.json" 2>/dev/null || echo null)"; [ -n "$tput" ] || tput=null
jq -c --arg c "$case_" --argjson t "$tput" \
  --argjson q "$(sort -n "$backlog" | awk '{v[NR]=$1} END{if(NR==0){print "null"} else {printf "{\"median\":%d,\"max\":%d}", v[int((NR+1)/2)], v[NR]}}')" \
  '{case:$c, ping_p50_ms:.p50_ms, ping_p95_ms:.p95_ms, ping_p99_ms:.p99_ms, ping_n:.n, ping_failures:.failures,
    download_mbps:($t.mbps // null), queue_backlog_packets:$q}' "$RUN/ll-rtt.json"
