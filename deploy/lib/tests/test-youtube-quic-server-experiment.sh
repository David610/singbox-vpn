#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$ROOT/deploy/lib/youtube-quic-server-experiment.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

CONFIG="$TMP/config.json"
DEPLOY="$TMP/deployment.toml"
LOCK="$TMP/lock"
SB="$TMP/sing-box"
SYSTEMCTL="$TMP/systemctl"
STATE="$TMP/systemctl-state"

cat >"$SB" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == check && "${2:-}" == -c && -n "${3:-}" ]]
if [[ "${FAIL_CHECK:-0}" == 1 ]]; then exit 1; fi
jq empty "$3"
SH
chmod +x "$SB"

cat >"$SYSTEMCTL" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  restart)
    if [[ "${FAIL_RESTART:-0}" == 1 ]]; then exit 1; fi
    printf active >"$FAKE_SYSTEMCTL_STATE"
    ;;
  is-active)
    [[ -f "$FAKE_SYSTEMCTL_STATE" ]]
    ;;
  *) exit 2 ;;
esac
SH
chmod +x "$SYSTEMCTL"

export SINGBOX_VPN_CONFIG_PATH="$CONFIG"
export SINGBOX_VPN_DEPLOYMENT_PATH="$DEPLOY"
export SINGBOX_VPN_SING_BOX_BIN="$SB"
export SINGBOX_VPN_SYSTEMCTL="$SYSTEMCTL"
export SINGBOX_VPN_SERVICE_NAME=sing-box
export SINGBOX_VPN_EXPERIMENT_LOCK="$LOCK"
export FAKE_SYSTEMCTL_STATE="$STATE"

reset_fixture() {
  cat >"$CONFIG" <<'JSON'
{
  "log": {"level": "fatal"},
  "inbounds": [{"type": "vless", "tag": "vless-reality-in"}],
  "outbounds": [{"type": "direct", "tag": "direct"}]
}
JSON
  cat >"$DEPLOY" <<'TOML'
schema_version = 2
node_id = "de1"
role = "exit"
public_host = "de1.example.test"
TOML
  rm -f "$STATE" "$LOCK"
}

rule_count() {
  jq '[.route.rules[]? | select(.network == "udp" and .port == 443 and .action == "reject" and .method == "default" and .no_drop == true)] | length' "$CONFIG"
}

reset_fixture
[[ "$(bash "$SCRIPT" status)" == DISABLED:* ]]

bash "$SCRIPT" enable >/dev/null
[[ "$(rule_count)" == 1 ]]
[[ "$(jq -r '.route.rules[0].network' "$CONFIG")" == udp ]]
[[ "$(jq -r '.route.rules[0].port' "$CONFIG")" == 443 ]]
[[ "$(jq -r '.route.rules[0].action' "$CONFIG")" == reject ]]
[[ "$(jq -r '.route.rules[0].method' "$CONFIG")" == default ]]
[[ "$(jq -r '.route.rules[0].no_drop' "$CONFIG")" == true ]]

# Enable is idempotent.
bash "$SCRIPT" enable >/dev/null
[[ "$(rule_count)" == 1 ]]

bash "$SCRIPT" disable >/dev/null
[[ "$(rule_count)" == 0 ]]
[[ "$(jq -r 'has("route")' "$CONFIG")" == false ]]

# Relay use is refused before mutation.
reset_fixture
sed -i 's/role = "exit"/role = "relay"/' "$DEPLOY"
BEFORE="$(sha256sum "$CONFIG" | awk '{print $1}')"
if bash "$SCRIPT" enable >/dev/null 2>&1; then
  echo "relay enable unexpectedly succeeded" >&2
  exit 1
fi
AFTER="$(sha256sum "$CONFIG" | awk '{print $1}')"
[[ "$BEFORE" == "$AFTER" ]]

# Validation failure is fail-closed.
reset_fixture
BEFORE="$(sha256sum "$CONFIG" | awk '{print $1}')"
if FAIL_CHECK=1 bash "$SCRIPT" enable >/dev/null 2>&1; then
  echo "validation failure unexpectedly succeeded" >&2
  exit 1
fi
AFTER="$(sha256sum "$CONFIG" | awk '{print $1}')"
[[ "$BEFORE" == "$AFTER" ]]

# Restart failure rolls the exact prior config back.
reset_fixture
BEFORE="$(sha256sum "$CONFIG" | awk '{print $1}')"
if FAIL_RESTART=1 bash "$SCRIPT" enable >/dev/null 2>&1; then
  echo "restart failure unexpectedly succeeded" >&2
  exit 1
fi
AFTER="$(sha256sum "$CONFIG" | awk '{print $1}')"
[[ "$BEFORE" == "$AFTER" ]]

echo "youtube-quic-server-experiment tests: PASS"
