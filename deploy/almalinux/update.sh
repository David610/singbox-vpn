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

# Mutual exclusion between install.sh/update.sh themselves
# (docs/FINAL_PRODUCTION_AUDIT.md P0-4: "two concurrent administrators
# must not lose each other's changes"). Deliberately a SEPARATE lock file
# from vpn-admin's own /run/lock/vpn1.lock (see apps/admin/src/lock.rs):
# this script invokes `vpn-admin render-config` below, which acquires
# that lock itself for the duration of that one call — holding the same
# lock around this whole script would deadlock the moment it shells out
# to vpn-admin. This lock only serializes concurrent install.sh/update.sh
# runs against each other; vpn-admin's own lock independently serializes
# concurrent `vpn user ...`/`vpn restore`/`vpn init --rotate` calls.
exec 200>/run/lock/vpn1-installer.lock
flock -x 200

log "backing up current binaries + config to $BACKUP_DIR"
mkdir -p "$BACKUP_DIR"
for f in vpn-admin vpn vpn-subscription-svc; do
  [ -f "$BIN_DIR/$f" ] && cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done
[ -f /etc/vpn/compat/sing-box/config.json ] && cp -a /etc/vpn/compat/sing-box/config.json "$BACKUP_DIR/config.json"
# The rollback below restores the OLD binary. `render-config` further down
# runs the NEW one against the live user store and key material, so those
# must be recoverable too — otherwise a rollback leaves an old binary
# reading state a newer one may have rewritten.
[ -f /etc/vpn/compat/users/users.json ] && cp -a /etc/vpn/compat/users/users.json "$BACKUP_DIR/users.json"
[ -d /etc/vpn/compat/reality ] && cp -a /etc/vpn/compat/reality "$BACKUP_DIR/reality"

# A prebuilt-release install has no Rust toolchain at all, and even when one
# exists rustup puts it in ~/.cargo/bin, which a `curl | sudo bash` PATH does
# not include. install.sh learned this; update.sh did not, so it aborted here
# on exactly the installs the release path produces.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
command -v cargo >/dev/null 2>&1 \
  || die "cargo not found. update.sh builds from source, so it needs a Rust toolchain.
Install one with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
then re-run this script."

log "running tests before touching any installed binary..."
( cd "$REPO_ROOT" && cargo test --workspace -p admin -p subscription -p compat-config ) \
  || die "tests failed — refusing to build/install new binaries. Nothing was changed."

log "building new binaries..."
( cd "$REPO_ROOT" && cargo build --release -p admin -p subscription )

log "installing new binaries..."
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin.new"
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn.new"
install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc.new"
mv "$BIN_DIR/vpn-admin.new" "$BIN_DIR/vpn-admin"
mv "$BIN_DIR/vpn.new" "$BIN_DIR/vpn"
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
  for f in vpn-admin vpn vpn-subscription-svc; do
    [ -f "$BACKUP_DIR/$f" ] && install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f"
  done
  if [ -f "$BACKUP_DIR/config.json" ]; then
    # `cp -a` preserves the backup's own mode/owner (0640 root:sing-box,
    # set by apply_config_atomically when it was originally written) —
    # never hardcode 0644 here (docs/PRODUCTION_HARDENING_PLAN.md #29).
    # config.json contains the REALITY private key, VLESS UUIDs, and
    # Hysteria2 passwords in cleartext; restoring it world-readable would
    # undo the whole point of the rollback.
    # Restore the user store and key material first, then RE-RENDER rather than
  # reinstating a point-in-time config.json. update.sh holds the installer
  # lock, not vpn-admin's state lock, so `vpn user create/remove` can and does
  # run concurrently during the (minutes-long) build above — reinstating the
  # snapshot silently resurrected users who had been revoked in the meantime.
  # users.json is the authoritative store; the old binary can regenerate a
  # correct config from it.
  [ -f "$BACKUP_DIR/users.json" ] && cp -a "$BACKUP_DIR/users.json" /etc/vpn/compat/users/users.json
  [ -d "$BACKUP_DIR/reality" ] && cp -a "$BACKUP_DIR/reality/." /etc/vpn/compat/reality/
  if ! "$BIN_DIR/vpn-admin" --config /etc/vpn/deployment.toml render-config >/dev/null 2>&1; then
    warn "re-render during rollback failed; falling back to the config.json snapshot."
    cp -a "$BACKUP_DIR/config.json" /etc/vpn/compat/sing-box/config.json
  fi
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
