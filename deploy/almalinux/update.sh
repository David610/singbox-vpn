#!/usr/bin/env bash
# Transactional PRODUCTION release-to-release updater.
#
#   sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version v1.1.0
#   sudo /opt/singbox-vpn/deploy/almalinux/update.sh --latest
#   sudo /opt/singbox-vpn/deploy/almalinux/update.sh --repair
#
# A working singbox-vpn release either updates COMPLETELY to the requested
# target release, or returns to the previous working release with
# COMPATIBLE persistent state. It never requires Cargo/Rust for a normal
# production update — a fresh singbox-vpn install using prebuilt release
# binaries stays updatable with no Rust toolchain ever installed.
#
# Transaction phases (see the block comments at each phase below):
#   STAGE -> PREPARE -> SWITCH -> ACTIVATE -> VERIFY -> COMMIT
# Nothing live changes before STAGE has fully verified the target
# release material; nothing is committed (the install-state manifest is
# not updated to the new version) before VERIFY's protocol acceptance
# check passes. Any failure after PREPARE begins triggers rollback_update(),
# which restores every category of file this script can change:
# binaries, systemd units, helper scripts, the sing-box binary (if the
# target release pins a different version), the persistent /opt/singbox-vpn
# source tree, and rendered config — never the authoritative
# users.json/REALITY material, which is re-rendered with the restored
# (old) tooling instead of being rewound.
#
# --repair re-fetches and re-verifies the CURRENTLY recorded release's
# own material and reconciles the host to it (fixes local drift/
# corruption) — it never resolves or switches to a different version.
#
# --dev-rebuild (or SINGBOX_VPN_CHANNEL=dev) is the explicit, clearly-separate
# escape hatch that rebuilds from whatever source is currently checked
# out at /opt/singbox-vpn via Cargo — development/testing only, never implied
# by a normal production update.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_DIR="/usr/local/bin"
SYSTEMD_DIR="/etc/systemd/system"
STATE_DIR_ROOT="/var/lib/singbox-vpn"
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
BACKUP_ROOT="/etc/vpn/backups"
BACKUP_DIR="$BACKUP_ROOT/$(date +%Y%m%d-%H%M%S)-$$"
INSTALL_STATE_MANIFEST="$STATE_DIR_ROOT/install-state.json"
TRANSACTION_MARKER="$STATE_DIR_ROOT/update-transaction.json"
SINGBOX_BIN="$BIN_DIR/sing-box"

# All systemd units install.sh's install_systemd_units() installs — kept
# as the single list both scripts key off conceptually (duplicated by
# necessity, since this is bash, not shared code; if you add a unit to
# one, add it to the other in the same commit — deploy/lib/tests/
# test-install-update-parity.sh fails loudly if these two lists drift).
SYSTEMD_UNITS=(sing-box.service vpn-subscription.service vpn-expiry-reconcile.service vpn-expiry-reconcile.timer vpn-service-watchdog.service vpn-service-watchdog.timer)

log() { echo "[update] $*"; }
warn() { echo "[update] WARNING: $*" >&2; }
die() { echo "[update] ERROR: $*" >&2; exit 1; }

# Deterministic failure injection for the destructive lifecycle-acceptance
# gate ONLY (deploy/almalinux/lifecycle-acceptance.sh). Mirrors install.sh's
# lifecycle_gate_abort_hook(): if SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER matches the
# stage name given, die immediately so rollback_update() (already trap-armed
# via on_exit at this point) fires and can be independently verified. A
# no-op unless the env var is explicitly set.
lifecycle_gate_abort_hook() {
  [ "${SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER:-}" = "$1" ] || return 0
  die "SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=$1 — deliberately aborting for lifecycle-gate testing."
}

[ "$(id -u)" -eq 0 ] || die "must run as root"

# Single authoritative source for COSIGN_VERSION/COSIGN_SHA256_AMD64/
# COSIGN_SHA256_ARM64 — see deploy/lib/versions.env's own header. This is
# the currently-installed source tree's own copy, not the target
# release's (cosign is a verification tool this script depends on, not
# part of what it is updating to).
VERSIONS_ENV="$REPO_ROOT/deploy/lib/versions.env"
[ -f "$VERSIONS_ENV" ] || die "missing $VERSIONS_ENV — cannot resolve pinned cosign version/checksums."
# shellcheck source=/dev/null
. "$VERSIONS_ENV"
for v in COSIGN_VERSION COSIGN_SHA256_AMD64 COSIGN_SHA256_ARM64; do
  [ -n "${!v:-}" ] || die "$v missing from $VERSIONS_ENV."
done

# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/os.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/perf-tuning.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/state-schema.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/binary-version-check.sh"

# Production updates perform the same kinds of GitHub/SagerNet fetches as a
# fresh install. A real lifecycle run reproduced a transient TCP connection
# refusal, which plain `--retry` does not treat as retryable. Keep updater
# downloads independently resilient because this array is defined after
# preflight.sh is sourced and therefore cannot rely on preflight's augmentation.
CURL_NET_FLAGS=(--connect-timeout 10 --max-time 300 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2 --retry-connrefused)

# ---------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------
TARGET_VERSION="${SINGBOX_VPN_VERSION:-}"
SINGBOX_VPN_REPO_OVERRIDE="${SINGBOX_VPN_REPO:-}"
RESOLVE_LATEST=0
REPAIR=0
DEV_REBUILD=0
ALLOW_DOWNGRADE=0
[ "${SINGBOX_VPN_CHANNEL:-}" = "dev" ] && DEV_REBUILD=1

print_update_help() {
  cat <<'USAGE'
singbox-vpn production updater (deploy/almalinux/update.sh).

  sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version v1.1.0
  sudo /opt/singbox-vpn/deploy/almalinux/update.sh --latest
  sudo /opt/singbox-vpn/deploy/almalinux/update.sh --repair

Flags:
  --version vX.Y.Z   update to this exact tagged release (production;
                     no Cargo/Rust required — uses prebuilt release
                     binaries, checksum-verified before any live change).
  --latest           resolve and update to the latest tagged stable
                     release (same verification as --version).
  --repair           reconcile the currently-installed release: re-fetch
                     and re-verify ITS OWN release material and restore
                     it exactly, fixing local drift/corruption. Never
                     resolves or switches to a different version.
  --dev-rebuild      development/testing only: rebuild from whatever
                     source is currently checked out at this path via
                     Cargo (requires a Rust toolchain). Same as
                     SINGBOX_VPN_CHANNEL=dev. Never used for a normal
                     production update.
  --allow-downgrade  permit an intentional downgrade with an explicit
                     --version. Never applies to --latest or implicitly.
  --repo owner/repo  update from a fork (default: the repo this host
                     was originally installed from, if recorded).
  -h, --help         show this message and exit
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) TARGET_VERSION="$2"; shift 2 ;;
    --version=*) TARGET_VERSION="${1#*=}"; shift ;;
    --latest) RESOLVE_LATEST=1; shift ;;
    --repair) REPAIR=1; shift ;;
    --dev-rebuild) DEV_REBUILD=1; shift ;;
    --allow-downgrade) ALLOW_DOWNGRADE=1; shift ;;
    --repo) SINGBOX_VPN_REPO_OVERRIDE="$2"; shift 2 ;;
    --repo=*) SINGBOX_VPN_REPO_OVERRIDE="${1#*=}"; shift ;;
    -h|--help) print_update_help; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

