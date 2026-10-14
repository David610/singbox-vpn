#!/usr/bin/env bash
# egress-probe.sh — C-16 / relay fail-closed probes through the real tunnels
# (lab only; requires lab.sh standard_up). Prints one JSON line per probe.
#
# Every request runs under `env -i` with `--noproxy ''`: curl applies
# NO_PROXY to SOCKS proxies too, and a NO_PROXY listing private ranges would
# otherwise send "tunnel" requests for literal private addresses direct.
set -uo pipefail
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"; RUN="$LAB_WORK/run"
LT="$LAB_WORK/bin/labtool"
SB="${SB:-/usr/local/bin/sing-box}"
pids=()
cleanup() {
  local p
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done
  ip -n exit addr del 169.254.169.254/32 dev lo 2>/dev/null
  ip -n exit addr del 10.20.30.40/32 dev lo 2>/dev/null
}
trap cleanup EXIT

# Targets that must be refused, all served by real listeners on the exit host
# (control column proves they are reachable without the tunnel).
ip -n exit addr add 169.254.169.254/32 dev lo 2>/dev/null
ip -n exit addr add 10.20.30.40/32 dev lo 2>/dev/null
ip netns exec exit "$LT" bulk -listen 0.0.0.0:8081 >/dev/null 2>&1 & pids+=("$!")
ip netns exec exit "$LT" bulk -listen 0.0.0.0:80 >/dev/null 2>&1 & pids+=("$!")
ip netns exec inet "$LT" echo -listen 11.0.3.2:25 >/dev/null 2>&1 & pids+=("$!")
# A client whose only route is the relay's own first hop (no exit behind it).
jq '.route.final = "Reality"' "$RUN/c2.json" \
  | jq '.inbounds = [{type:"mixed", tag:"socks", listen:"127.0.0.1", listen_port:1089}]' >"$RUN/c-relay-only.json"
ip netns exec cli "$SB" run -c "$RUN/c-relay-only.json" >"$RUN/c-relay-only.log" 2>&1 & pids+=("$!")
sleep 1.5

http_code() { # http_code NS SOCKS_PORT|- URL
  local ns="$1" socks="$2" url="$3" args=(-s -o /dev/null -w '%{http_code}' --max-time 4 --noproxy '')
  [ "$socks" != - ] && args+=(--socks5-hostname "127.0.0.1:$socks")
  ip netns exec "$ns" env -i PATH=/usr/bin:/bin curl "${args[@]}" "$url" 2>/dev/null || true
}
tcp_echo() { # tcp_echo SOCKS TARGET -> ok|refused
  ip netns exec cli "$LT" connlat -socks "127.0.0.1:$1" -target "$2" -n 1 -timeout 4s \
    | jq -r 'if .failures == 0 then "ok" else "refused" end'
}

probe() { # probe NAME URL
  local name="$1" url="$2" control r h
  control="$(http_code exit - "$url")"
  r="$(http_code cli 1080 "$url")"
  h="$(http_code cli 1081 "$url")"
  jq -cn --arg t "$name" --arg u "$url" --arg c "$control" --arg r "$r" --arg h "$h" \
    '{target:$t, url:$u, control_from_exit:$c, via_reality:$r, via_hysteria2:$h}'
}

probe "loopback literal" "http://127.0.0.1:8081/ok"
probe "own public IP literal" "http://11.0.2.2:8081/ok"
probe "domain -> 127.0.0.1 (rebinding)" "http://rebind.test:8081/ok"
probe "domain -> own public IP" "http://self.test:8081/ok"
probe "metadata literal 169.254.169.254" "http://169.254.169.254/ok"
probe "domain -> metadata" "http://meta.test/ok"
probe "RFC1918 literal 10.20.30.40" "http://10.20.30.40:8081/ok"
probe "ordinary destination (positive control)" "http://dest.test/ok"
jq -cn --arg r "$(tcp_echo 1080 11.0.3.2:25)" --arg h "$(tcp_echo 1081 11.0.3.2:25)" \
  --arg c "$(ip netns exec exit bash -c 'exec 3<>/dev/tcp/11.0.3.2/25 && echo ok' 2>/dev/null || echo refused)" \
  '{target:"TCP/25 (SMTP)", control_from_exit:$c, via_reality:$r, via_hysteria2:$h}'
jq -cn --arg relay "$(http_code cli 1089 "http://dest.test/ok")" --arg via "$(http_code cli 1082 "http://dest.test/ok")" \
  '{target:"relay first hop as an exit (must fail closed)", relay_only_route:$relay, relay_to_exit_route:$via}'
