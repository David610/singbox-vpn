#!/usr/bin/env bash
# outage.sh SECONDS [ROUNDS] -> JSON lines (class C, lab only)
#
# Drops every packet between the client and exit networks for SECONDS (a
# phone in a tunnel or lift, a Wi-Fi/LTE gap that keeps the address), then
# reports per transport whether an already-open proxied connection survived
# (a 1 s ping-pong with a 120 s per-ping deadline) and how long after the
# outage a NEW request first succeeded. Requires lab.sh standard_up.
set -uo pipefail
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
LT="$LAB_WORK/bin/labtool"
secs="$1"; rounds="${2:-2}"

block() {
  ip netns exec rtr nft -f - <<NFT
table inet laboutage {
  chain forward_filter {
    type filter hook forward priority -10; policy accept;
    ip saddr 11.0.1.0/24 ip daddr 11.0.2.0/24 drop
    ip saddr 11.0.2.0/24 ip daddr 11.0.1.0/24 drop
  }
}
NFT
}
unblock() { ip netns exec rtr nft delete table inet laboutage 2>/dev/null || true; }
trap unblock EXIT

try() {
  [ "$(ip netns exec cli env -i PATH=/usr/bin:/bin curl -s -o /dev/null -w '%{http_code}' --max-time 3 \
        --noproxy '' --socks5-hostname "127.0.0.1:$1" http://dest.test/ok 2>/dev/null)" = 200 ]
}

for r in $(seq 1 "$rounds"); do
  for socks in 1080 1081; do
    ip netns exec cli "$LT" rtt -socks "127.0.0.1:$socks" -target 11.0.3.2:7 \
      -duration "$((secs + 30))s" -interval 1s -timeout 120s -name "long-$socks" >"$RUN/outage-long-$socks.json" 2>/dev/null &
  done
  sleep 3
  block; sleep "$secs"; unblock
  t0="$(date +%s.%N)"
  for socks in 1080 1081; do
    (
      n=0
      until try "$socks"; do n=$((n + 1)); [ "$n" -ge 120 ] && break; sleep 0.5; done
      t1="$(date +%s.%N)"
      echo "$socks $(echo "$t1 - $t0" | bc)" >"$RUN/outage-new-$socks.txt"
    ) &
  done
  wait
  for socks in 1080 1081; do
    jq -c --arg t "$([ "$socks" = 1080 ] && echo reality || echo hy2)" --argjson s "$secs" --argjson r "$r" \
      --arg new "$(cut -d' ' -f2 "$RUN/outage-new-$socks.txt")" \
      '{transport:$t, outage_s:$s, round:$r,
        open_connection_survived:(.failures == 0),
        open_connection_samples:.n, open_connection_max_ms:(.max_ms // null | if . == null then null else round end),
        first_new_request_success_s_after_outage:($new|tonumber*100|round/100)}' "$RUN/outage-long-$socks.json"
  done
  sleep 2
done
