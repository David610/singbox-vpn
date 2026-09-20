#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
HELPER="$ROOT/deploy/almalinux/vpn-web-provision"
TMP="$(mktemp -d)"
trap 'sudo rm -rf "$TMP"' EXIT

STUB="$TMP/vpn-admin"
LOG="$TMP/calls.log"
ROSTER="$TMP/roster.txt"
STATE="$TMP/state"
USER_ID="123e4567-e89b-42d3-a456-426614174000"
ORPHAN_ID="223e4567-e89b-42d3-a456-426614174001"
ORPHAN_NAME="web-${ORPHAN_ID//-/}"

cat > "$STUB" <<'STUB'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >> "$VPN_WEB_TEST_LOG"

case "$*" in
  "user list")
    printf '%-20s %-16s %-8s\n' "ID" "NAME" "ENABLED"
    if [[ -f "$VPN_WEB_TEST_ROSTER" ]]; then
      cat "$VPN_WEB_TEST_ROSTER"
    fi
    ;;

  user\ create*)
    name=""
    previous=""
    for arg in "$@"; do
      if [[ "$previous" == "--name" ]]; then
        name="$arg"
        break
      fi
      previous="$arg"
    done
    [[ -n "$name" ]]
    printf 'user_web %s yes\n' "$name" >> "$VPN_WEB_TEST_ROSTER"
    printf '{"id":"user_web","name":"%s","enabled":true,"subscription_url":"https://sub.example/sub/test-token?format=hiddify","provisioning_url":"https://sub.example/v1/provision/test-token"}\n' "$name"
    ;;

  user\ rotate-token*)
    id="$3"
    printf 'New Hiddify subscription URL for %s:\n' "$id"
    printf '  https://sub.example/sub/recovered-%s?format=hiddify\n' "$id"
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
  sudo env \
    VPN_ADMIN="$STUB" \
    VPN_WEB_STATE_DIR="$STATE" \
    VPN_WEB_TEST_LOG="$LOG" \
    VPN_WEB_TEST_ROSTER="$ROSTER" \
    "$HELPER" "$@"
}

# Normal provision is idempotent: the second call returns the persisted mapping
# and does not mint a second VPN identity.
first="$(run_helper provision "$USER_ID")"
second="$(run_helper provision "$USER_ID")"
run_helper disable "$USER_ID" >/dev/null
run_helper enable "$USER_ID" >/dev/null

[[ "$(printf '%s' "$first" | jq -r .id)" == "user_web" ]]
[[ "$first" == "$second" ]]
sudo test "$(sudo stat -c '%a' "$STATE/$USER_ID.json")" = "600"

# Simulate the only dangerous crash boundary: vpn-admin successfully created a
# deterministic web user, but the helper died before writing its mapping file.
# A retry must find that user, rotate a fresh token, and persist it rather than
# create a duplicate.
sudo sh -c "printf '%s %s yes\n' 'user_orphan' '$ORPHAN_NAME' >> '$ROSTER'"
recovered="$(run_helper provision "$ORPHAN_ID")"

[[ "$(printf '%s' "$recovered" | jq -r .id)" == "user_orphan" ]]
[[ "$(printf '%s' "$recovered" | jq -r .subscription_url)" ==    "https://sub.example/sub/recovered-user_orphan?format=hiddify" ]]
[[ "$(printf '%s' "$recovered" | jq -r .provisioning_url)" ==    "https://sub.example/v1/provision/recovered-user_orphan" ]]

# The helper correctly creates root-only state. Read only the non-secret command
# log via sudo after all privileged actions are complete.
calls="$(sudo cat "$LOG")"
[[ "$(printf '%s\n' "$calls" | grep -c '^user create ')" -eq 1 ]]
[[ "$(printf '%s\n' "$calls" | grep -c '^user rotate-token user_orphan$')" -eq 1 ]]
printf '%s\n' "$calls" | grep -q '^user disable user_web$'
printf '%s\n' "$calls" | grep -q '^user enable user_web$'

printf 'vpn-web-provision tests: OK\n'
