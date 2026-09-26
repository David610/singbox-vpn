#!/usr/bin/env bash
# Phase 4 protocol-health real test: stand up an isolated singbox-vpn server
# (state under /root/ph, port 2443 tcp+udp, unit ph-singbox) plus the agent
# under test (unit ph-agent), without touching any other sing-box/agent on
# the host. Usage: setup-node.sh <node_id> <public_ipv4>
# Expects /root/ph/bin/{vpn-admin,vpn-provisioning-agent} (static musl builds).
set -euo pipefail
NODE_ID="$1"; IP="$2"; PORT=2443; D=/root/ph
mkdir -p "$D/compat/hysteria" && chmod 700 "$D"
cat > "$D/deployment.toml" <<TOML
schema_version = 2
node_id = "$NODE_ID"
role = "exit"
public_host = "$IP"
subscription_host = "$IP"
state_dir = "$D/compat"
[reality]
listen_port = $PORT
handshake_server = "www.cloudflare.com"
handshake_port = 443
[hysteria2]
listen_port = $PORT
[subscription]
listen_port = 19100
public_port = 19100
TOML
if [ ! -s "$D/compat/hysteria/cert.pem" ]; then
  # Self-signed, 30 days: the agent reports its expiry as a dimension; the
  # probe client runs with tls_insecure_for_tests=true because of it.
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 30 \
    -subj "/CN=$NODE_ID.test.invalid" -keyout "$D/compat/hysteria/key.pem" -out "$D/compat/hysteria/cert.pem" 2>/dev/null
  chmod 600 "$D/compat/hysteria/key.pem"
fi
# vpn-admin manages the "sing-box" unit; map it to ph-singbox so the host's
# other sing-box (if any) is never touched.
cat > "$D/systemctl" <<'SH'
#!/bin/sh
args=""
for a in "$@"; do case "$a" in sing-box|sing-box.service) a=ph-singbox.service;; esac; args="$args $a"; done
exec /usr/bin/systemctl $args
SH
chmod 755 "$D/systemctl"
SB=$(command -v sing-box)
cat > /etc/systemd/system/ph-singbox.service <<UNIT
[Unit]
Description=sing-box (protocol-health test, port $PORT)
[Service]
ExecStartPre=$SB check -c $D/compat/sing-box/config.json
ExecStart=$SB run -c $D/compat/sing-box/config.json
Restart=on-failure
UNIT
systemctl daemon-reload
export SINGBOX_VPN_SYSTEMCTL="$D/systemctl"
if [ ! -s "$D/compat/reality/private.key" ]; then
  SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 "$D/bin/vpn-admin" --config "$D/deployment.toml" init >"$D/init.log" 2>&1 || { tail -3 "$D/init.log"; exit 1; }
fi
SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 "$D/bin/vpn-admin" --config "$D/deployment.toml" render-config >"$D/render.log" 2>&1 || { tail -3 "$D/render.log"; exit 1; }
systemctl restart ph-singbox; sleep 1; systemctl is-active ph-singbox
cat > "$D/agent.toml" <<T
worker_url = "http://127.0.0.1:18788"
node_id = "$NODE_ID"
agent_api_key = "$NODE_ID"
vpn_admin_binary = "$D/bin/vpn-admin"
vpn_admin_config = "$D/deployment.toml"
poll_interval_secs = 5
heartbeat_interval_secs = 20
lease_pool_size = 0
lease_state_file = "$D/lease-pool.json"
[protocol_probe]
singbox_binary = "$SB"
interval_secs = 20
timeout_secs = 8
hysteria2_cert_path = "$D/compat/hysteria/cert.pem"
tls_insecure_for_tests = true
T
chmod 600 "$D/agent.toml"
systemctl stop ph-agent 2>/dev/null || true
systemd-run --unit=ph-agent --collect -E RUST_LOG=info -E SINGBOX_VPN_SYSTEMCTL="$D/systemctl" \
  -E SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=0 "$D/bin/vpn-provisioning-agent" --config "$D/agent.toml"
echo "setup ok: $NODE_ID $IP:$PORT"