[ -f "$DEPLOYMENT_TOML" ] || die "no existing singbox-vpn deployment found ($DEPLOYMENT_TOML missing) — update.sh only operates on an existing installation. Use install.sh for a fresh install."

# ---------------------------------------------------------------------
# Interrupted-transaction detection (checkpoint-3 requirement): a
# subsequent invocation must detect an incomplete prior update rather
# than blindly starting another one on top of half-swapped state. The
# marker is written in PREPARE, before the first live mutation, and
# removed only on a successful COMMIT or a successful rollback.
# ---------------------------------------------------------------------
if [ -e "$TRANSACTION_MARKER" ]; then
  die "an update transaction did not finish cleanly and left $TRANSACTION_MARKER in place — refusing to start a new update on top of unknown state.
$(cat "$TRANSACTION_MARKER" 2>/dev/null)
Manual recovery: inspect the backup_dir/prev_opt_dir named above. If prev_opt_dir still exists, the source tree was never switched back — 'mv' it to /opt/singbox-vpn after removing whatever is there, restore binaries/units from backup_dir the same way rollback_update() does below, then 'systemctl daemon-reload'. Once the host is confirmed healthy, remove $TRANSACTION_MARKER and retry the update."
fi

# See install.sh's verify_release_attestation() for the full rationale
# (docs/SUPPLY_CHAIN_SECURITY.md): verified with cosign against the public
# Sigstore Rekor transparency log, not `gh attestation verify` — gh
# refuses to run at all without `gh auth login`/GH_TOKEN, even read-only
# against a public repo, which this host has no way to provide. There is
# also no version-gated checksum-only fallback: gating the requirement on
# the release's own version string let an attacker with release-publish
# access (the exact actor attestation is meant to contain) republish an
# old-numbered or malformed tag and skip attestation entirely. Every
# release, historical or not, requires attestation here too.
COSIGN_BIN=""
ensure_cosign() {
  [ -n "$COSIGN_BIN" ] && return 0
  if command -v cosign >/dev/null 2>&1; then
    COSIGN_BIN="$(command -v cosign)"
    return 0
  fi
  local expected_sha256
  case "$ARCH" in
    amd64) expected_sha256="$COSIGN_SHA256_AMD64" ;;
    arm64) expected_sha256="$COSIGN_SHA256_ARM64" ;;
    *) die "unsupported architecture '$ARCH' — cannot verify release attestations." ;;
  esac
  local tmp asset dest
  tmp="$(mktemp -d)"
  asset="cosign-linux-${ARCH}"
  dest="$tmp/$asset"
  log "downloading pinned cosign v${COSIGN_VERSION} (${ARCH}) to verify release attestations..."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$dest" "https://github.com/sigstore/cosign/releases/download/v${COSIGN_VERSION}/${asset}" \
    || die "could not download cosign — required to verify release attestations. Nothing live has changed."
  local actual_sha256
  actual_sha256="$(sha256sum "$dest" | awk '{print $1}')"
  [ "$actual_sha256" = "$expected_sha256" ] \
    || die "checksum verification failed for $asset: expected $expected_sha256, got $actual_sha256 — refusing to run an unverified cosign binary. Nothing live has changed."
  chmod +x "$dest"
  COSIGN_BIN="$dest"
}

verify_release_attestation() {
  local artifact="$1" version="$2" repo="$3"
  ensure_cosign
  local bundle
  bundle="$(mktemp)"
  if ! curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$bundle" "https://github.com/$repo/releases/download/$version/$(basename "$artifact").sigstore.json"; then
    rm -f "$bundle"
    die "could not download the attestation bundle for $(basename "$artifact")/$version. Nothing live has changed. There is no checksum-only fallback for any release, historical or otherwise."
  fi
  if ! "$COSIGN_BIN" verify-blob-attestation \
      --bundle "$bundle" \
      --certificate-identity "https://github.com/$repo/.github/workflows/release.yml@refs/tags/$version" \
      --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
      --type "https://slsa.dev/provenance/v1" \
      "$artifact" >/dev/null; then
    rm -f "$bundle"
    die "artifact attestation verification failed or is missing for $version/$repo. Nothing live has changed. There is no checksum-only fallback for any release, historical or otherwise."
  fi
  rm -f "$bundle"
  log "artifact attestation verified for repository $repo."
}
shopt -s nullglob
for stale in /opt/.singbox-vpn-update-staging.* /opt/.singbox-vpn-prev-*; do
  [ -e "$stale" ] || continue
  die "a stale update-staging/rollback directory exists at $stale from a previous run that did not clean up — refusing to start a new update until it is reviewed and removed manually (it may contain the last known-working release; do not delete it without checking first)."
done
shopt -u nullglob

# Serialize against a concurrent install.sh/update.sh run — same lock
# install.sh uses, extended here rather than inventing a second one.
exec 200>/run/lock/singbox-vpn-installer.lock
flock -x 200

# ---------------------------------------------------------------------
# Current release identity, read from the authoritative install-state
# manifest (never guessed from what happens to be on disk).
# ---------------------------------------------------------------------
manifest_field() {
  grep -o "\"$1\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$INSTALL_STATE_MANIFEST" 2>/dev/null \
    | head -n1 | sed -E 's/.*"([^"]*)"$/\1/'
}
CURRENT_VERSION=""
CURRENT_REPO=""
CURRENT_SINGBOX_PINNED=""
CURRENT_ARCH=""
if [ -f "$INSTALL_STATE_MANIFEST" ]; then
  CURRENT_VERSION="$(manifest_field singbox_vpn_version)"
  CURRENT_REPO="$(manifest_field singbox_vpn_repo)"
  CURRENT_SINGBOX_PINNED="$(manifest_field sing_box_version_pinned)"
  CURRENT_ARCH="$(manifest_field arch)"
fi
SINGBOX_VPN_REPO="${SINGBOX_VPN_REPO_OVERRIDE:-${CURRENT_REPO:-David610/singbox-vpn}}"
ARCH="${CURRENT_ARCH:-$(detect_arch)}" || die "unsupported CPU architecture: $(uname -m)."

