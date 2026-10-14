#!/usr/bin/env bash
# classb.sh LABEL SING_BOX_BINARY [REPS] -> JSON lines (class B: no added delay)
#
# Same-host protocol overhead with the exit pinned to ONE CPU (a 1-vCPU proxy)
# and clients on another. For the given sing-box binary:
#   - REALITY and Hysteria2 (Salamander, as deployed), download and upload
#   - Hysteria2 WITHOUT Salamander (attribution only; never a recommendation)
#   - REALITY carrying TLS (HTTPS download, Vision can skip outer encryption)
#     vs plain HTTP (every byte double-encrypted)
#   - the exit pinned to TWO CPUs ("would a second vCPU help?")
# Records Mbit/s, server/client CPU %, the exit's Hysteria2 UDP socket buffer
# sizes and the exit namespace's UDP RcvbufErrors delta. netem must be off.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"; STATE="$LAB_WORK/state"
LT="$LAB_WORK/bin/labtool"
label="$1"; sb="$2"; reps="${3:-3}"
export SB="$sb"
pids=()
cleanup() { local p; for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; "$HERE/lab.sh" stop server-exit; "$HERE/lab.sh" stop cR; "$HERE/lab.sh" stop cH; }
trap cleanup EXIT

rcvbuf_errors() { ip netns exec exit awk '/^Udp:/{n++; if(n==1){for(i=2;i<=NF;i++) if($i=="RcvbufErrors") c=i} else {print $c; exit}}' /proc/net/snmp; }
hy2_skmem() { ip netns exec exit ss -Huamn 'sport = :443' 2>/dev/null | grep -o 'rb[0-9]*,t[0-9]*,tb[0-9]*' | head -1; }

restart() { # restart SERVER_CONFIG CPUS CLIENT_SUB
  "$HERE/lab.sh" stop server-exit; "$HERE/lab.sh" stop cR; "$HERE/lab.sh" stop cH; sleep 0.5
  ip netns exec exit taskset -c "$2" "$sb" run -c "$1" >"$RUN/server-exit.log" 2>&1 &
  echo $! >"$RUN/server-exit.pid"; sleep 1
  "$HERE/lab.sh" client_up cR Reality 1080 15201 4 2 "$3" >/dev/null
  "$HERE/lab.sh" client_up cH Hysteria2 1081 15301 4 2 "$3" >/dev/null
  ip netns exec cli env -i PATH=/usr/bin:/bin curl -s -o /dev/null --noproxy '' --socks5-hostname 127.0.0.1:1081 http://dest.test/ok
}

run() { # run TEST TRANSPORT PORT CLIENT REVERSE EXTRA_JSON
  local e0 i
  for i in $(seq 1 "$reps"); do
    e0="$(rcvbuf_errors)"
    CLIENT_PID_NAME="$4" "$HERE/tput.sh" "$2" "$3" 8 1 "$5" \
      | jq -c --arg l "$label" --arg t "$1" --argjson rep "$i" --argjson extra "$6" \
          --argjson rcv "$(( $(rcvbuf_errors) - e0 ))" --arg sk "$(hy2_skmem)" \
          '. + {singbox:$l, test:$t, rep:$rep, exit_udp_rcvbuf_errors_delta:$rcv, exit_hy2_skmem:$sk} + $extra'
  done
}

cfg="$STATE/exit/sing-box/config.json"
sub="$STATE/exit/client-sub.json"
jq 'del(.inbounds[] | select(.type=="hysteria2") | .obfs)' "$cfg" >"$RUN/exit-noobfs.json"
jq '(.outbounds[] | select(.type=="hysteria2")) |= del(.obfs)' "$sub" >"$RUN/client-sub-noobfs.json"

restart "$cfg" 1 "$sub"
run download reality 15201 cR 1 '{"cpus":1,"obfs":true}'
run upload reality 15201 cR 0 '{"cpus":1,"obfs":true}'
run download hy2 15301 cH 1 '{"cpus":1,"obfs":true}'
run upload hy2 15301 cH 0 '{"cpus":1,"obfs":true}'

# REALITY carrying TLS vs plain HTTP: curl through the SOCKS port, fixed size.
ip netns exec inet "$LT" bulk -listen 11.0.3.2:8443 -tls-name dest.test >/dev/null 2>&1 & pids+=("$!")
sleep 0.5
for scheme in http https; do
  for i in $(seq 1 "$reps"); do
    url="http://dest.test/bytes/2000000000"; [ "$scheme" = https ] && url="https://dest.test:8443/bytes/2000000000"
    s0="$(awk '{print $14+$15}' "/proc/$(cat "$RUN/server-exit.pid")/stat")"; t0="$(date +%s.%N)"
    mbps="$(ip netns exec cli env -i PATH=/usr/bin:/bin curl -k -s -o /dev/null --max-time 20 --noproxy '' \
      --socks5-hostname 127.0.0.1:1080 -w '%{speed_download}' "$url" | awk '{printf "%.1f", $1*8/1e6}')"
    t1="$(date +%s.%N)"; s1="$(awk '{print $14+$15}' "/proc/$(cat "$RUN/server-exit.pid")/stat")"
    jq -cn --arg l "$label" --arg s "$scheme" --argjson m "$mbps" --argjson rep "$i" \
      --argjson cpu "$(echo "($s1 - $s0) / $(getconf CLK_TCK) / ($t1 - $t0) * 100" | bc -l)" \
      '{singbox:$l, test:"reality-payload", payload:$s, rep:$rep, mbps:$m, server_cpu_pct:($cpu|round), cpus:1}'
  done
done

restart "$RUN/exit-noobfs.json" 1 "$RUN/client-sub-noobfs.json"
run download hy2-noobfs 15301 cH 1 '{"cpus":1,"obfs":false}'
run upload hy2-noobfs 15301 cH 0 '{"cpus":1,"obfs":false}'

restart "$cfg" 1,3 "$sub"
run download reality 15201 cR 1 '{"cpus":2,"obfs":true}'
run download hy2 15301 cH 1 '{"cpus":2,"obfs":true}'
