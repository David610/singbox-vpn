#!/usr/bin/env bash
# Performance-lab process control: build the lab tools, render node configs
# with the PRODUCTION renderer (vpn-admin, as CI's singbox-validate job does),
# start the "internet" services, the servers under test and the clients.
# State lives in $LAB_WORK (default /tmp/singbox-vpn-perf-lab), never in the
# repository. DISPOSABLE MACHINES ONLY.
set -euo pipefail
# Refuse to run on a VPN node: the installer persists this tree to
# /opt/singbox-vpn, so these scripts can exist on a production host.
if [ -e /etc/vpn/deployment.toml ] || systemctl is-active --quiet sing-box 2>/dev/null; then
  if [ "${LAB_I_KNOW_THIS_IS_NOT_A_VPN_SERVER:-}" != 1 ]; then
    echo "refusing: this host looks like a singbox-vpn node (/etc/vpn/deployment.toml or an active sing-box.service). The perf lab is for disposable machines only." >&2
    exit 1
  fi
fi
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="${REPO:-$(cd "$HERE/../../.." && pwd)}"
LAB_WORK="${LAB_WORK:-${TMPDIR:-/tmp}/singbox-vpn-perf-lab}"
RUN="$LAB_WORK/run"; STATE="$LAB_WORK/state"; BIN="$LAB_WORK/bin"
mkdir -p "$RUN" "$STATE" "$BIN"
ADMIN="${ADMIN:-$REPO/target/debug/vpn-admin}"
SUBSVC="${SUBSVC:-$REPO/target/debug/subscription}"
SB="${SB:-/usr/local/bin/sing-box}"
LABTOOL="$BIN/labtool"

build() {
  (cd "$HERE/netem-tun" && go build -o "$BIN/netem-tun" .)
  (cd "$HERE/labtool" && go build -o "$BIN/labtool" .)
}

bg() { # bg NAME NS CPUS CMD...
  local name="$1" ns="$2" cpus="$3"; shift 3
  ip netns exec "$ns" taskset -c "$cpus" "$@" >"$RUN/$name.log" 2>&1 &
  echo $! >"$RUN/$name.pid"
}
stop() { local f; for f in "$RUN"/${1:-*}.pid; do [ -f "$f" ] || continue; kill "$(cat "$f")" 2>/dev/null || true; rm -f "$f"; done; }

# render_node NAME HOST IP ROLE [extra-toml-file]
render_node() {
  local name="$1" host="$2" ip="$3" role="${4:-exit}" extra="${5:-}" st="$STATE/$1"
  rm -rf "$st"; mkdir -p "$st/hysteria"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -days 7 -nodes \
    -keyout "$st/hysteria/key.pem" -out "$st/hysteria/cert.pem" \
    -subj "/CN=$host" -addext "subjectAltName=DNS:$host" >/dev/null 2>&1
  cat >"$st/deployment.toml" <<TOML
public_host = "$host"
subscription_host = "$host"
state_dir = "$st"
singbox_binary = "$SB"
public_ipv4 = "$ip"
role = "$role"
node_id = "$name"

[reality]
listen_port = 443
handshake_server = "decoy.test"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
TOML
  [ -n "$extra" ] && cat "$extra" >>"$st/deployment.toml"
  SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 "$ADMIN" --config "$st/deployment.toml" init >/dev/null
  SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 "$ADMIN" --config "$st/deployment.toml" user create-probe --json >/dev/null 2>&1
}

# fetch_sub NODE NS USER_JSON OUT: client document via the real subscription service
fetch_sub() {
  local node="$1" ns="$2" user_json="$3" out="$4" token pid
  token="$(jq -r .subscription_url "$user_json" | sed -E 's#.*/sub/([^?]+).*#\1#')"
  ip netns exec "$ns" "$SUBSVC" --config "$STATE/$node/deployment.toml" >"$RUN/sub-$node.log" 2>&1 &
  pid=$!
  sleep 1
  ip netns exec "$ns" curl -fsS "http://127.0.0.1:9100/sub/$token?format=singbox" -o "$out"
  kill "$pid" 2>/dev/null || true
}

# render: exit with one user; relay paired to it (two-hop route).
render() {
  export SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1
  render_node exit exit.lab.test 11.0.2.2 exit
  "$ADMIN" --config "$STATE/exit/deployment.toml" user create --name lab-user --json >"$STATE/exit/user.json" 2>/dev/null
  "$ADMIN" --config "$STATE/exit/deployment.toml" render-config >/dev/null 2>&1 || true
  fetch_sub exit exit "$STATE/exit/user.json" "$STATE/exit/client-sub.json"
  local pub sid extra="$RUN/relay-extra.toml"
  pub="$(cat "$STATE/exit/reality/public.key")"; sid="$(cat "$STATE/exit/reality/short_id.txt")"
  cat >"$extra" <<TOML

[[access_paths]]
id = "relay-ingress"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]

[[peer_endpoints]]
id = "exit-direct"
tag = "Exit Direct"
host = "exit.lab.test"
port = 443
transport = "vless_reality"
server_name = "decoy.test"
reality_public_key = "$pub"
reality_short_id = "$sid"
failure_domain = "exit:lab"

