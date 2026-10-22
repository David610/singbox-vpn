#!/usr/bin/env bash
# Disposable-host acceptance only. Called after fresh candidate install or
# v1.2.0 -> candidate update; do not run on customer/production nodes.
set -euo pipefail
[ "${SINGBOX_VPN_DISPOSABLE_EGRESS_GATE:-0}" = 1 ] || { echo 'requires an explicitly authorized disposable host' >&2; exit 2; }
ADMIN="${EGRESS_GATE_ADMIN:-/usr/local/bin/vpn-admin}"
CONFIG="${EGRESS_GATE_CONFIG:-/etc/vpn/deployment.toml}"
# A fresh candidate install declares reject_google (the installer default
# since v1.3.1); a v1.2.0 -> candidate update keeps the allow default. Either
# is a valid fresh standalone baseline; the gate restores whichever it found.
initial="$("$ADMIN" --config "$CONFIG" egress show | head -1 || true)"
case "$initial" in
  'ipv6_literal: allow (plain exit default)') baseline=allow ;;
  'ipv6_literal: reject_google') baseline=reject_google ;;
  *) echo 'requires the fresh standalone baseline (allow default or installer-declared reject_google)' >&2; exit 2 ;;
esac
restore_default() {
  if [ "$baseline" = allow ]; then
    "$ADMIN" --config "$CONFIG" egress clear >/dev/null
  else
    "$ADMIN" --config "$CONFIG" egress set "$baseline" >/dev/null
  fi || { echo "FAIL: disposable egress gate could not restore its $baseline baseline" >&2; exit 1; }
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
