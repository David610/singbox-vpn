#!/usr/bin/env bash
# Production installer for the Hiddify/VLESS-REALITY/Hysteria2
# compatibility stack on AlmaLinux 9. Idempotent where realistically
# possible: re-running skips already-generated secrets and
# already-installed packages. See docs/ALMALINUX_DEPLOYMENT.md.
#
# Explicitly out of scope here (per docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md):
# the native direct-tls/noise-quic stack (rendezvous/relay-agent) is not
# deployed by this script — it remains the `deploy/local/` dev slice.
# This installer only stands up the compatibility (sing-box) data plane
# and its Rust control-plane pieces (vpn-admin, subscription service).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STATE_DIR="/etc/vpn/compat"
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
BIN_DIR="/usr/local/bin"
SINGBOX_VERSION="1.13.14"
SINGBOX_BIN="$BIN_DIR/sing-box"

log() { echo "[install] $*"; }
die() { echo "[install] ERROR: $*" >&2; exit 1; }

require_root() {
  [ "$(id -u)" -eq 0 ] || die "must run as root (sudo ./deploy/almalinux/install.sh)"
}

check_os() {
  [ -f /etc/os-release ] || die "cannot detect OS (/etc/os-release missing)"
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}-${VERSION_ID:-}" in
    almalinux-9*|almalinux-9) ;;
    *)
      log "warning: this installer targets AlmaLinux 9; detected ${PRETTY_NAME:-unknown}. Continuing, but untested elsewhere."
      ;;
  esac
}

install_packages() {
  log "installing OS packages..."
  dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ make pkgconf-pkg-config openssl-devel \
    firewalld policycoreutils-python-utils tar curl jq >/dev/null
  systemctl enable --now firewalld >/dev/null
}

install_rust_toolchain_if_missing() {
  if ! command -v cargo >/dev/null 2>&1; then
    die "cargo not found. Install a Rust toolchain (e.g. via rustup) before running this installer."
  fi
}

build_binaries() {
  log "building release binaries (admin, subscription)..."
  ( cd "$REPO_ROOT" && cargo build --release -p admin -p subscription )
  install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin"
  install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc"
}

detect_arch() {
  case "$(uname -m)" in
    x86_64) echo "amd64" ;;
    aarch64) echo "arm64" ;;
    *) die "unsupported architecture: $(uname -m)" ;;
  esac
}

install_singbox() {
  if [ -x "$SINGBOX_BIN" ] && "$SINGBOX_BIN" version 2>/dev/null | grep -q "$SINGBOX_VERSION"; then
    log "sing-box $SINGBOX_VERSION already installed, skipping."
    return
  fi
  local arch tmpdir tarball
  arch="$(detect_arch)"
  tmpdir="$(mktemp -d)"
  tarball="sing-box-${SINGBOX_VERSION}-linux-${arch}.tar.gz"
  local url="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/${tarball}"
  log "downloading pinned sing-box ${SINGBOX_VERSION} (${arch}) from official release assets..."
  curl -fsSL -o "$tmpdir/$tarball" "$url" || die "download failed: $url"

  # Verify against upstream-published checksums when available. sing-box
  # does not currently publish a detached checksums.txt on every release;
  # if one is not published for this version, we hash-log the artifact
  # instead of silently skipping verification, per spec §10 ("verify
  # downloaded artifact integrity when upstream publishes checksums").
  local sums_url="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/sing-box_${SINGBOX_VERSION}_checksums.txt"
  if curl -fsSL -o "$tmpdir/checksums.txt" "$sums_url" 2>/dev/null; then
    ( cd "$tmpdir" && sha256sum --ignore-missing -c checksums.txt ) || die "checksum verification failed for $tarball"
    log "checksum verified against upstream checksums.txt."
  else
    log "no upstream checksums.txt found for this release; recording sha256 for audit: $(sha256sum "$tmpdir/$tarball" | awk '{print $1}')"
  fi

  tar -xzf "$tmpdir/$tarball" -C "$tmpdir"
  local extracted_dir="$tmpdir/sing-box-${SINGBOX_VERSION}-linux-${arch}"
  install -m 0755 "$extracted_dir/sing-box" "$SINGBOX_BIN"
  rm -rf "$tmpdir"
  "$SINGBOX_BIN" version || die "installed sing-box binary failed to run"
  log "sing-box ${SINGBOX_VERSION} installed at $SINGBOX_BIN"
}