# A pre-checkpoint-3 install, or one made with SINGBOX_VPN_CHANNEL=dev, may
# have no pinned version recorded at all ("main", or empty). Neither a
# production --version/--latest update NOR --repair can resolve or
# re-fetch a release that was never actually tagged — fall through to
# the explicit dev-rebuild path instead, with a clear explanation,
# rather than failing with a confusing "release not found".
if [ -z "$CURRENT_VERSION" ] || [ "$CURRENT_VERSION" = "main" ]; then
  if [ "$DEV_REBUILD" -eq 1 ]; then
    : # proceed to dev-rebuild below
  elif [ "$REPAIR" -eq 1 ]; then
    warn "no pinned release version is recorded for this install (singbox_vpn_version='${CURRENT_VERSION:-<none>}') — --repair cannot re-fetch a release that was never tagged. Falling back to a --dev-rebuild-equivalent local reconcile."
    DEV_REBUILD=1
  else
    die "no pinned release version is recorded for this install (singbox_vpn_version='${CURRENT_VERSION:-<none>}') — this host was installed with SINGBOX_VPN_CHANNEL=dev or predates version tracking. A production --version/--latest update needs a known starting release. Use --dev-rebuild to rebuild from the currently checked-out source instead, or reinstall with a pinned --version first."
  fi
fi

# =======================================================================
# DEV-REBUILD PATH — explicit escape hatch only, requires Cargo, rebuilds
# from whatever source is already checked out at $REPO_ROOT. Kept
# structurally close to the original (pre-checkpoint-3) update.sh: same
# backup/rollback shape for binaries/units/helper scripts, since that
# part was already correct — the defect this checkpoint fixes is that
# THIS used to be the only path, including for ordinary production
# updates, which is exactly what --dev-rebuild now scopes it to.
# =======================================================================
if [ "$DEV_REBUILD" -eq 1 ]; then
  log "DEV-REBUILD mode (SINGBOX_VPN_CHANNEL=dev / --dev-rebuild): rebuilding from the source currently at $REPO_ROOT via Cargo. This is NOT the production update path — see --help."
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
  command -v cargo >/dev/null 2>&1 \
    || die "cargo not found. --dev-rebuild requires a Rust toolchain. (A normal production update does not need this — use --version/--latest instead.)"

  log "running tests before touching installed state..."
  ( cd "$REPO_ROOT" && cargo test --workspace --locked -p admin -p subscription -p compat-config ) \
    || die "tests failed; installed state was not changed."
  log "building new binaries..."
  # --locked matches every CI build/test job: without it, this could
  # silently resolve a different dependency set than the one committed
  # Cargo.lock records and cargo audit gates in CI.
  ( cd "$REPO_ROOT" && cargo build --release --locked -p admin -p subscription )

  install -d -m 0700 -o root -g root "$BACKUP_ROOT"
  install -d -m 0700 -o root -g root "$BACKUP_DIR"
  install -d -m 0700 -o root -g root "$BACKUP_DIR/systemd"
  for f in vpn-admin vpn vpn-subscription-svc; do
    [ -f "$BIN_DIR/$f" ] || die "installed binary $BIN_DIR/$f is missing; refusing a non-recoverable update"
    cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
  done
  for u in "${SYSTEMD_UNITS[@]}"; do
    [ -f "$SYSTEMD_DIR/$u" ] || die "installed systemd unit $SYSTEMD_DIR/$u is missing; refusing a non-recoverable update"
    cp -a "$SYSTEMD_DIR/$u" "$BACKUP_DIR/systemd/$u"
  done
  for f in vpn-health-check vpn-benchmark vpn-benchmark-lib.sh vpn-service-watchdog; do
    [ -f "$BIN_DIR/$f" ] && cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
  done

  install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin.update-new"
  install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn.update-new"
  install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc.update-new"
  for u in "${SYSTEMD_UNITS[@]}"; do
    install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/$u" "$SYSTEMD_DIR/$u.update-new"
  done
  install -m 0755 "$REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check.update-new"
  install -m 0755 "$REPO_ROOT/deploy/lib/vpn-benchmark.sh" "$BIN_DIR/vpn-benchmark.update-new"
  install -m 0644 "$REPO_ROOT/deploy/lib/vpn-benchmark-lib.sh" "$BIN_DIR/vpn-benchmark-lib.sh.update-new"
  install -m 0755 "$REPO_ROOT/deploy/almalinux/service-watchdog.sh" "$BIN_DIR/vpn-service-watchdog.update-new"

  exec 201>/run/lock/singbox-vpn.lock
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
    for f in vpn-health-check vpn-benchmark vpn-benchmark-lib.sh vpn-service-watchdog; do
      if [ -f "$BACKUP_DIR/$f" ]; then
        install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f.rollback" || failed=1
        mv -f "$BIN_DIR/$f.rollback" "$BIN_DIR/$f" || failed=1
      else
        rm -f "$BIN_DIR/$f"
      fi
      rm -f "$BIN_DIR/$f.update-new"
    done

    if ! SINGBOX_VPN_LOCK_PATH="$BACKUP_DIR/rollback-inner.lock" \
        "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config; then
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
      echo "[update] UPDATE FAILED — PREVIOUS RELEASE RESTORED AND VERIFIED (dev-rebuild)." >&2
      exit 1
    fi
    echo "[update] UPDATE FAILED — ROLLBACK ALSO FAILED" >&2
    echo "[update] MANUAL INTERVENTION REQUIRED. Backups: $BACKUP_DIR" >&2
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
  mv -f "$BIN_DIR/vpn-service-watchdog.update-new" "$BIN_DIR/vpn-service-watchdog"

  log "reloading systemd unit definitions..."
  systemctl daemon-reload

  log "install mode: UPGRADE (dev-rebuild) — checking persistent state schema before rendering..."
  schema_rc=0
  state_schema_validate_and_migrate "$BIN_DIR/vpn-admin" "$DEPLOYMENT_TOML" || schema_rc=$?
  case "$schema_rc" in
    0 | 2) ;;
    3)
      die "persistent state migration failed — see output above. Rolling back to the previous working binaries/config; nothing live was changed by this step."
      ;;
    *)
      die "persistent state is INVALID/unsupported — see output above. Rolling back to the previous working binaries/config."
      ;;
  esac

  log "rendering current authoritative users/REALITY state with new tooling..."
  SINGBOX_VPN_LOCK_PATH="$BACKUP_DIR/update-inner.lock" \
    "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config

  log "restarting services..."
  systemctl restart vpn-subscription
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

  log "verifying real protocol handshake..."
  doctor_output=""
  doctor_rc=0
  doctor_output="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" doctor --protocol --require-protocol 2>&1)" || doctor_rc=$?
  if [ "$doctor_rc" -ne 0 ]; then
    echo "$doctor_output" >&2
    die "post-update protocol acceptance check failed (status $doctor_rc) — see output above. Rolling back to the previous working binaries/config."
  fi

  committed=1
  trap - ERR INT TERM EXIT
  rm -rf "$BACKUP_DIR"
  perf_tuning_apply || warn "kernel network tuning re-apply failed; update itself still succeeded."
  log "dev-rebuild update complete."
  exit 0
