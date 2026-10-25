#!/usr/bin/env bash
# Real-sing-box lab for the nested-VLESS (relay detour) path: client -> relay -> exit -> HTTP/2 target,
# all on loopback, hand-written sing-box configs (NOT production-generated; the production-generated
# two-hop scenarios are crates/compat-config/tests/two_hop_system.rs S18-S20).
#
# IMPORTANT: this lab does NOT reproduce the 2026-10-25 field failure (Chromium HTTP/2 hanging on a
# Vision/Vision relay chain). On loopback every case below passes, including outer=vision inner=vision.
# It only shows that the Vision-less nested shape (first hop keeps Vision, the nested exit credential
# carries no flow) is accepted by the pinned sing-box and carries parallel HTTP/2 streams. The root cause
# of the field failure is unclassified (docs/RELAY_EXIT_VISION.md).
# Usage: nested_vision_lab.sh <sing-box> <go> ; exit 0 = the Vision-less shapes carried the traffic.
set -u
SB="${1:-sing-box}"; GO="${2:-go}"
W="$(mktemp -d)"; trap 'kill $(cat "$W"/*.pid 2>/dev/null) 2>/dev/null; rm -rf "$W"' EXIT
DEC=19500; EXIT_P=19501; RELAY_P=19502; SOCKS=19503
U1=11111111-1111-4111-8111-111111111111; U2=22222222-2222-4222-8222-222222222222
KP="$("$SB" generate reality-keypair)"; PRIV="$(echo "$KP" | awk '/PrivateKey/{print $2}')"; PUB="$(echo "$KP" | awk '/PublicKey/{print $2}')"; SID=0a1b2c3d
( cd "$(dirname "$0")" && "$GO" build -o "$W/h2target" h2target.go ) || { echo "SKIP: go build failed"; exit 3; }
"$W/h2target" $DEC > "$W/dec.log" 2>&1 & echo $! > "$W/dec.pid"
inbound() { # port uuid flow
cat <<J
{"type":"vless","tag":"in","listen":"127.0.0.1","listen_port":$1,"users":[{"name":"u","uuid":"$2","flow":"$3"}],
 "tls":{"enabled":true,"server_name":"decoy.test","reality":{"enabled":true,"handshake":{"server":"127.0.0.1","server_port":$DEC},"private_key":"$PRIV","short_id":["$SID"]}}}
J
}
outbound() { # tag port uuid flow detour
 d=""; [ -n "${5:-}" ] && d=",\"detour\":\"$5\""
cat <<J
{"type":"vless","tag":"$1","server":"127.0.0.1","server_port":$2,"uuid":"$3","flow":"$4"$d,
 "tls":{"enabled":true,"server_name":"decoy.test","utls":{"enabled":true,"fingerprint":"chrome"},"reality":{"enabled":true,"public_key":"$PUB","short_id":"$SID"}}}
J
}
run_case() { # label outer_flow inner_flow  -> prints verdict
  local label="$1" f_out="$2" f_in="$3"
  for p in exit relay client; do kill $(cat "$W/$p.pid" 2>/dev/null) 2>/dev/null; done; sleep 0.3
  # exit: terminates the inner VLESS; relay: forwards ONLY to the exit port (opaque TCP, like production)
  cat > "$W/exit.json" <<J
{"log":{"level":"error"},"inbounds":[$(inbound $EXIT_P $U2 "$f_in")],"outbounds":[{"type":"direct","tag":"direct"}],"route":{"final":"direct"}}
J
  cat > "$W/relay.json" <<J
{"log":{"level":"error"},"inbounds":[$(inbound $RELAY_P $U1 "$f_out")],"outbounds":[{"type":"direct","tag":"direct"}],
 "route":{"rules":[{"ip_cidr":["127.0.0.1/32"],"port":$EXIT_P,"network":"tcp","action":"route","outbound":"direct"},{"action":"reject"}]}}
J
  cat > "$W/client.json" <<J
{"log":{"level":"error"},"inbounds":[{"type":"socks","tag":"s","listen":"127.0.0.1","listen_port":$SOCKS}],
 "outbounds":[$(outbound outer $RELAY_P $U1 "$f_out"),$(outbound inner $EXIT_P $U2 "$f_in" outer)],"route":{"final":"inner"}}
J
  for p in exit relay client; do "$SB" check -c "$W/$p.json" || { echo "CONFIG_INVALID $p"; return 9; }; "$SB" run -c "$W/$p.json" > "$W/$p.log" 2>&1 & echo $! > "$W/$p.pid"; done
  for i in $(seq 1 40); do sleep 0.25; (echo > /dev/tcp/127.0.0.1/$SOCKS) 2>/dev/null && break; done
  ok=0; bad=0
  for round in 1 2 3; do
    out="$(curl -sk --http2 --parallel --parallel-max 16 -m 25 --socks5-hostname 127.0.0.1:$SOCKS -w '%{http_code} %{size_download} %{http_version}\n' -o /dev/null "https://127.0.0.1:$DEC/blob?n=3000000&r=[1-16]" 2>"$W/curl.err")"
    n_ok="$(echo "$out" | grep -c '^200 3000000 2')"; ok=$((ok + n_ok)); bad=$((bad + 16 - n_ok))
  done
  [ "$bad" -gt 0 ] && { echo "  curl: $(sort -u "$W/curl.err" | head -2 | tr "
" " ")"; echo "  client log: $(head -c 300 "$W/client.log" | tr "
" " ")"; }
  cr=""
  if [ -n "${PW_PY:-}" ]; then
    cr="$("$PW_PY" "$(dirname "$0")/chromium_h2_fetch.py" $SOCKS $DEC 16 3000000 3 2>/dev/null | tail -1)"
    case "$cr" in *" bad=0"*) ;; *) bad=$((bad + 1)) ;; esac
  fi
  echo "$label: curl ok=$ok/48; chromium ${cr:-not run}; client_log_corruption=$(grep -ci 'received record' "$W/client.log")"
  [ "$bad" -eq 0 ]
}
run_case "outer=vision inner=vision (shape that failed in the field; passes here)" xtls-rprx-vision xtls-rprx-vision; r_vv=$?
run_case "outer=vision inner=none   (Vision-less nested exit: the supported option)" xtls-rprx-vision "" ; r_vn=$?
run_case "outer=none   inner=none" "" ""; r_nn=$?
echo "RESULT vision/vision=$([ $r_vv = 0 ] && echo pass || echo FAIL) vision/none=$([ $r_vn = 0 ] && echo pass || echo FAIL) none/none=$([ $r_nn = 0 ] && echo pass || echo FAIL)"
[ $r_vn = 0 ] && [ $r_nn = 0 ]
