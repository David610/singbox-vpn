#!/usr/bin/env bash
# Boots the full local vertical slice: test-service -> relay-agent
# (combined ingress+egress) -> rendezvous -> client-daemon, all on
# loopback with freshly generated dev-only keys/certs. See
# docs/DEPLOYMENT.md. Not for production use — see that doc's caveats.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

LOG_DIR="$(mktemp -d)"
echo "logs: $LOG_DIR"

echo "[1/4] building..."
cargo build -q -p test-service -p relay-agent -p rendezvous -p client-daemon -p cli

echo "[2/4] starting test-service on 127.0.0.1:8081"
TEST_SERVICE_BIND=127.0.0.1:8081 ./target/debug/test-service >"$LOG_DIR/test-service.log" 2>&1 &
echo $! >"$LOG_DIR/test-service.pid"

echo "[2/4] starting relay-agent (combined) on 127.0.0.1:9443 (tls) / 127.0.0.1:9444 (quic)"
RUST_LOG=info ./target/debug/relay-agent --role combined --bind-tls 127.0.0.1:9443 --bind-quic 127.0.0.1:9444 \
  >"$LOG_DIR/relay-agent.log" 2>&1 &
echo $! >"$LOG_DIR/relay-agent.pid"
sleep 1
CERT_SHA256=$(grep -o 'cert_sha256=[0-9a-f]*' "$LOG_DIR/relay-agent.log" | head -1 | cut -d= -f2)
if [ -z "$CERT_SHA256" ]; then
  echo "failed to read relay cert_sha256 from log, see $LOG_DIR/relay-agent.log" >&2
  exit 1
fi
echo "relay cert_sha256=$CERT_SHA256"

cat >deploy/local/relay-pool.json <<EOF
[
  {
    "id": "relay-1",
    "transport": "direct-tls",
    "address": "127.0.0.1:9443",
    "provider_tag": "dev",
    "capabilities": ["STREAM"],
    "cert_sha256_hex": "$CERT_SHA256"
  }
]
EOF

echo "[3/4] starting rendezvous on 127.0.0.1:9000"
./target/debug/rendezvous --bind 127.0.0.1:9000 --pool-file deploy/local/relay-pool.json \
  >"$LOG_DIR/rendezvous.log" 2>&1 &
echo $! >"$LOG_DIR/rendezvous.pid"
sleep 1

echo "[4/4] starting client-daemon SOCKS5 proxy on 127.0.0.1:1080"
./target/debug/client-daemon --socks-bind 127.0.0.1:1080 \
  --rendezvous-url http://127.0.0.1:9000 \
  --rendezvous-root-pub-hex-file deploy/local/relay-pool.json.root_pub \
  >"$LOG_DIR/client-daemon.log" 2>&1 &
echo $! >"$LOG_DIR/client-daemon.pid"
sleep 1

echo
echo "up. try:"
echo "  curl --socks5-hostname 127.0.0.1:1080 http://127.0.0.1:8081/"
echo
echo "pids + logs in $LOG_DIR — stop with:"
echo "  kill \$(cat $LOG_DIR/*.pid)"
