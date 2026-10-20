#!/usr/bin/env bash
# Regression (I-31): update.sh must record the node's own public address in
# deployment.toml (public_ipv4/public_ipv6) before it re-renders the sing-box
# config. Reproduced on published artifacts: after `update.sh --version
# v1.3.1-rc.2` from v1.3.0 the keys were absent, doctor warned 'C-16: no
# own-public-address deny rule', and tunnel users could reach the node itself.
# shellcheck disable=SC1090,SC2034  # sourced lib + variables consumed by it
set -Euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"
LIB="$REPO_ROOT/deploy/lib/own-address.sh"
failures=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; failures=$((failures + 1)); }

# 1. static: the updater sources the shared lib and calls the backfill BEFORE rendering
grep -q 'deploy/lib/own-address.sh' "$UPDATE_SH" && ok "update.sh sources deploy/lib/own-address.sh" || bad "update.sh does not source own-address.sh"
mapfile -t calls < <(grep -n '^ *update_backfill_own_address$' "$UPDATE_SH" | cut -d: -f1)
mapfile -t renders < <(grep -n 'rendering current authoritative users/REALITY state' "$UPDATE_SH" | cut -d: -f1)
if [ "${#renders[@]}" -ge 1 ] && [ "${#calls[@]}" -eq "${#renders[@]}" ]; then
  allbefore=1
  for i in "${!renders[@]}"; do [ "${calls[$i]}" -lt "${renders[$i]}" ] || allbefore=0; done
  [ "$allbefore" = 1 ] && ok "every config re-render in update.sh (${#renders[@]}) is preceded by update_backfill_own_address" || bad "backfill is not before every render (calls=${calls[*]} renders=${renders[*]})"
else bad "update.sh has ${#calls[@]} backfill calls for ${#renders[@]} render steps"; fi

# 2. functional: the shared lib, used the way update.sh uses it (no install.sh)
[ -f "$LIB" ] || { bad "deploy/lib/own-address.sh missing"; [ "$failures" -eq 0 ] || exit 1; }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
cat >"$T/legacy.toml" <<'EOF'
schema_version = 2
node_id = "fin1"
role = "exit"
public_host = "vpn.example.test"
subscription_host = "vpn.example.test"

[reality]
listen_port = 443
EOF
out="$(
  set +e
  log() { echo "LOG: $*"; }; warn() { echo "WARN: $*"; }; die() { echo "DIE: $*"; exit 1; }
  # shellcheck source=/dev/null
  . "$REPO_ROOT/deploy/lib/preflight.sh"
  # shellcheck source=/dev/null
  . "$LIB"
  preflight_local_public_addrs() { [ "$1" = 4 ] && echo 203.0.113.10 || echo 2001:db8:1::10; }
  preflight_detect_egress_ip_confirmed() { return 1; }
  preflight_resolve_host_addrs() { printf '203.0.113.10\n'; }
  DEPLOYMENT_TOML="$T/legacy.toml"
  PUBLIC_HOST=vpn.example.test
  backfill_own_public_addresses; echo "rc=$?"
)"
grep -q '^public_ipv4 = "203.0.113.10"' "$T/legacy.toml" && grep -q '^public_ipv6 = "2001:db8:1::10"' "$T/legacy.toml" \
  && ok "lib backfills both address keys on a legacy file" || bad "keys not written: $(cat "$T/legacy.toml")"
grep -q 'rc=0' <<<"$out" && ok "backfill returns 0" || bad "backfill rc: $out"
awk '/^\[/{t=1} /^public_ipv[46]/{ if(t){print "inside-table"; exit} }' "$T/legacy.toml" | grep -q inside-table \
  && bad "address keys landed inside a table (must be top-level)" || ok "address keys are top-level"
cp "$T/legacy.toml" "$T/once.toml"
( log(){ :; }; warn(){ :; }; die(){ exit 1; }
  # shellcheck source=/dev/null
  . "$REPO_ROOT/deploy/lib/preflight.sh"; . "$LIB"
  preflight_local_public_addrs() { echo 198.51.100.9; }; preflight_detect_egress_ip_confirmed() { return 1; }; preflight_resolve_host_addrs() { :; }
  DEPLOYMENT_TOML="$T/legacy.toml"; PUBLIC_HOST=vpn.example.test; backfill_own_public_addresses )
cmp -s "$T/legacy.toml" "$T/once.toml" && ok "idempotent: existing keys are never replaced" || bad "second run changed the file"

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }; echo "all passed"
