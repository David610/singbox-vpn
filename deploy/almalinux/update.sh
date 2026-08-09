#!/usr/bin/env bash
# Safe update flow (spec §48): build, validate, atomically install,
# restart, health-check; roll back on failure rather than leaving a
# broken service running.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_DIR="/usr/local/bin"
BACKUP_DIR="/etc/vpn/backups/$(date +%Y%m%d-%H%M%S)"

log() { echo "[update] $*"; }
die() { echo "[update] ERROR: $*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "must run as root"

log "backing up current binaries + config to $BACKUP_DIR"
mkdir -p "$BACKUP_DIR"
for f in vpn-admin vpn-subscription-svc; do
  [ -f "$BIN_DIR/$f" ] && cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done
[ -f /etc/vpn/compat/sing-box/config.json ] && cp -a /etc/vpn/compat/sing-box/config.json "$BACKUP_DIR/config.json"

log "running tests before touching any installed binary..."
( cd "$REPO_ROOT" && cargo test --workspace -p admin -p subscription -p compat-config ) \
  || die "tests failed — refusing to build/install new binaries. Nothing was changed."

log "building new binaries..."
( cd "$REPO_ROOT" && cargo build --release -p admin -p subscription )

log "installing new binaries..."
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin.new"
install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc.new"
mv "$BIN_DIR/vpn-admin.new" "$BIN_DIR/vpn-admin"
mv "$BIN_DIR/vpn-subscription-svc.new" "$BIN_DIR/vpn-subscription-svc"

log "re-rendering + validating sing-box config against the new tooling..."
if ! /usr/local/bin/vpn-admin --config /etc/vpn/deployment.toml render-config; then
  die "config render/validate failed — old binaries are backed up at $BACKUP_DIR, config untouched (apply_config_atomically never overwrote it)"
fi

log "restarting services..."
systemctl restart vpn-subscription
systemctl reload-or-restart sing-box

log "running health check..."
if ! /usr/local/bin/vpn-health-check; then
  log "health check failed — rolling back binaries from $BACKUP_DIR"
  for f in vpn-admin vpn-subscription-svc; do
    [ -f "$BACKUP_DIR/$f" ] && install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f"
  done
  if [ -f "$BACKUP_DIR/config.json" ]; then
    # `cp -a` preserves the backup's own mode/owner (0640 root:sing-box,
    # set by apply_config_atomically when it was originally written) —
    # never hardcode 0644 here (docs/PRODUCTION_HARDENING_PLAN.md #29).
    # config.json contains the REALITY private key, VLESS UUIDs, and
    # Hysteria2 passwords in cleartext; restoring it world-readable would
    # undo the whole point of the rollback.
    cp -a "$BACKUP_DIR/config.json" /etc/vpn/compat/sing-box/config.json
  fi
  systemctl restart vpn-subscription
  systemctl reload-or-restart sing-box

  # A rollback is only a success if the RESTORED services are actually
  # healthy — re-run the health check rather than assuming the restart
  # succeeded (docs/PRODUCTION_HARDENING_PLAN.md #28).
  if /usr/local/bin/vpn-health-check; then
    die "rolled back to previous binaries/config — ROLLBACK SUCCESS (services healthy on previous version); update itself did not complete"
  else
    die "ROLLBACK FAILED: restored previous binaries/config but the post-rollback health check ALSO failed. Manual intervention required — check 'systemctl status sing-box vpn-subscription' and 'journalctl -u sing-box -u vpn-subscription'."
  fi
fi

log "update complete. Previous version backed up at $BACKUP_DIR."
