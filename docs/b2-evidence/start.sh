set -euo pipefail
mkdir -p /root/b2 && chmod 700 /root/b2
install -m 0700 /tmp/mockcp.py /root/b2/mockcp.py
vpn-admin --config /etc/vpn/deployment.toml render-config >/root/b2/render.log 2>&1 || true
systemctl restart sing-box; sleep 1; systemctl is-active sing-box
cat > /root/b2/agent.toml <<T
worker_url = "http://127.0.0.1:8788"
node_id = "b2-test-exit"
agent_api_key = "test-only-not-a-secret"
vpn_admin_binary = "/usr/local/bin/vpn-admin"
vpn_admin_config = "/etc/vpn/deployment.toml"
poll_interval_secs = 3
lease_pool_size = 4
lease_slot_lifetime_secs = 900
lease_state_file = "/var/lib/vpn-provisioning-agent/lease-pool.json"
T
systemd-run --unit=b2-mockcp --collect /usr/bin/python3 /root/b2/mockcp.py
sleep 1
systemd-run --unit=b2-agent --collect -E RUST_LOG=info /usr/local/bin/vpn-provisioning-agent --config /root/b2/agent.toml
sleep 12
journalctl -u b2-agent --no-pager -o cat | tail -8
cat /root/b2/sync.log | tail -3
ls -l /var/lib/vpn-provisioning-agent/
