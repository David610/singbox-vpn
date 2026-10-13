#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
GATE="$ROOT/deploy/lib/verify-exit-egress-policy.sh"
if bash "$GATE" >/dev/null 2>&1; then echo 'gate accepted an unauthorized host' >&2; exit 1; fi
cat > "$TMP/admin" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
shift 2 # --config PATH
case "$1 $2" in
  'egress show') echo "${MOCK_SHOW:-ipv6_literal: allow (plain exit default)}" ;;
  'egress set') echo "$3" > "$GATE_STATE"; echo "set $3" >> "$GATE_CALLS" ;;
  'egress clear') echo allow > "$GATE_STATE"; echo clear >> "$GATE_CALLS" ;;
  'render-config --require-applied')
    echo render >> "$GATE_CALLS"
    [ "$(cat "$GATE_STATE")" != "${FAIL_POLICY:-none}" ] ;;
  'doctor --protocol') [ "$3" = --require-protocol ]; echo doctor >> "$GATE_CALLS" ;;
  *) exit 3 ;;
esac
MOCK
chmod 0755 "$TMP/admin"
export EGRESS_GATE_ADMIN="$TMP/admin" EGRESS_GATE_CONFIG="$TMP/deployment.toml"
export GATE_STATE="$TMP/state" GATE_CALLS="$TMP/calls"
SINGBOX_VPN_DISPOSABLE_EGRESS_GATE=1 bash "$GATE" >/dev/null
[ "$(cat "$GATE_STATE")" = allow ]
[ "$(grep -c '^render$' "$GATE_CALLS")" = 6 ]
[ "$(grep -c '^doctor$' "$GATE_CALLS")" = 3 ]
[ "$(tail -1 "$GATE_CALLS")" = clear ]
if FAIL_POLICY=reject_all SINGBOX_VPN_DISPOSABLE_EGRESS_GATE=1 bash "$GATE" >/dev/null 2>&1; then
  echo 'injected reconcile failure was lost' >&2; exit 1
fi
[ "$(cat "$GATE_STATE")" = allow ]
[ "$(tail -1 "$GATE_CALLS")" = clear ]
# A fresh candidate install declares reject_google (installer default since
# v1.3.1): the gate must accept that baseline and restore exactly it.
: > "$GATE_CALLS"
MOCK_SHOW='ipv6_literal: reject_google' SINGBOX_VPN_DISPOSABLE_EGRESS_GATE=1 bash "$GATE" >/dev/null
[ "$(cat "$GATE_STATE")" = reject_google ]
[ "$(tail -1 "$GATE_CALLS")" = 'set reject_google' ]
[ "$(grep -c '^clear$' "$GATE_CALLS")" = 0 ]
if MOCK_SHOW='ipv6_literal: reject_all' SINGBOX_VPN_DISPOSABLE_EGRESS_GATE=1 bash "$GATE" >/dev/null 2>&1; then
  echo 'gate accepted a non-baseline policy' >&2; exit 1
fi
echo 'disposable egress gate authorization, sequencing, failure propagation and cleanup: PASS'
