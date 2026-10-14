#!/usr/bin/env bash
# restart-recovery.sh [ROUNDS] -> JSON lines (class B/C, lab only)
#
# What a user sees when the exit's sing-box restarts (every user change,
# expiry reconcile or update does this): restart the server, then from the
# already-running clients open a NEW proxied request every 250 ms per
# transport and record how long until requests succeed again and how many
# failed. Clients are NOT restarted: a phone keeps its tunnel up too.
# Requires lab.sh standard_up. Environment knobs:
#   PRE_IDLE=SECONDS  wait this long between the warm-up request and the
#                     restart, so no tail packet of it reaches the stopped
#                     server
#   IDLE_GAP=SECONDS  send nothing for this long after the restart before the
#                     first probe. With both set, the client is a phone that
#                     was idle while the server was down: none of its packets
#                     could draw an ICMP port-unreachable.
#   SERVER_CFG=FILE   restart the exit with this config instead of the
#                     rendered one (attribution runs only, e.g. no Salamander;
#                     start a matching client first)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"
rounds="${1:-5}"
idle_gap="${IDLE_GAP:-0}"
pre_idle="${PRE_IDLE:-0}"
server_cfg="${SERVER_CFG:-}"

try() { # try SOCKS -> 0 on success
  [ "$(ip netns exec cli env -i PATH=/usr/bin:/bin curl -s -o /dev/null -w '%{http_code}' --max-time 3 \
        --noproxy '' --socks5-hostname "127.0.0.1:$1" http://dest.test/ok 2>/dev/null)" = 200 ]
}

for r in $(seq 1 "$rounds"); do
  for socks in 1080 1081; do try "$socks" || true; done   # both tunnels warm
  sleep "$pre_idle"
  "$HERE/lab.sh" stop server-exit
  t0="$(date +%s.%N)"
  if [ -n "$server_cfg" ]; then
    "$HERE/lab.sh" bg server-exit exit 1 "${SB:-/usr/local/bin/sing-box}" run -c "$server_cfg"
    sleep 1
  else
    "$HERE/lab.sh" server_up exit exit 1
  fi
  sleep "$idle_gap"
  for socks in 1080 1081; do
    (
      fails=0; first=""
      while :; do
        if try "$socks"; then first="$(date +%s.%N)"; break; fi
        fails=$((fails + 1))
        [ "$fails" -ge 120 ] && break
        sleep 0.25
      done
      jq -cn --arg t "$( [ "$socks" = 1080 ] && echo reality || echo hy2 )" --argjson r "$r" --argjson f "$fails" \
        --arg rec "$( [ -n "$first" ] && echo "$first - $t0" | bc || echo null )" \
        --argjson gap "$idle_gap" --argjson pre "$pre_idle" --arg cfg "$( [ -n "$server_cfg" ] && basename "$server_cfg" || echo rendered )" \
        '{transport:$t, round:$r, pre_idle_s:$pre, idle_gap_s:$gap, server_config:$cfg, failed_requests:$f,
          seconds_from_restart_to_first_success:(if $rec=="null" then null else ($rec|tonumber*100|round/100) end)}'
    ) &
  done
  wait
  sleep 3
done