fi

# =======================================================================
# PRODUCTION PATH — resolves one immutable target release, verifies its
# material before any live mutation, stages it fully, then runs the
# real STAGE -> PREPARE -> SWITCH -> ACTIVATE -> VERIFY -> COMMIT
# transaction. No Cargo required anywhere in this path.
# =======================================================================

# ---- resolve exactly one target version, before touching anything ----
if [ "$REPAIR" -eq 1 ]; then
  [ -z "$TARGET_VERSION" ] || die "--repair reconciles the CURRENTLY installed release ($CURRENT_VERSION) — it does not take --version/--latest. Run the production update (--version/--latest) to change version, then --repair separately if still needed."
  [ "$RESOLVE_LATEST" -eq 0 ] || die "--repair and --latest are mutually exclusive."
  TARGET_VERSION="$CURRENT_VERSION"
  log "REPAIR mode: reconciling the currently-installed release $TARGET_VERSION (re-fetching and re-verifying its own release material; never switching version)."
elif [ "$RESOLVE_LATEST" -eq 1 ]; then
  [ -z "$TARGET_VERSION" ] || die "--latest and --version are mutually exclusive."
  log "resolving latest stable release tag for $SINGBOX_VPN_REPO..."
  latest_tag="$(curl -fsSL "${CURL_NET_FLAGS[@]}" \
      "https://api.github.com/repos/$SINGBOX_VPN_REPO/releases/latest" 2>/dev/null \
      | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name" *: *"([^"]*)".*/\1/')" || true
  [ -n "$latest_tag" ] || die "could not resolve the latest stable release for $SINGBOX_VPN_REPO (no tagged release found, or the GitHub API request failed)."
  TARGET_VERSION="$latest_tag"
  log "resolved latest stable release: $TARGET_VERSION"
elif [ -n "$TARGET_VERSION" ]; then
  :
else
  die "a production update requires an explicit target: --version vX.Y.Z, --latest, or --repair (reconcile the current release). See --help."
fi

echo "$TARGET_VERSION" | grep -Eq '^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.]+)?$' \
  || die "TARGET_VERSION '$TARGET_VERSION' does not look like a valid release tag (expected vX.Y.Z) — refusing to use it."

if [ "$ALLOW_DOWNGRADE" -eq 1 ] && { [ "$RESOLVE_LATEST" -eq 1 ] || [ "$REPAIR" -eq 1 ]; }; then
  die "--allow-downgrade is valid only with an explicit --version target."
fi
version_is_older() {
  local candidate="${1#v}" current="${2#v}" first
  local candidate_base="${candidate%%-*}" current_base="${current%%-*}"
  local candidate_pre="" current_pre=""
  [ "$candidate" != "$current" ] || return 1
  [[ "$candidate" == *-* ]] && candidate_pre="${candidate#*-}"
  [[ "$current" == *-* ]] && current_pre="${current#*-}"
  if [ "$candidate_base" != "$current_base" ]; then
    first="$(printf '%s\n%s\n' "$candidate_base" "$current_base" | sort -V | head -n1)"
    [ "$first" = "$candidate_base" ]
    return
  fi
  # SemVer: a prerelease is older than the corresponding final release.
  [ -n "$candidate_pre" ] && [ -z "$current_pre" ] && return 0
  [ -z "$candidate_pre" ] && [ -n "$current_pre" ] && return 1
  first="$(printf '%s\n%s\n' "$candidate_pre" "$current_pre" | sort -V | head -n1)"
  [ "$first" = "$candidate_pre" ]
}
if version_is_older "$TARGET_VERSION" "$CURRENT_VERSION"; then
  if [ "$ALLOW_DOWNGRADE" -ne 1 ]; then
    die "refusing unintended downgrade $CURRENT_VERSION -> $TARGET_VERSION. Re-run with the explicit target and --allow-downgrade only after reviewing state-schema compatibility."
  fi
  warn "EXPLICIT DOWNGRADE AUTHORIZED: $CURRENT_VERSION -> $TARGET_VERSION. Rollback remains transactional, but older code may not understand newer persistent state."
fi

if [ "$TARGET_VERSION" = "$CURRENT_VERSION" ] && [ "$REPAIR" -ne 1 ]; then
  log "Already at $CURRENT_VERSION — nothing to update. Pass --repair to reconcile this release's material without changing version."
  exit 0
fi

log "current release: ${CURRENT_VERSION:-<unknown>} ($CURRENT_REPO) -> target release: $TARGET_VERSION ($SINGBOX_VPN_REPO)"

# ---- STAGE: download + verify the target release's source archive ----
# Reuses the exact same trust model as install.sh's bootstrap
# (download_verified_source_release()): the source archive is
# checksum-verified against a SHA256SUMS manifest published by
# .github/workflows/release.yml for that exact tag before anything is
# extracted, let alone executed. Staged on the SAME filesystem as
# /opt/singbox-vpn (under /opt) so the eventual SWITCH is an atomic rename, not
# a cross-filesystem copy.
STAGING_ROOT="$(mktemp -d /opt/.singbox-vpn-update-staging.XXXXXX)" || die "mktemp failed"
STAGE_OK=0
cleanup_staging() {
  [ "$STAGE_OK" -eq 1 ] && return
  rm -rf "$STAGING_ROOT"
}
trap cleanup_staging EXIT

download_verified_source_release() {
  local version="$1" tarball="$2"
  local base_url="https://github.com/$SINGBOX_VPN_REPO/releases/download/$version"
  local sums="$STAGING_ROOT/SHA256SUMS"
  log "downloading singbox-vpn $version release source archive + checksum manifest..."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tarball" "$base_url/singbox-vpn-src.tar.gz" \
    || die "could not download release source archive 'singbox-vpn-src.tar.gz' for $version from $SINGBOX_VPN_REPO. Nothing live has been changed."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$sums" "$base_url/SHA256SUMS" \
    || die "release $version was found but its SHA256SUMS checksum manifest could not be downloaded — refusing to use an unverified source archive. Nothing live has been changed."
  grep -qE '^[0-9a-f]{64}  singbox-vpn-src\.tar\.gz$' "$sums" \
    || die "SHA256SUMS for $version has no well-formed entry for singbox-vpn-src.tar.gz — refusing to use an unverified source archive. Nothing live has been changed."
  ( cd "$STAGING_ROOT" && grep -E '  singbox-vpn-src\.tar\.gz$' SHA256SUMS | sha256sum -c - ) \
    || die "checksum verification failed for singbox-vpn-src.tar.gz against $version's published SHA256SUMS. Nothing live has been changed."
  log "source archive checksum verified against release SHA256SUMS."
  verify_release_attestation "$tarball" "$version" "$SINGBOX_VPN_REPO"
}

download_verified_source_release "$TARGET_VERSION" "$STAGING_ROOT/singbox-vpn-src.tar.gz"
tar -xzf "$STAGING_ROOT/singbox-vpn-src.tar.gz" -C "$STAGING_ROOT" || die "failed to extract downloaded source archive. Nothing live has been changed."
STAGED_SRC_DIR="$(find "$STAGING_ROOT" -mindepth 1 -maxdepth 1 -type d ! -name '*.tar.gz' | head -n1)"
[ -n "$STAGED_SRC_DIR" ] && [ -x "$STAGED_SRC_DIR/deploy/almalinux/install.sh" ] \
  || die "downloaded release source for $TARGET_VERSION does not look like a valid singbox-vpn source tree. Nothing live has been changed."
[ -f "$STAGED_SRC_DIR/deploy/lib/versions.env" ] \
  || die "downloaded release source for $TARGET_VERSION is missing deploy/lib/versions.env. Nothing live has been changed."
expected_package_version="${TARGET_VERSION#v}"
expected_package_version="${expected_package_version%%-*}"
# apps/admin/Cargo.toml declares version.workspace = true (no literal
# version string of its own — see deploy/lib/check-workspace-version-
# consistency.sh), so the authoritative version lives only in the root
# Cargo.toml's [workspace.package] section.
source_package_version="$(awk '
  /^\[workspace\.package\]/ { insec=1; next }
  /^\[/ { insec=0 }
  insec && /^version[[:space:]]*=/ { print; exit }
' "$STAGED_SRC_DIR/Cargo.toml" 2>/dev/null | sed -nE 's/^version[[:space:]]*=[[:space:]]*"([0-9]+\.[0-9]+\.[0-9]+)"[[:space:]]*$/\1/p')"
[ "$source_package_version" = "$expected_package_version" ] \
  || die "authenticated source archive version '$source_package_version' does not match requested release '$TARGET_VERSION'. Nothing live has been changed."

# ---- read the TARGET release's own pinned sing-box version — never
# mix this release's templates/units with a different release's
# sing-box pin (grep, not source, to avoid clobbering this script's own
# already-sourced current-repo variables of the same name). ----
TARGET_SINGBOX_VERSION="$(grep -E '^SINGBOX_VERSION=' "$STAGED_SRC_DIR/deploy/lib/versions.env" | cut -d= -f2)"
TARGET_SINGBOX_SHA256_AMD64="$(grep -E '^SINGBOX_SHA256_AMD64=' "$STAGED_SRC_DIR/deploy/lib/versions.env" | cut -d= -f2)"
TARGET_SINGBOX_SHA256_ARM64="$(grep -E '^SINGBOX_SHA256_ARM64=' "$STAGED_SRC_DIR/deploy/lib/versions.env" | cut -d= -f2)"
[ -n "$TARGET_SINGBOX_VERSION" ] || die "target release $TARGET_VERSION's versions.env has no SINGBOX_VERSION — refusing an inconsistent release. Nothing live has been changed."

TARGET_RUST_TARGET="$(rust_target_for_arch "$ARCH")" || die "unsupported architecture: $ARCH. Nothing live has been changed."

# ---- STAGE: download + verify the target release's prebuilt singbox-vpn
# binaries. Production NEVER falls back to Cargo here — see --help. ----
STAGED_BIN_DIR="$STAGING_ROOT/bin"
install -d -m 0700 "$STAGED_BIN_DIR"
stage_prebuilt_binaries() {
  local base_url="https://github.com/$SINGBOX_VPN_REPO/releases/download/$TARGET_VERSION"
  local asset="singbox-vpn-${TARGET_RUST_TARGET}.tar.gz"
  log "checking for prebuilt release binaries ($asset)..."
  if ! curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$STAGING_ROOT/$asset" "$base_url/$asset" 2>/dev/null; then
    return 1
  fi
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$STAGING_ROOT/BIN_SHA256SUMS" "$base_url/SHA256SUMS" 2>/dev/null \
    || die "release binary asset $asset was found but SHA256SUMS was not — refusing to use an unverified binary. Nothing live has been changed."
  ( cd "$STAGING_ROOT" && grep -E "  ${asset}\$" BIN_SHA256SUMS | sha256sum -c - ) \
    || die "checksum verification failed for $asset against $TARGET_VERSION's published SHA256SUMS. Nothing live has been changed."
  verify_release_attestation "$STAGING_ROOT/$asset" "$TARGET_VERSION" "$SINGBOX_VPN_REPO"
  tar -xzf "$STAGING_ROOT/$asset" -C "$STAGING_ROOT"
  local extracted="$STAGING_ROOT/singbox-vpn-${TARGET_RUST_TARGET}"
  [ -d "$extracted" ] || die "release asset $asset did not contain the expected singbox-vpn-${TARGET_RUST_TARGET}/ directory — packaging bug, not a transient failure. Nothing live has been changed."
  install -m 0755 "$extracted/vpn-admin" "$STAGED_BIN_DIR/vpn-admin"
  install -m 0755 "$extracted/subscription" "$STAGED_BIN_DIR/vpn-subscription-svc"
  log "staged prebuilt singbox-vpn $TARGET_VERSION binaries ($TARGET_RUST_TARGET) — no Rust compiler needed."
  return 0
}
if ! stage_prebuilt_binaries; then
  die "no prebuilt binaries are available for $TARGET_VERSION/$TARGET_RUST_TARGET — a production update requires prebuilt release assets and never silently falls back to a source build. Nothing live has been changed. Use --dev-rebuild (requires Cargo) if you specifically intend a source-tree rebuild instead of an ordinary update."
fi
# Validation that the staged binaries actually run before either is
# ever treated as trustworthy (checkpoint-3 requirement #7.5). See
# deploy/lib/binary-version-check.sh: distinguishes "cannot execute at
# all" (e.g. a GLIBC/ABI mismatch between this release and this host)
# from "executes but wrong version" instead of the old `--help
# >/dev/null 2>&1` check (which proved nothing beyond exit status) plus
# a separate `2>/dev/null`-masked version comparison. Checks both
# shipped binaries, not just vpn-admin.
staged_version_context="for $TARGET_VERSION. Nothing live has been changed."
check_binary_version "$STAGED_BIN_DIR/vpn-admin" "$expected_package_version" "vpn-admin" "$staged_version_context"
check_binary_version "$STAGED_BIN_DIR/vpn-subscription-svc" "$expected_package_version" "subscription" "$staged_version_context"

# ---- STAGE: sing-box, only if the target release pins a different
# version than what's currently installed. Same checksum-verification
# pattern as install.sh's install_singbox(). ----
STAGED_SINGBOX_BIN=""
if [ "$TARGET_SINGBOX_VERSION" != "$CURRENT_SINGBOX_PINNED" ]; then
  log "target release pins sing-box $TARGET_SINGBOX_VERSION (currently ${CURRENT_SINGBOX_PINNED:-unknown}) — staging it..."
  singbox_tarball="sing-box-${TARGET_SINGBOX_VERSION}-linux-${ARCH}.tar.gz"
  singbox_url="https://github.com/SagerNet/sing-box/releases/download/v${TARGET_SINGBOX_VERSION}/${singbox_tarball}"
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$STAGING_ROOT/$singbox_tarball" "$singbox_url" \
    || die "download failed: $singbox_url. Nothing live has been changed."
  sums_url="https://github.com/SagerNet/sing-box/releases/download/v${TARGET_SINGBOX_VERSION}/sing-box_${TARGET_SINGBOX_VERSION}_checksums.txt"
  if curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$STAGING_ROOT/singbox-checksums.txt" "$sums_url" 2>/dev/null; then
    ( cd "$STAGING_ROOT" && sha256sum --ignore-missing -c singbox-checksums.txt ) \
      || die "checksum verification failed for $singbox_tarball (upstream checksums.txt). Nothing live has been changed."
  else
    expected_sha256=""
    case "$ARCH" in
      amd64) expected_sha256="$TARGET_SINGBOX_SHA256_AMD64" ;;
      arm64) expected_sha256="$TARGET_SINGBOX_SHA256_ARM64" ;;
    esac
    [ -n "$expected_sha256" ] || die "no upstream checksums.txt and no pinned expected SHA256 for sing-box ${TARGET_SINGBOX_VERSION}/${ARCH} in the target release. Nothing live has been changed."
    actual_sha256="$(sha256sum "$STAGING_ROOT/$singbox_tarball" | awk '{print $1}')"
    [ "$actual_sha256" = "$expected_sha256" ] \
      || die "checksum verification failed for $singbox_tarball: expected $expected_sha256, got $actual_sha256. Nothing live has been changed."
  fi
  tar -xzf "$STAGING_ROOT/$singbox_tarball" -C "$STAGING_ROOT"
  extracted_singbox="$STAGING_ROOT/sing-box-${TARGET_SINGBOX_VERSION}-linux-${ARCH}"
  [ -f "$extracted_singbox/sing-box" ] || die "sing-box release asset did not contain the expected binary. Nothing live has been changed."
  install -m 0755 "$extracted_singbox/sing-box" "$STAGED_BIN_DIR/sing-box"
  [ -f "$extracted_singbox/LICENSE" ] && install -m 0644 "$extracted_singbox/LICENSE" "$STAGED_BIN_DIR/sing-box.LICENSE"
  "$STAGED_BIN_DIR/sing-box" version 2>/dev/null | grep -q "$TARGET_SINGBOX_VERSION" \
    || die "staged sing-box binary does not report version $TARGET_SINGBOX_VERSION — refusing to install it. Nothing live has been changed."
  STAGED_SINGBOX_BIN="$STAGED_BIN_DIR/sing-box"
else
  log "target release pins the same sing-box version ($TARGET_SINGBOX_VERSION) already installed — no sing-box change needed."
fi

STAGE_OK=1
trap - EXIT
log "STAGE complete: $TARGET_VERSION verified and ready (source + binaries$([ -n "$STAGED_SINGBOX_BIN" ] && echo " + sing-box"))."

# ---- STAGE: reject an impossible state-schema transition before ANY
# live mutation, not just before it's too late. The check_state_schema
# block further below (after SWITCH) already runs `vpn-admin config
# validate`/`config migrate` using the NEW binary against the live
# state -- which correctly refuses a schema the new binary can't read
# (e.g. an unintended downgrade past a schema bump), but only AFTER
# SWITCH has already replaced the running binaries/units, relying on
# this script's rollback path to recover. Running the exact same
# read-only `config validate` here, against the STAGED (not yet
# installed) target binary, catches the identical incompatibility
# before touching anything live at all -- the rollback path stays as
# defense in depth, not the primary safety mechanism, for this specific
# failure. A clean, existing deployment.toml is required for this
# check to mean anything; a repair with no prior deployment has nothing
# to validate yet.
if [ -f "$DEPLOYMENT_TOML" ]; then
  precheck_rc=0
  precheck_output="$("$STAGED_BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" config validate 2>&1)" || precheck_rc=$?
  case "$precheck_rc" in
    0 | 2)
      log "pre-switch schema compatibility check: $TARGET_VERSION's vpn-admin can read the current persistent state (status $precheck_rc)."
      ;;
    *)
      echo "$precheck_output" >&2
      die "$TARGET_VERSION's vpn-admin cannot read the current persistent state (status $precheck_rc) -- this update (or downgrade) would leave singbox-vpn unable to start. Nothing live has been changed. See output above."
      ;;
  esac