[[peer_endpoints]]
id = "exit-via-relay"
tag = "Exit via Relay"
host = "exit.lab.test"
port = 443
transport = "vless_reality"
server_name = "decoy.test"
reality_public_key = "$pub"
reality_short_id = "$sid"
failure_domain = "exit:lab"
path = "relay-ingress"
credential_ref = "exit-direct"
TOML
  render_node relay relay.lab.test 11.0.4.2 relay "$extra"
  "$ADMIN" --config "$STATE/relay/deployment.toml" user create --name relay-user --json >"$STATE/relay/user.json" 2>/dev/null
  local rid
  rid="$(jq -r .id "$STATE/relay/user.json")"
  # The exit user's VLESS UUID moves between the two state dirs on stdin only.
  jq -r '.. | objects | select(.name?=="lab-user") | .vless_uuid' "$STATE"/exit/users/*.json | head -1 \
    | "$ADMIN" --config "$STATE/relay/deployment.toml" user peer set "$rid" exit-direct --credential-stdin >/dev/null
  "$ADMIN" --config "$STATE/relay/deployment.toml" render-config >/dev/null 2>&1 || true
  fetch_sub relay relay "$STATE/relay/user.json" "$STATE/relay/client-sub.json"
}

inet_up() {
  bg dns inet 0 "$LABTOOL" dns -listen 11.0.3.2:53 \
    -map "decoy.test=11.0.3.2,dest.test=11.0.3.2,exit.lab.test=11.0.2.2,relay.lab.test=11.0.4.2,pair.lab.test=11.0.5.2,rebind.test=127.0.0.1,meta.test=169.254.169.254,self.test=11.0.2.2" \
    -delay "${DNS_DELAY:-0}"
  bg decoy inet 0 "$LABTOOL" decoy -listen 11.0.3.2:443 -name decoy.test
  bg echo inet 0 "$LABTOOL" echo -listen 11.0.3.2:7
  bg bulk inet 0 "$LABTOOL" bulk -listen 11.0.3.2:80
  local i
  for i in $(seq 0 "${IPERF_SERVERS:-7}"); do
    bg "iperf$i" inet 0 iperf3 -s -p $((5201 + i)) -B 11.0.3.2
  done
  sleep 0.5
}

# server_up NODE NS CPUS [config-override.json merged with jq]
server_up() {
  local node="$1" ns="$2" cpus="$3" cfg="$STATE/$1/sing-box/config.json"
  if [ -n "${4:-}" ]; then
    jq -s '.[0] * .[1]' "$cfg" "$4" >"$RUN/$node-server.json"
    cfg="$RUN/$node-server.json"
  fi
  bg "server-$node" "$ns" "$cpus" "$SB" run -c "$cfg"
  sleep 1
}

# server_up_caps NODE NS CPUS UP_MBPS DOWN_MBPS: the rendered exit plus the
# Hysteria2 bandwidth caps exactly as compat-config renders them: caps only,
# and the no-caps default's ignore_client_bandwidth removed (with caps it
# would refuse every client that declares no bandwidth).
server_up_caps() {
  local node="$1" ns="$2" cpus="$3" up="$4" down="$5"
  jq --argjson up "$up" --argjson down "$down" \
    '(.inbounds[] | select(.type=="hysteria2")) |= (del(.ignore_client_bandwidth) | .up_mbps = $up | .down_mbps = $down)' \
    "$STATE/$node/sing-box/config.json" >"$RUN/$node-server-caps.json"
  bg "server-$node" "$ns" "$cpus" "$SB" run -c "$RUN/$node-server-caps.json"
  sleep 1
}

# client_config TAG SOCKS_PORT FWD_BASE_PORT N_FWD [sub-json]
client_config() {
  local tag="$1" socks="$2" fwd="$3" nfwd="$4" sub="${5:-$STATE/exit/client-sub.json}"
  local cert="$STATE/exit/hysteria/cert.pem"
  jq --arg tag "$tag" --argjson socks "$socks" --argjson fwd "$fwd" --argjson nfwd "$nfwd" --arg cert "$cert" '
    def fwds: [range(0; $nfwd) | {type:"direct", tag:("fwd\(.)"), listen:"127.0.0.1",
       listen_port: ($fwd + .), network:"tcp", override_address:"11.0.3.2", override_port:(5201 + .)}];
    { log:{level:"warn"},
      inbounds: ([{type:"mixed", tag:"socks", listen:"127.0.0.1", listen_port:$socks}] + fwds),
      outbounds: [ .outbounds[] | select(.type=="vless" or .type=="hysteria2")
                   | if .type=="hysteria2" then .tls.certificate_path=$cert else . end ],
      route: {final:$tag} }' "$sub"
}

client_up() { # client_up NAME TAG SOCKS FWD NFWD CPUS [sub]
  local name="$1"
  client_config "$2" "$3" "$4" "$5" "${7:-}" >"$RUN/$name.json"
  "$SB" check -c "$RUN/$name.json"
  bg "$name" cli "$6" "$SB" run -c "$RUN/$name.json"
  sleep 1
}

# Standard topology used for docs/reviews/2026-10-14-vpn-audit/: server on
# CPU 1 (a "1 vCPU" proxy), clients on CPU 2, relay on CPU 3, load on CPU 0.
standard_up() {
  inet_up
  server_up exit exit 1
  client_up cR Reality 1080 15201 4 2
  client_up cH Hysteria2 1081 15301 4 2
  server_up relay relay 3
  client_up c2 "Exit via Relay" 1082 15401 4 2 "$STATE/relay/client-sub.json"
}

"$@"
