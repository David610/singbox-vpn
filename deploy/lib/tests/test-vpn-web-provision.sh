#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
HELPER="$ROOT/deploy/almalinux/vpn-web-provision"
TMP="$(mktemp -d)"
trap 'sudo rm -rf "$TMP"' EXIT

STUB="$TMP/vpn-admin"
LOG="$TMP/calls.log"
STATE="$TMP/state"
USER_ID="123e4567-e89b-42d3-a456-426614174000"

cat > "$STUB" <<'STUB'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >> "$VPN_WEB_TEST_LOG"
case "$*" in
  user\ create*)
    printf 'config applied\n'
    printf '{"id":"user_web","name":"web","enabled":true,"subscription_url":"https://sub.example/sub/test-token?format=hiddify","provisioning_url":"https://sub.example/v1/provision/test-token"}\n'
    ;;
  user\ enable*|user\ disable*)
    ;;
  *)
    exit 99
    ;;
esac
STUB
chmod 0755 "$STUB"

run_helper() {
  sudo env     VPN_ADMIN="$STUB"     VPN_WEB_STATE_DIR="$STATE"     VPN_WEB_TEST_LOG="$LOG"     "$HELPER" "$@"
}

first="$(run_helper provision "$USER_ID")"
second="$(run_helper provision "$USER_ID")"
run_helper disable "$USER_ID" >/dev/null
run_helper enable "$USER_ID" >/dev/null

# The helper correctly creates root-only state. Copy the non-secret call log
# into the test user's context only after all privileged actions are complete.
calls="$(sudo cat "$LOG")"

[[ "$(printf '%s' "$first" | jq -r .id)" == "user_web" ]]
[[ "$first" == "$second" ]]
[[ "$(printf '%s\n' "$calls" | grep -c '^user create ')" -eq 1 ]]
printf '%s\n' "$calls" | grep -q '^user disable user_web$'
printf '%s\n' "$calls" | grep -q '^user enable user_web$'
sudo test "$(sudo stat -c '%a' "$STATE/$USER_ID.json")" = "600"

printf 'vpn-web-provision tests: OK\n'