fi

# =======================================================================
# PREPARE — snapshot everything this transaction may change, write the
# transaction marker (enables interrupted-transaction detection above),
# THEN begin the only section of this script allowed to mutate live
# state.
# =======================================================================
install -d -m 0700 -o root -g root "$BACKUP_ROOT"
install -d -m 0700 -o root -g root "$BACKUP_DIR"
install -d -m 0700 -o root -g root "$BACKUP_DIR/systemd"
for f in vpn-admin vpn vpn-subscription-svc; do
  [ -f "$BIN_DIR/$f" ] || die "installed binary $BIN_DIR/$f is missing; refusing a non-recoverable update. Nothing live has been changed."
  cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done
for u in "${SYSTEMD_UNITS[@]}"; do
  [ -f "$SYSTEMD_DIR/$u" ] || die "installed systemd unit $SYSTEMD_DIR/$u is missing; refusing a non-recoverable update. Nothing live has been changed."
  cp -a "$SYSTEMD_DIR/$u" "$BACKUP_DIR/systemd/$u"
done
for f in vpn-health-check vpn-benchmark vpn-benchmark-lib.sh vpn-service-watchdog; do
  [ -f "$BIN_DIR/$f" ] && cp -a "$BIN_DIR/$f" "$BACKUP_DIR/$f"
