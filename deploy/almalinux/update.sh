#!/usr/bin/env bash
# Transactional update flow: build before mutation, serialize against every
# vpn-admin state change, atomically swap binaries, render from CURRENT live
# state, restart, verify, and automatically roll back every handled failure.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_DIR="/usr/local/bin"
BACKUP_ROOT="/etc/vpn/backups"
BACKUP_DIR="$BACKUP_ROOT/$(date +%Y%m%d-%H%M%S)-$$"

log() { echo "[update] $*"; }
warn() { echo "[update] WARNING: $*" >&2; }
die() { echo "[update] ERROR: $*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "must run as root"

# Serialize installers while the slow build runs. The deployment state lock
# is acquired only immediately before mutation, after all expensive work.
exec 200>/run/lock/vpn1-installer.lock
flock -x 200

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
command -v cargo >/dev/null 2>&1 \
  || die "cargo not found. Install a Rust toolchain, then re-run update.sh."

log "running tests before touching installed state..."
( cd "$REPO_ROOT" && cargo test --workspace -p admin -p subscription -p compat-config ) \
  || die "tests failed; installed state was not changed."

log "building new binaries..."
( cd "$REPO_ROOT" && cargo build --release -p admin -p subscription )

install -d -m 0700 -o root -g root "$BACKUP_ROOT"
install -d -m 0700 -o root -g root "$BACKUP_DIR"
for f in vpn-admin vpn vpn-subscription-svc; do
  [ -f "$BIN_DIR/$f" ] || die "installed binary $BIN_DIR/$f is missing; refusing a non-recoverable update"
  cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done

# Fully stage new executables before the transaction begins.
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin.update-new"
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn.update-new"
install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc.update-new"

# Block user mutations/key rotation/restore for the short commit window.
exec 201>/run/lock/vpn1.lock
flock -x 201
[ -f /etc/vpn/compat/sing-box/config.json ] \
  && cp -a /etc/vpn/compat/sing-box/config.json "$BACKUP_DIR/config.json"

mutation_started=0
committed=0
rolling_back=0

rollback_update() {
  rolling_back=1
  trap - ERR INT TERM EXIT
  set +e
  local failed=0
  warn "update did not commit; restoring previous binaries from $BACKUP_DIR"
  for f in vpn-admin vpn vpn-subscription-svc; do
    install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f.rollback" || failed=1
    mv -f "$BIN_DIR/$f.rollback" "$BIN_DIR/$f" || failed=1
  done

  # Never rewind users.json or REALITY material. They are authoritative and
  # may have changed while the build ran. Render them with the restored tool.
  if ! VPN1_LOCK_PATH="$BACKUP_DIR/rollback-inner.lock" \
      "$BIN_DIR/vpn-admin" --config /etc/vpn/deployment.toml render-config; then
    warn "rollback render failed; restoring the config snapshot taken while the state lock was held"
    if [ -f "$BACKUP_DIR/config.json" ]; then
      cp -a "$BACKUP_DIR/config.json" /etc/vpn/compat/sing-box/config.json || failed=1
    else
      failed=1
    fi
  fi
  systemctl restart vpn-subscription || failed=1
  systemctl reload-or-restart sing-box || failed=1
  /usr/local/bin/vpn-health-check || failed=1
  rm -f "$BIN_DIR/vpn-admin.update-new" "$BIN_DIR/vpn.update-new" \
    "$BIN_DIR/vpn-subscription-svc.update-new"

  if [ "$failed" -eq 0 ]; then
    echo "[update] ROLLBACK SUCCESS: previous binaries are healthy; update itself failed." >&2
    exit 1
  fi
  echo "[update] ROLLBACK FAILED: manual intervention required. Backups: $BACKUP_DIR" >&2
  exit 2
}

on_exit() {
  local rc=$?
  if [ "$rc" -ne 0 ] && [ "$mutation_started" -eq 1 ] \
      && [ "$committed" -eq 0 ] && [ "$rolling_back" -eq 0 ]; then
    rollback_update
  fi
  exit "$rc"
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

log "installing staged binaries..."
mutation_started=1
mv -f "$BIN_DIR/vpn-admin.update-new" "$BIN_DIR/vpn-admin"
mv -f "$BIN_DIR/vpn.update-new" "$BIN_DIR/vpn"
mv -f "$BIN_DIR/vpn-subscription-svc.update-new" "$BIN_DIR/vpn-subscription-svc"

log "rendering current authoritative users/REALITY state with new tooling..."
VPN1_LOCK_PATH="$BACKUP_DIR/update-inner.lock" \
  "$BIN_DIR/vpn-admin" --config /etc/vpn/deployment.toml render-config

log "restarting services..."
systemctl restart vpn-subscription
systemctl reload-or-restart sing-box

log "running health check..."
/usr/local/bin/vpn-health-check

committed=1
trap - ERR INT TERM EXIT
log "update complete. Previous binaries and config are root-only at $BACKUP_DIR."
