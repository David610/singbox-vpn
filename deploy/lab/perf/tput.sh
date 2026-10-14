#!/usr/bin/env bash
# tput.sh LABEL PORT DURATION PARALLEL [REVERSE=1]  -> one JSON line
# Runs iperf3 in the cli namespace against PORT (a client forwarder, or
# 5201 on 11.0.3.2 directly), sampling server/client sing-box CPU.
set -euo pipefail
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
label="$1" port="$2" dur="$3" par="$4" rev="${5:-1}" host="${HOST:-127.0.0.1}"
spid="$(cat "$RUN/${SERVER_PID_NAME:-server-exit}.pid" 2>/dev/null || true)"
cpid="$(cat "$RUN/${CLIENT_PID_NAME:-none}.pid" 2>/dev/null || true)"
ticks() { [ -n "$1" ] && [ -r "/proc/$1/stat" ] && awk '{print $14+$15}' "/proc/$1/stat" || echo 0; }
s0=$(ticks "$spid"); c0=$(ticks "$cpid"); t0=$(date +%s.%N)
args=(-c "$host" -p "$port" -t "$dur" -P "$par" -J --connect-timeout 10000)
[ "$rev" = 1 ] && args+=(-R)
out="$(ip netns exec cli taskset -c 0 iperf3 "${args[@]}" 2>&1)" || true
t1=$(date +%s.%N); s1=$(ticks "$spid"); c1=$(ticks "$cpid")
hz=$(getconf CLK_TCK)
echo "$out" | jq -c --arg label "$label" --argjson s "$((s1 - s0))" --argjson c "$((c1 - c0))" \
  --argjson wall "$(echo "$t1 - $t0" | bc)" --argjson hz "$hz" '
  { label: $label,
    mbps: ((.end.sum_received.bits_per_second // 0) / 1e6 | . * 10 | round / 10),
    retrans: (.end.sum_sent.retransmits // null),
    server_cpu_pct: (($s / $hz) / $wall * 100 | round),
    client_cpu_pct: (($c / $hz) / $wall * 100 | round),
    error: (.error // null) }'