done
if [ -n "$STAGED_SINGBOX_BIN" ]; then
  [ -f "$SINGBOX_BIN" ] && cp -a "$SINGBOX_BIN" "$BACKUP_DIR/sing-box"
  [ -f "$BIN_DIR/sing-box.LICENSE" ] && cp -a "$BIN_DIR/sing-box.LICENSE" "$BACKUP_DIR/sing-box.LICENSE"
fi
cp -a "$INSTALL_STATE_MANIFEST" "$BACKUP_DIR/install-state.json.bak" 2>/dev/null || true

PREV_OPT_DIR="/opt/.singbox-vpn-prev-$$"

cat > "$TRANSACTION_MARKER.tmp" <<EOF
{
  "from_version": "${CURRENT_VERSION:-unknown}",
  "to_version": "$TARGET_VERSION",
  "backup_dir": "$BACKUP_DIR",
  "prev_opt_dir": "$PREV_OPT_DIR",
  "staged_src_dir": "$STAGED_SRC_DIR",
  "started_at_unix": $(date +%s)
}
EOF
chmod 0600 "$TRANSACTION_MARKER.tmp"
mv -f "$TRANSACTION_MARKER.tmp" "$TRANSACTION_MARKER"

# Block user mutations/key rotation/restore for the update's commit
# window (same lock vpn-admin's own commands already use — extended
# here, not a second locking mechanism).
exec 201>/run/lock/singbox-vpn.lock
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
  warn "update to $TARGET_VERSION did not commit; restoring previous release (${CURRENT_VERSION:-unknown}) from $BACKUP_DIR"

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
  for f in vpn-health-check vpn-benchmark vpn-benchmark-lib.sh vpn-service-watchdog; do
    if [ -f "$BACKUP_DIR/$f" ]; then
      install -m 0755 "$BACKUP_DIR/$f" "$BIN_DIR/$f.rollback" || failed=1
      mv -f "$BIN_DIR/$f.rollback" "$BIN_DIR/$f" || failed=1
    else
      rm -f "$BIN_DIR/$f"
    fi
    rm -f "$BIN_DIR/$f.update-new"
  done
  if [ -f "$BACKUP_DIR/sing-box" ]; then
    install -m 0755 "$BACKUP_DIR/sing-box" "$SINGBOX_BIN.rollback" || failed=1
    mv -f "$SINGBOX_BIN.rollback" "$SINGBOX_BIN" || failed=1
    [ -f "$BACKUP_DIR/sing-box.LICENSE" ] && cp -a "$BACKUP_DIR/sing-box.LICENSE" "$BIN_DIR/sing-box.LICENSE"
  fi

  # Restore the previous /opt/singbox-vpn source tree if the SWITCH phase ever
  # renamed it away — the source of truth for templates/units/scripts
  # must go back to matching the restored binaries exactly.
  if [ -d "$PREV_OPT_DIR" ]; then
    rm -rf /opt/singbox-vpn.rollback-failed 2>/dev/null || true
    if [ -d /opt/singbox-vpn ]; then mv -f /opt/singbox-vpn /opt/singbox-vpn.rollback-failed || failed=1; fi
    mv -f "$PREV_OPT_DIR" /opt/singbox-vpn || failed=1
    rm -rf /opt/singbox-vpn.rollback-failed 2>/dev/null || true
  fi

  systemctl daemon-reload || failed=1

  # Never rewind users.json or REALITY material — authoritative, may
  # have changed while staging/download ran. Render with the restored
  # (old) tooling instead.
  if ! SINGBOX_VPN_LOCK_PATH="$BACKUP_DIR/rollback-inner.lock" \
      "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config; then
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

  # install-state.json is never rewritten until COMMIT, so nothing to
  # restore there under normal operation — but a corrupted/short write
  # is defended against anyway (belt and suspenders).
  if [ -f "$BACKUP_DIR/install-state.json.bak" ] && ! cmp -s "$BACKUP_DIR/install-state.json.bak" "$INSTALL_STATE_MANIFEST" 2>/dev/null; then
    cp -a "$BACKUP_DIR/install-state.json.bak" "$INSTALL_STATE_MANIFEST" || failed=1
  fi

  rm -f "$TRANSACTION_MARKER"
  rm -rf "$STAGING_ROOT"

  if [ "$failed" -eq 0 ]; then
    echo "[update] UPDATE FAILED — PREVIOUS RELEASE RESTORED AND VERIFIED (${CURRENT_VERSION:-unknown})." >&2
    exit 1
  fi
  echo "[update] UPDATE FAILED — ROLLBACK ALSO FAILED" >&2
  echo "[update] MANUAL INTERVENTION REQUIRED. Backups: $BACKUP_DIR ; transaction marker left at $TRANSACTION_MARKER for forensics." >&2
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

