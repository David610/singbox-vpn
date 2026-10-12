#!/usr/bin/env bash
# Disposable-host acceptance only. Called after fresh candidate install or
# v1.2.0 -> candidate update; do not run on customer/production nodes.
set -euo pipefail
[ "${SINGBOX_VPN_DISPOSABLE_EGRESS_GATE:-0}" = 1 ] || { echo 'requires an explicitly authorized disposable host' >&2; exit 2; }
ADMIN="${EGRESS_GATE_ADMIN:-/usr/local/bin/vpn-admin}"
CONFIG="${EGRESS_GATE_CONFIG:-/etc/vpn/deployment.toml}"
[ "$("$ADMIN" --config "$CONFIG" egress show | head -1)" = 'ipv6_literal: allow (plain exit default)' ] || { echo 'requires the fresh standalone allow baseline' >&2; exit 2; }
restore_default() {
  if ! "$ADMIN" --config "$CONFIG" egress clear >/dev/null; then
    echo 'FAIL: disposable egress gate could not restore its allow baseline' >&2
    exit 1
  fi
}
trap restore_default EXIT
for policy in reject_google reject_all allow; do
  "$ADMIN" --config "$CONFIG" egress set "$policy" >/dev/null
  "$ADMIN" --config "$CONFIG" render-config --require-applied >/dev/null
  "$ADMIN" --config "$CONFIG" doctor --protocol --require-protocol >/dev/null
  # A second render must be a no-op. The regular reconcile/idempotence tests
  # additionally assert that a matching fingerprint does not restart services.
  "$ADMIN" --config "$CONFIG" render-config --require-applied >/dev/null
  echo "PASS: $policy disposable apply/reconcile/doctor"
done