create_service_users() {
  id vpn-subscription >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin vpn-subscription
  id sing-box >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin sing-box
}

create_directories() {
  install -d -m 0755 /etc/vpn
  install -d -m 0750 -o root -g vpn-subscription "$STATE_DIR"
  install -d -m 0750 -o root -g vpn-subscription "$STATE_DIR/reality"
  install -d -m 0700 -o root -g root "$STATE_DIR/hysteria"
  # vpn-subscription reads users.json (to verify tokens) but never
  # writes it — only vpn-admin (run as root) does. Group-readable, not
  # group-writable.
  install -d -m 0750 -o root -g vpn-subscription "$STATE_DIR/users"
  install -d -m 0755 -o sing-box -g sing-box "$STATE_DIR/sing-box"
}

render_deployment_toml() {
  if [ -f "$DEPLOYMENT_TOML" ]; then
    log "$DEPLOYMENT_TOML already exists, leaving it untouched."
    return
  fi
  : "${PUBLIC_HOST:?Set PUBLIC_HOST=vpn.example.com before running install.sh}"
  : "${SUBSCRIPTION_HOST:="$PUBLIC_HOST"}"
  : "${REALITY_HANDSHAKE_SERVER:="www.microsoft.com"}"
  sed -e "s/{{PUBLIC_HOST}}/$PUBLIC_HOST/" \
      -e "s/{{SUBSCRIPTION_HOST}}/$SUBSCRIPTION_HOST/" \
      -e "s/{{REALITY_HANDSHAKE_SERVER}}/$REALITY_HANDSHAKE_SERVER/" \
      "$REPO_ROOT/deploy/almalinux/templates/deployment.toml.template" >"$DEPLOYMENT_TOML"
  chmod 0644 "$DEPLOYMENT_TOML"
  log "wrote $DEPLOYMENT_TOML"
}

init_secrets_and_config() {
  log "generating/verifying compatibility secrets (REALITY keypair)..."
  "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" init
  log "rendering initial sing-box config..."
  "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config || true
  chown -R root:sing-box "$STATE_DIR/sing-box"
}

install_systemd_units() {
  log "installing systemd units..."
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/sing-box.service" /etc/systemd/system/sing-box.service
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/vpn-subscription.service" /etc/systemd/system/vpn-subscription.service
  systemctl daemon-reload
}

configure_firewall() {
  "$REPO_ROOT/deploy/almalinux/firewall.sh"
}

configure_selinux() {
  # sing-box binds 443/tcp+udp and /usr/local/bin isn't in the default
  # SELinux policy's "bin_t" search path for network services in all
  # configurations; label it so it runs confined rather than needing
  # setenforce 0 (spec §34 hard requirement).
  if command -v semanage >/dev/null 2>&1; then
    semanage fcontext -a -t bin_t "$SINGBOX_BIN" 2>/dev/null || true
    restorecon -v "$SINGBOX_BIN" >/dev/null 2>&1 || true
  else
    log "semanage not found; skipping explicit SELinux file context (policycoreutils-python-utils should have installed it)."
  fi
}

enable_and_start_services() {
  systemctl enable --now sing-box.service
  systemctl enable --now vpn-subscription.service
}

print_next_steps() {
  cat <<EOF

Install complete.

Next steps:
  1. Create your first user:
       sudo vpn-admin --config $DEPLOYMENT_TOML user create --name test
  2. Put a reverse proxy (nginx/caddy) in front of 127.0.0.1:9100 to
     terminate public HTTPS on 8443 for the subscription endpoint — see
     docs/ALMALINUX_DEPLOYMENT.md.
  3. Run the health check:
       sudo $BIN_DIR/vpn-health-check
  4. Import the printed subscription URL into Hiddify on Android.

EOF
}

main() {
  require_root
  check_os
  install_packages
  install_rust_toolchain_if_missing
  build_binaries
  install_singbox
  create_service_users
  create_directories
  render_deployment_toml
  init_secrets_and_config
  install_systemd_units
  configure_firewall
  configure_selinux
  enable_and_start_services
  install -m 0755 "$REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check"
  print_next_steps
}

main "$@"