# =======================================================================
# SWITCH — install the staged, verified target release. This is the
# first section allowed to change live state.
# =======================================================================
log "SWITCHING to $TARGET_VERSION..."
mutation_started=1

mv -f /opt/singbox-vpn "$PREV_OPT_DIR"
mv -f "$STAGED_SRC_DIR" /opt/singbox-vpn
NEW_REPO_ROOT="/opt/singbox-vpn"

install -m 0755 "$STAGED_BIN_DIR/vpn-admin" "$BIN_DIR/vpn-admin.update-new"
install -m 0755 "$STAGED_BIN_DIR/vpn-admin" "$BIN_DIR/vpn.update-new"
install -m 0755 "$STAGED_BIN_DIR/vpn-subscription-svc" "$BIN_DIR/vpn-subscription-svc.update-new"
for u in "${SYSTEMD_UNITS[@]}"; do
  install -m 0644 "$NEW_REPO_ROOT/deploy/almalinux/systemd/$u" "$SYSTEMD_DIR/$u.update-new"
done
install -m 0755 "$NEW_REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check.update-new"
install -m 0755 "$NEW_REPO_ROOT/deploy/lib/vpn-benchmark.sh" "$BIN_DIR/vpn-benchmark.update-new"
install -m 0644 "$NEW_REPO_ROOT/deploy/lib/vpn-benchmark-lib.sh" "$BIN_DIR/vpn-benchmark-lib.sh.update-new"
install -m 0755 "$NEW_REPO_ROOT/deploy/almalinux/service-watchdog.sh" "$BIN_DIR/vpn-service-watchdog.update-new"

mv -f "$BIN_DIR/vpn-admin.update-new" "$BIN_DIR/vpn-admin"
mv -f "$BIN_DIR/vpn.update-new" "$BIN_DIR/vpn"
mv -f "$BIN_DIR/vpn-subscription-svc.update-new" "$BIN_DIR/vpn-subscription-svc"
for u in "${SYSTEMD_UNITS[@]}"; do
  mv -f "$SYSTEMD_DIR/$u.update-new" "$SYSTEMD_DIR/$u"
