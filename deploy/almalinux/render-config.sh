#!/usr/bin/env bash
# Thin wrapper: regenerate + validate + atomically apply the sing-box
# config from the current user store, then reload the service if it
# changed. Safe to run any time (e.g. from a cron/timer as a
# belt-and-suspenders reconciliation, though `vpn-admin user ...`
# already triggers this after every mutation).
set -euo pipefail

DEPLOYMENT_TOML="${DEPLOYMENT_TOML:-/etc/vpn/deployment.toml}"

/usr/local/bin/vpn-admin --config "$DEPLOYMENT_TOML" render-config

if systemctl is-active --quiet sing-box; then
  systemctl reload-or-restart sing-box
fi
