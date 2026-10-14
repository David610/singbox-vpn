#!/usr/bin/env bash
# concurrency.sh TRANSPORT N DURATION -> one JSON line (class B/C lab only)
#
# N simultaneous downloads through the exit, for capacity and fairness:
#   reality : N proxied TCP connections through one client process
#             (REALITY opens one TCP+TLS connection per proxied connection)
#   hy2     : N client processes = N independent QUIC connections ("users"),
#             one download each
# Reports aggregate and per-client throughput, Jain's fairness index, failed
# downloads, server CPU %, server RSS and peak open file descriptors, plus a
# ping-pong latency sample taken on a separate connection during the load.
# Temporary files go to mktemp -d (set TMPDIR to keep them elsewhere).
# Requires lab.sh standard_up (server-exit, cR, cH running).
set -uo pipefail
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
SB="${SB:-/usr/local/bin/sing-box}"
transport="$1" n="$2" dur="$3"
spid="$(cat "$RUN/server-exit.pid")"
work="$(mktemp -d)"
client_pids=()
cleanup() {
  local p
  for p in "${client_pids[@]}"; do kill "$p" 2>/dev/null; done
  rm -rf "$work"
}
trap cleanup EXIT

ports=()
case "$transport" in
  reality)
    for _ in $(seq 1 "$n"); do ports+=(1080); done
    ;;
  hy2)
    for i in $(seq 1 "$n"); do
      port=$((1100 + i))
      jq --argjson p "$port" '.inbounds = [{type:"mixed", tag:"socks", listen:"127.0.0.1", listen_port:$p}]' \
        "$RUN/cH.json" >"$work/c$i.json"
      ip netns exec cli taskset -c 0,2 "$SB" run -c "$work/c$i.json" >"$work/c$i.log" 2>&1 &
      client_pids+=("$!")
      ports+=("$port")
    done
    sleep 2
    ;;
  *) echo "usage: $0 reality|hy2 N DURATION" >&2; exit 2 ;;
esac

ticks() { awk '{print $14+$15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
rss0="$(awk '/VmRSS/{print $2}' "/proc/$spid/status")"
s0="$(ticks "$spid")"; t0="$(date +%s.%N)"
curl_pids=()
for i in $(seq 1 "$n"); do
  # env -i: a NO_PROXY covering the destination must not bypass the tunnel
  ip netns exec cli env -i PATH=/usr/bin:/bin curl -s -o /dev/null --max-time "$dur" \
    --noproxy '' --socks5-hostname "127.0.0.1:${ports[$((i - 1))]}" \
    -w '%{size_download} %{http_code}\n' "http://dest.test/bytes/100000000000" >"$work/r$i" 2>/dev/null &
  curl_pids+=("$!")
done
fdmax=0
# Latency is sampled on a connection that carries no download of its own
# (REALITY: a new TCP connection; Hysteria2: the idle standard client cH, its
# own QUIC connection), so both transports measure the shared bottleneck and
# the server, not one QUIC connection's internal queueing (loaded-latency.sh).
lat_port=1080
[ "$transport" = hy2 ] && lat_port=1081
ip netns exec cli "$LAB_WORK/bin/labtool" rtt -socks "127.0.0.1:$lat_port" -target 11.0.3.2:7 \
  -duration "$((dur - 4))s" -interval 100ms -name "$transport" >"$work/lat.json" 2>/dev/null &
latpid=$!
end=$(( $(date +%s) + dur ))
while [ "$(date +%s)" -lt "$end" ]; do
  fds="$(find "/proc/$spid/fd" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)"
  [ "$fds" -gt "$fdmax" ] && fdmax="$fds"
  sleep 1
done
wait "${curl_pids[@]}" 2>/dev/null
wait "$latpid" 2>/dev/null
t1="$(date +%s.%N)"; s1="$(ticks "$spid")"
rss1="$(awk '/VmRSS/{print $2}' "/proc/$spid/status")"
hz="$(getconf CLK_TCK)"
# per-client Mbit/s from bytes received in the fixed window; < 1000 bytes = failed
awk -v d="$dur" '{ printf "%.3f\n", $1 * 8 / d / 1e6 }' "$work"/r* >"$work/mbps"
jq -s -c --arg t "$transport" --argjson n "$n" --argjson cpu "$(( s1 - s0 ))" --argjson hz "$hz" \
  --argjson wall "$(echo "$t1 - $t0" | bc)" --argjson rss0 "$rss0" --argjson rss1 "$rss1" \
  --argjson fdmax "$fdmax" --argjson lat "$(cat "$work/lat.json" 2>/dev/null || echo null)" '
  (sort) as $v | ($v | add) as $sum | ($v | map(. * .) | add) as $sq |
  { transport: $t, clients: $n,
    aggregate_mbps: ($sum * 10 | round / 10),
    per_client_min: ($v[0] * 10 | round / 10),
    per_client_median: ($v[(($v | length) - 1) / 2 | floor] * 10 | round / 10),
    per_client_max: ($v[-1] * 10 | round / 10),
    jain: ((($sum * $sum) / (($v | length) * $sq + 1e-9)) * 1000 | round / 1000),
    failed: ($v | map(select(. < 0.001)) | length),
    server_cpu_pct: ((($cpu / $hz) / $wall * 100) | round),
    server_rss_kb_before: $rss0, server_rss_kb_after: $rss1, server_fd_peak: $fdmax,
    latency_during_load: $lat }' "$work/mbps"