done
mv -f "$BIN_DIR/vpn-health-check.update-new" "$BIN_DIR/vpn-health-check"
mv -f "$BIN_DIR/vpn-benchmark.update-new" "$BIN_DIR/vpn-benchmark"
mv -f "$BIN_DIR/vpn-benchmark-lib.sh.update-new" "$BIN_DIR/vpn-benchmark-lib.sh"
mv -f "$BIN_DIR/vpn-service-watchdog.update-new" "$BIN_DIR/vpn-service-watchdog"

singbox_binary_changed=0
if [ -n "$STAGED_SINGBOX_BIN" ]; then
  install -m 0755 "$STAGED_SINGBOX_BIN" "$SINGBOX_BIN.update-new"
  mv -f "$SINGBOX_BIN.update-new" "$SINGBOX_BIN"
  [ -f "$STAGED_BIN_DIR/sing-box.LICENSE" ] && install -m 0644 "$STAGED_BIN_DIR/sing-box.LICENSE" "$BIN_DIR/sing-box.LICENSE"
  singbox_binary_changed=1
  "$SINGBOX_BIN" version || die "installed sing-box binary failed to run after SWITCH."
fi

# =======================================================================
# ACTIVATE
# =======================================================================
log "reloading systemd unit definitions..."
systemctl daemon-reload

log "install mode: UPDATE ${CURRENT_VERSION:-unknown} -> $TARGET_VERSION — checking persistent state schema before rendering..."
schema_rc=0
state_schema_validate_and_migrate "$BIN_DIR/vpn-admin" "$DEPLOYMENT_TOML" || schema_rc=$?
case "$schema_rc" in
  0) ;;
  2)
    lifecycle_gate_abort_hook after_migration
    ;;
  3)
    die "persistent state migration failed — see output above. Rolling back to $CURRENT_VERSION; nothing about this failure changes credentials."
    ;;
  *)
    die "persistent state is INVALID/unsupported — see output above. Rolling back to $CURRENT_VERSION."
    ;;
esac
lifecycle_gate_abort_hook after_switch

log "rendering current authoritative users/REALITY state with new tooling (credentials are never rotated by an update)..."
SINGBOX_VPN_LOCK_PATH="$BACKUP_DIR/update-inner.lock" \
  "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config

log "restarting services..."
systemctl restart vpn-subscription

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
if [ "$singbox_binary_changed" -eq 1 ]; then
  log "sing-box binary changed ($CURRENT_SINGBOX_PINNED -> $TARGET_SINGBOX_VERSION); restarting to run the new binary..."
  systemctl restart sing-box
elif [ "$singbox_config_changed" -eq 0 ] && [ "$singbox_unit_changed" -eq 1 ]; then
  log "sing-box.service unit file changed (config/binary did not); restarting to pick it up..."
  systemctl reload-or-restart sing-box
elif [ "$singbox_config_changed" -eq 1 ]; then
  log "sing-box config changed; render-config above already reloaded/restarted it."
else
  log "sing-box config, unit file, and binary unchanged; not restarting (keeps live connections up)."
fi

# =======================================================================
# VERIFY
# =======================================================================
log "running health check..."
/usr/local/bin/vpn-health-check

log "verifying real protocol handshake..."
doctor_output=""
doctor_rc=0
doctor_output="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" doctor --protocol --require-protocol 2>&1)" || doctor_rc=$?
if [ "$doctor_rc" -ne 0 ]; then
  echo "$doctor_output" >&2
  die "post-update protocol acceptance check failed (status $doctor_rc) — see output above. Rolling back to $CURRENT_VERSION."
fi

# =======================================================================
# COMMIT — only now is the authoritative version manifest updated. A
# failure at ANY point above this line rolls back and NEVER reaches here.
# =======================================================================
log "writing install-state manifest for $TARGET_VERSION..."
singbox_version_reported="$("$SINGBOX_BIN" version 2>/dev/null | head -n1 | sed 's/"/\\"/g' || echo unknown)"
pinned_singbox_sha256=""
case "$ARCH" in
  amd64) pinned_singbox_sha256="$TARGET_SINGBOX_SHA256_AMD64" ;;
  arm64) pinned_singbox_sha256="$TARGET_SINGBOX_SHA256_ARM64" ;;
esac
prior_manifest="$(cat "$INSTALL_STATE_MANIFEST" 2>/dev/null || echo '{}')"
public_host="$(echo "$prior_manifest" | grep -o '"public_host"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"([^"]*)"$/\1/')"
subscription_host="$(echo "$prior_manifest" | grep -o '"subscription_host"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"([^"]*)"$/\1/')"
firewall_backend="$(echo "$prior_manifest" | grep -o '"firewall_backend"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"([^"]*)"$/\1/')"
os_family="$(echo "$prior_manifest" | grep -o '"os_family"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"([^"]*)"$/\1/')"
cat > "$INSTALL_STATE_MANIFEST.tmp" <<EOF
{
  "singbox_vpn_version": "$TARGET_VERSION",
  "singbox_vpn_repo": "$SINGBOX_VPN_REPO",
  "sing_box_version": "$singbox_version_reported",
  "sing_box_version_pinned": "$TARGET_SINGBOX_VERSION",
  "sing_box_sha256_pinned": "$pinned_singbox_sha256",
  "arch": "$ARCH",
  "installed_at_unix": $(date +%s),
  "public_host": "$public_host",
  "subscription_host": "$subscription_host",
  "firewall_backend": "$firewall_backend",
  "os_family": "$os_family",
  "repo_root": "$NEW_REPO_ROOT",
  "acceptance": "accepted"
}
EOF
chmod 0644 "$INSTALL_STATE_MANIFEST.tmp"
mv -f "$INSTALL_STATE_MANIFEST.tmp" "$INSTALL_STATE_MANIFEST"

committed=1
trap - ERR INT TERM EXIT

# Transaction-only backups exist only to restore the previous release if
# this update failed — remove them now that it did not (checkpoint-3
# requirement: never turn update rollback backups into a permanent
# backup product).
rm -rf "$PREV_OPT_DIR" "$BACKUP_DIR" "$STAGING_ROOT"
rm -f "$TRANSACTION_MARKER"

perf_tuning_apply || warn "kernel network tuning re-apply failed; update itself still succeeded."

log "UPDATE COMPLETE: ${CURRENT_VERSION:-unknown} -> $TARGET_VERSION. sing-box: $TARGET_SINGBOX_VERSION. User credentials and REALITY/Hysteria2 material were preserved."
