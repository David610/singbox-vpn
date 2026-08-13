#!/usr/bin/env bash
# Transactional update flow: build before mutation, serialize against every
# vpn-admin state change, atomically swap binaries, render from CURRENT live
# state, restart, verify, and automatically roll back every handled failure.
#
# Keeps runtime state in lockstep with what a fresh install.sh run would
# produce: binaries, systemd units (with a daemon-reload + restart-if-
# affected so unit-file-only changes like a Nice= tweak actually take
# effect), and the helper scripts installed to $BIN_DIR
# (vpn-health-check, vpn-benchmark + its vpn-benchmark-lib.sh sidecar).
# Every one of those categories is staged, backed up, and rolled back
# exactly like the binaries already were — a partial update must not
# leave any of them ahead of (or behind) the others.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_DIR="/usr/local/bin"
SYSTEMD_DIR="/etc/systemd/system"
BACKUP_ROOT="/etc/vpn/backups"
BACKUP_DIR="$BACKUP_ROOT/$(date +%Y%m%d-%H%M%S)-$$"

# All systemd units install.sh's install_systemd_units() installs — kept
# as the single list both scripts key off conceptually (duplicated by
# necessity, since this is bash, not shared code; if you add a unit to
# one, add it to the other in the same commit, same discipline as
# SINGBOX_VERSION/SINGBOX_SHA256_* below).
SYSTEMD_UNITS=(sing-box.service vpn-subscription.service vpn-expiry-reconcile.service vpn-expiry-reconcile.timer)

