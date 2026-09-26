set -euo pipefail
source ~/sb/deploy/lib/versions.env
cd /tmp
if ! /usr/local/bin/sing-box version 2>/dev/null | grep -q "$SINGBOX_VERSION"; then
  curl -fsSL -o sb.tgz "https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/sing-box-${SINGBOX_VERSION}-linux-amd64.tar.gz"
  echo "${SINGBOX_SHA256_AMD64}  sb.tgz" | sha256sum -c -
  tar xzf sb.tgz && install -m 0755 "sing-box-${SINGBOX_VERSION}-linux-amd64/sing-box" /usr/local/bin/sing-box
fi
sing-box version | head -1
source ~/.cargo/env
cd ~/sb && cargo build -q -p admin -p provisioning-agent
install -m 0755 target/debug/vpn-admin /usr/local/bin/vpn-admin
install -m 0755 target/debug/vpn-provisioning-agent /usr/local/bin/vpn-provisioning-agent
mkdir -p /etc/vpn /etc/vpn/compat/hysteria /etc/vpn/compat/sing-box
cat > /etc/vpn/deployment.toml <<TOML
schema_version = 2
node_id = "b2-test-exit"
role = "exit"
public_host = "62.238.46.190"
subscription_host = "62.238.46.190"
[reality]
listen_port = 443
handshake_server = "www.cloudflare.com"
handshake_port = 443
[hysteria2]
listen_port = 443
[subscription]
listen_port = 9100
public_port = 9100
TOML
if [ ! -s /etc/vpn/compat/hysteria/cert.pem ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 3 \
    -subj "/CN=b2-test.invalid" -keyout /etc/vpn/compat/hysteria/key.pem -out /etc/vpn/compat/hysteria/cert.pem 2>/dev/null
  chmod 600 /etc/vpn/compat/hysteria/key.pem
fi
cat > /etc/systemd/system/sing-box.service <<UNIT
[Unit]
Description=sing-box (B2 test)
[Service]
ExecStartPre=/usr/local/bin/sing-box check -c /etc/vpn/compat/sing-box/config.json
ExecStart=/usr/local/bin/sing-box run -c /etc/vpn/compat/sing-box/config.json
Restart=on-failure
[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 vpn-admin --config /etc/vpn/deployment.toml init >/tmp/init.log 2>&1 || { tail -5 /tmp/init.log; exit 1; }
echo init-ok
ls /etc/vpn/compat /etc/vpn/compat/reality
