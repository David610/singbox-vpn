#!/usr/bin/env bash
# soak.sh DURATION_S [OUT_DIR] — lab soak of the exit under mixed load (class C)
#
# For DURATION_S seconds, per transport (REALITY via cR, Hysteria2 via cH):
#   - 2 long-lived proxied connections doing a 64-byte ping-pong every second
#     (do they survive, does latency drift?)
#   - connection churn: batches of 20 new proxied connections every 10 s
#   - a 10 s bulk download every 60 s
# and samples the exit sing-box every 10 s: RSS, open fds, threads, CPU ticks.
# Writes samples.csv and a summary JSON line to OUT_DIR. Requires
# lab.sh standard_up. Not a substitute for the real-host soak
# (deploy/lib/soak-sampler.sh + soak_evaluate.py).
set -uo pipefail
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
LT="$LAB_WORK/bin/labtool"
dur="$1"; out="${2:-$LAB_WORK/soak-$(date +%s)}"
mkdir -p "$out"
spid="$(cat "$RUN/server-exit.pid")"
pids=()
cleanup() { local p; for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; }
trap cleanup EXIT

for socks in 1080 1081; do
  for k in 1 2; do
    ip netns exec cli "$LT" rtt -socks "127.0.0.1:$socks" -target 11.0.3.2:7 \
      -duration "${dur}s" -interval 1s -name "long-$socks-$k" >"$out/long-$socks-$k.json" 2>&1 &
    pids+=("$!")
  done
done

churn() { # churn SOCKS
  local end=$(( $(date +%s) + dur ))
  while [ "$(date +%s)" -lt "$end" ]; do
    ip netns exec cli "$LT" connlat -socks "127.0.0.1:$1" -target 11.0.3.2:7 -n 20 -gap 50ms -name "churn-$1" >>"$out/churn-$1.jsonl" 2>/dev/null
    sleep 10
  done
}
bulk() { # bulk SOCKS
  local end=$(( $(date +%s) + dur ))
  while [ "$(date +%s)" -lt "$end" ]; do
    ip netns exec cli env -i PATH=/usr/bin:/bin curl -s -o /dev/null --max-time 10 --noproxy '' \
      --socks5-hostname "127.0.0.1:$1" -w '%{size_download}\n' "http://dest.test/bytes/100000000000" >>"$out/bulk-$1.txt" 2>/dev/null
    sleep 50
  done
}
for socks in 1080 1081; do
  churn "$socks" & pids+=("$!")
  bulk "$socks" & pids+=("$!")
done

echo "epoch,rss_kb,fds,threads,cpu_ticks" >"$out/samples.csv"
end=$(( $(date +%s) + dur ))
while [ "$(date +%s)" -lt "$end" ]; do
  if [ -r "/proc/$spid/status" ]; then
    printf '%s,%s,%s,%s,%s\n' "$(date +%s)" \
      "$(awk '/^VmRSS:/{print $2}' "/proc/$spid/status")" \
      "$(find "/proc/$spid/fd" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)" \
      "$(awk '/^Threads:/{print $2}' "/proc/$spid/status")" \
      "$(awk '{print $14+$15}' "/proc/$spid/stat")" >>"$out/samples.csv"
  else
    echo "$(date +%s),dead,,," >>"$out/samples.csv"
  fi
  sleep 10
done
sleep 5
cleanup
pids=()

jq -s -c '{long_lived: map({name, n, failures, p50_ms, p99_ms, max_ms})}' "$out"/long-*.json >"$out/long.json" 2>/dev/null
{
  echo "server_alive=$( [ -r "/proc/$spid/status" ] && echo yes || echo no )"
  awk -F, 'NR==2{r0=$2;f0=$3} NR>1 && $2!="dead"{r=$2;f=$3; if($2>rm)rm=$2; if($3>fm)fm=$3} END{printf "rss_kb_start=%s rss_kb_end=%s rss_kb_max=%s fds_start=%s fds_end=%s fds_max=%s\n",r0,r,rm,f0,f,fm}' "$out/samples.csv"
  for s in 1080 1081; do
    jq -s -r --arg s "$s" '"churn \($s): batches=\(length) failures=\(map(.failures)|add) p50_of_p50=\(map(.p50_ms)|sort|.[length/2|floor])"' "$out/churn-$s.jsonl" 2>/dev/null
    awk -v s="$s" '{n++; if ($1 < 1000) f++} END{printf "bulk %s: runs=%d failed=%d\n", s, n, f}' "$out/bulk-$s.txt" 2>/dev/null
  done
  cat "$out/long.json"
} | tee "$out/summary.txt"