log() { echo "[update] $*"; }
warn() { echo "[update] WARNING: $*" >&2; }
die() { echo "[update] ERROR: $*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "must run as root"

# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/perf-tuning.sh"

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
install -d -m 0700 -o root -g root "$BACKUP_DIR/systemd"
for f in vpn-admin vpn vpn-subscription-svc; do
  [ -f "$BIN_DIR/$f" ] || die "installed binary $BIN_DIR/$f is missing; refusing a non-recoverable update"
  cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done
# Every unit install.sh installs must already exist on any real prior
# install — same "refuse a non-recoverable update" posture as the
# binaries loop above, so an unexpectedly missing unit is a hard stop,
# not a silent skip.
for u in "${SYSTEMD_UNITS[@]}"; do
  [ -f "$SYSTEMD_DIR/$u" ] || die "installed systemd unit $SYSTEMD_DIR/$u is missing; refusing a non-recoverable update"
  cp -a "$SYSTEMD_DIR/$u" "$BACKUP_DIR/systemd/$u"
done
# Helper scripts are allowed to be genuinely absent — an installation
# from before vpn-benchmark (or even health-check) existed has none of
# these yet, and this update is exactly what brings it current. Back up
# only what's actually there; rollback below deletes what wasn't.
for f in vpn-health-check vpn-benchmark vpn-benchmark-lib.sh; do
  [ -f "$BIN_DIR/$f" ] && cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done

# Fully stage new executables/units/helper-scripts before the
# transaction begins — same "everything ready before any live file
# moves" discipline as the binaries.
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin.update-new"
install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn.update-new"
install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc.update-new"
for u in "${SYSTEMD_UNITS[@]}"; do
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/$u" "$SYSTEMD_DIR/$u.update-new"
done
install -m 0755 "$REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check.update-new"
install -m 0755 "$REPO_ROOT/deploy/lib/vpn-benchmark.sh" "$BIN_DIR/vpn-benchmark.update-new"
install -m 0644 "$REPO_ROOT/deploy/lib/vpn-benchmark-lib.sh" "$BIN_DIR/vpn-benchmark-lib.sh.update-new"

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
  warn "update did not commit; restoring previous binaries/units/helper scripts from $BACKUP_DIR"
  for f in vpn-admin vpn vpn-subscription-svc; do
    install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f.rollback" || failed=1
    mv -f "$BIN_DIR/$f.rollback" "$BIN_DIR/$f" || failed=1
  done
  for u in "${SYSTEMD_UNITS[@]}"; do
    if [ -f "$BACKUP_DIR/systemd/$u" ]; then
      install -m 0644 "$BACKUP_DIR/systemd/$u" "$SYSTEMD_DIR/$u.rollback" || failed=1
      mv -f "$SYSTEMD_DIR/$u.rollback" "$SYSTEMD_DIR/$u" || failed=1
    fi
    rm -f "$SYSTEMD_DIR/$u.update-new"
  done
  systemctl daemon-reload || failed=1
  for f in vpn-health-check vpn-benchmark vpn-benchmark-lib.sh; do
    if [ -f "$BACKUP_DIR/$f" ]; then
      install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f.rollback" || failed=1
      mv -f "$BIN_DIR/$f.rollback" "$BIN_DIR/$f" || failed=1
    else
      # No prior version existed (this update introduced it) — restore
      # means restoring absence, not leaving a half-updated new file
      # behind. Safe: nothing else's systemd unit ExecStart references
      # these helper scripts.
      rm -f "$BIN_DIR/$f"
    fi
    rm -f "$BIN_DIR/$f.update-new"
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
    echo "[update] ROLLBACK SUCCESS: previous binaries/units/helper scripts are healthy; update itself failed." >&2
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

log "installing staged binaries, systemd units, and helper scripts..."
mutation_started=1
mv -f "$BIN_DIR/vpn-admin.update-new" "$BIN_DIR/vpn-admin"
mv -f "$BIN_DIR/vpn.update-new" "$BIN_DIR/vpn"
mv -f "$BIN_DIR/vpn-subscription-svc.update-new" "$BIN_DIR/vpn-subscription-svc"
for u in "${SYSTEMD_UNITS[@]}"; do
  mv -f "$SYSTEMD_DIR/$u.update-new" "$SYSTEMD_DIR/$u"
done
mv -f "$BIN_DIR/vpn-health-check.update-new" "$BIN_DIR/vpn-health-check"
mv -f "$BIN_DIR/vpn-benchmark.update-new" "$BIN_DIR/vpn-benchmark"
mv -f "$BIN_DIR/vpn-benchmark-lib.sh.update-new" "$BIN_DIR/vpn-benchmark-lib.sh"

# Required so a unit-file-only change (e.g. sing-box.service's Nice=)
# actually takes effect on the restart below — systemd caches unit
# definitions and does not notice an on-disk edit without this.
log "reloading systemd unit definitions..."
systemctl daemon-reload

log "rendering current authoritative users/REALITY state with new tooling..."
VPN1_LOCK_PATH="$BACKUP_DIR/update-inner.lock" \
  "$BIN_DIR/vpn-admin" --config /etc/vpn/deployment.toml render-config

log "restarting services..."
systemctl restart vpn-subscription

# sing-box.service has no ExecReload=, so any restart of it is a full
# process restart that drops every live VLESS+REALITY/Hysteria2
# connection. render-config above already reloaded/restarted sing-box
# if its rendered config actually changed (apps/admin's own no-op fast
# path — render_and_apply_singbox_config's target_already_matches
# check). Unconditionally restarting again here — as this script used
# to — guaranteed a connection-dropping restart on every update even
# when sing-box had nothing new to pick up. The one case render-config
# cannot see or handle itself is the sing-box.service UNIT FILE
# changing (e.g. the Nice= priority tweak) independent of config
# content — that only takes effect at process exec time, never via
# sing-box's own config reload, so it still needs an explicit restart.
singbox_config_changed=1
if [ -f "$BACKUP_DIR/config.json" ] && [ -f /etc/vpn/compat/sing-box/config.json ] \
    && cmp -s "$BACKUP_DIR/config.json" /etc/vpn/compat/sing-box/config.json; then
  singbox_config_changed=0
fi
singbox_unit_changed=1
if [ -f "$BACKUP_DIR/systemd/sing-box.service" ] \
    && cmp -s "$BACKUP_DIR/systemd/sing-box.service" "$SYSTEMD_DIR/sing-box.service"; then
  singbox_unit_changed=0
fi
if [ "$singbox_config_changed" -eq 0 ] && [ "$singbox_unit_changed" -eq 1 ]; then
  log "sing-box.service unit file changed (config content did not); restarting to pick it up..."
  systemctl reload-or-restart sing-box
elif [ "$singbox_config_changed" -eq 1 ]; then
  log "sing-box config changed; render-config above already reloaded/restarted it."
else
  log "sing-box config and unit file unchanged; not restarting (keeps live connections up)."
fi

log "running health check..."
/usr/local/bin/vpn-health-check

# Health check alone is L1-L3 (process/config-syntax/TLS) only — it
# cannot see REALITY key material disagreeing between what sing-box
# enforces and what vpn-subscription advertises, and it cannot see a
# syntactically-valid-but-functionally-broken REALITY config, both of
# which install.sh's own acceptance gate already blocks on. An update
# is exactly as capable of introducing that class of regression as an
# install is, so it must be held to the same bar before being allowed
# to commit — see install.sh's acceptance_stage for the precedent this
# mirrors.
log "verifying real protocol handshake..."
doctor_output=""
doctor_rc=0
doctor_output="$("$BIN_DIR/vpn" --config /etc/vpn/deployment.toml doctor --protocol --require-protocol 2>&1)" || doctor_rc=$?
if [ "$doctor_rc" -ne 0 ]; then
  echo "$doctor_output" >&2
  die "post-update protocol acceptance check failed (status $doctor_rc) — see output above. Rolling back to the previous working binaries/config."
fi

committed=1
trap - ERR INT TERM EXIT

# Best-effort re-apply of kernel network tuning (idempotent — a no-op if
# already applied by a prior install.sh/update.sh run). Never blocks or
# rolls back an otherwise-successful update; see deploy/lib/perf-tuning.sh.
perf_tuning_apply || warn "kernel network tuning re-apply failed; update itself still succeeded."

log "update complete. Previous binaries/units/helper scripts and config are root-only at $BACKUP_DIR."
