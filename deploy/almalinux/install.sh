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
#
# Explicit stages (see docs/PRODUCTION_HARDENING_PLAN.md #22): each is
# logged as it runs. `set -euo pipefail` means any unhandled failure in
# any stage aborts the whole script BEFORE "Install complete." is ever
# printed — this script does not claim success on partial completion.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STATE_DIR="/etc/vpn/compat"
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
BIN_DIR="/usr/local/bin"
SINGBOX_VERSION="1.13.14"
SINGBOX_BIN="$BIN_DIR/sing-box"
NGINX_CONF="/etc/nginx/conf.d/vpn-subscription.conf"

log() { echo "[install] $*"; }
die() { echo "[install] ERROR: $*" >&2; exit 1; }
stage() { echo; echo "[install] === [$1/15] $2 ==="; }

# ---------------------------------------------------------------------
# [1] prerequisites
# ---------------------------------------------------------------------
require_root() {
  [ "$(id -u)" -eq 0 ] || die "must run as root (sudo ./deploy/almalinux/install.sh)"
}

check_os() {
  [ -f /etc/os-release ] || die "cannot detect OS (/etc/os-release missing)"
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}-${VERSION_ID:-}" in
    almalinux-9*) ;;
    *)
      log "warning: this installer targets AlmaLinux 9; detected ${PRETTY_NAME:-unknown}. Continuing, but untested elsewhere."
      ;;
  esac
}

install_packages() {
  log "installing OS packages..."
  dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ make pkgconf-pkg-config openssl-devel openssl \
    firewalld policycoreutils-python-utils tar curl jq nginx >/dev/null
  systemctl enable --now firewalld >/dev/null
}

require_env() {
  : "${PUBLIC_HOST:?Set PUBLIC_HOST=vpn.example.com before running install.sh}"
  : "${SUBSCRIPTION_HOST:="$PUBLIC_HOST"}"
  export SUBSCRIPTION_HOST
}

prerequisites() {
  stage 1 "prerequisites"
  require_root
  require_env
  check_os
  install_packages
}

# ---------------------------------------------------------------------
# [2] Rust build
# ---------------------------------------------------------------------
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

rust_build_stage() {
  stage 2 "Rust build"
  install_rust_toolchain_if_missing
  build_binaries
}

# ---------------------------------------------------------------------
# [3] sing-box installation
# ---------------------------------------------------------------------
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
  # sing-box is GPL-3.0-only upstream (see docs/COMPATIBILITY_VERSIONS.md)
  # and invoked here as an unmodified external binary, never linked into
  # any Rust binary in this repo. Keep its LICENSE alongside the binary
  # so redistribution obligations are met if this system image is itself
  # redistributed.
  if [ -f "$extracted_dir/LICENSE" ]; then
    install -m 0644 "$extracted_dir/LICENSE" "$BIN_DIR/sing-box.LICENSE"
  fi
  rm -rf "$tmpdir"
  "$SINGBOX_BIN" version || die "installed sing-box binary failed to run"
  log "sing-box ${SINGBOX_VERSION} installed at $SINGBOX_BIN"
}

singbox_install_stage() {
  stage 3 "sing-box installation"
  install_singbox
}

# ---------------------------------------------------------------------
# [4] users/groups
# ---------------------------------------------------------------------
create_service_users() {
  id vpn-subscription >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin vpn-subscription
  id sing-box >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin sing-box
}

users_groups_stage() {
  stage 4 "users/groups"
  create_service_users
}

# ---------------------------------------------------------------------
# [5] directories
# ---------------------------------------------------------------------
# Ownership matrix (docs/PRODUCTION_HARDENING_PLAN.md #1): every
# directory/file is owned by the group that ACTUALLY needs to read it at
# runtime, not by convention. `sing-box` (User=sing-box) must read the
# REALITY private key and the Hysteria2 TLS cert/key directly — it is
# never added to `vpn-subscription`'s group, and vice versa.
create_directories() {
  install -d -m 0755 /etc/vpn
  install -d -m 0750 -o root -g vpn-subscription "$STATE_DIR"
  # reality/: private.key is sing-box's secret (root:sing-box 0640);
  # public.key + short_id.txt are vpn-subscription's read-only inputs
  # (root:vpn-subscription 0640). The directory itself must be
  # traversable by both groups.
  install -d -m 0750 -o root -g sing-box "$STATE_DIR/reality"
  chmod g+rx "$STATE_DIR/reality" # traversal for the vpn-subscription-owned files inside too
  # hysteria/: cert+key are read exclusively by sing-box.
  install -d -m 0750 -o root -g sing-box "$STATE_DIR/hysteria"
  # vpn-subscription reads users.json (to verify tokens) but never
  # writes it — only vpn-admin (run as root) does. Group-readable, not
  # group-writable.
  install -d -m 0750 -o root -g vpn-subscription "$STATE_DIR/users"
  install -d -m 0755 -o sing-box -g sing-box "$STATE_DIR/sing-box"
}

directories_stage() {
  stage 5 "directories"
  create_directories
}

# ---------------------------------------------------------------------
# [6] certificates
# ---------------------------------------------------------------------
# Hysteria2 requires a valid TLS cert/key BEFORE sing-box can start —
# there is no automatic ACME provisioning here (task requirement: don't
# hand-roll ACME; a single VPS is simpler with one explicit operator
# step than a second scripted ACME integration). Fail loudly with the
# exact commands rather than starting a service that's guaranteed to
# fail (docs/PRODUCTION_HARDENING_PLAN.md #5).
require_hysteria_tls() {
  local cert="$STATE_DIR/hysteria/cert.pem"
  local key="$STATE_DIR/hysteria/key.pem"
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    cat >&2 <<EOF
[install] ERROR: Hysteria2 TLS certificate/key missing.

  Hysteria2 TLS certificate missing: $cert
  Hysteria2 TLS key missing:         $key

Provision a certificate for ${PUBLIC_HOST:-<PUBLIC_HOST>} first, e.g. with certbot:

  dnf install -y certbot
  systemctl stop nginx 2>/dev/null || true
  certbot certonly --standalone -d ${PUBLIC_HOST:-<PUBLIC_HOST>} \\
    --non-interactive --agree-tos -m admin@${PUBLIC_HOST:-<PUBLIC_HOST>}

Then copy/link the issued files into place:

  install -d -m 0750 -o root -g sing-box $STATE_DIR/hysteria
  install -m 0640 -o root -g sing-box \\
    /etc/letsencrypt/live/${PUBLIC_HOST:-<PUBLIC_HOST>}/fullchain.pem $cert
  install -m 0640 -o root -g sing-box \\
    /etc/letsencrypt/live/${PUBLIC_HOST:-<PUBLIC_HOST>}/privkey.pem $key

Re-run install.sh once both files exist. Installation stops here —
sing-box.service would otherwise start with a config that is guaranteed
to fail (spec: never claim success while Hysteria2 is guaranteed to fail).
EOF
    exit 1
  fi
  if ! openssl x509 -in "$cert" -noout -checkend 0 >/dev/null 2>&1; then
    die "Hysteria2 TLS certificate at $cert is not a valid/current certificate (openssl x509 -checkend failed)."
  fi
  log "Hysteria2 TLS cert/key present and valid: $cert"
}

# Same requirement for the subscription HTTPS vhost (nginx). Not
# ACME-automated here either, same reasoning.
require_subscription_tls() {
  local host="${SUBSCRIPTION_HOST:-$PUBLIC_HOST}"
  local cert="/etc/letsencrypt/live/$host/fullchain.pem"
  local key="/etc/letsencrypt/live/$host/privkey.pem"
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    log "warning: subscription HTTPS certificate not found at $cert."
    log "  Provision one with: certbot certonly --standalone -d $host --non-interactive --agree-tos -m admin@$host"
    log "  The subscription vhost will NOT be enabled this run — see final status."
    SUBSCRIPTION_TLS_READY=0
    return
  fi
  SUBSCRIPTION_TLS_READY=1
  log "subscription HTTPS cert/key present: $cert"
}

create_hysteria_masquerade_placeholder() {
  # sing-box `masquerade` (file type) serves this directory back to
  # unauthenticated/invalid Hysteria2 connections instead of a
  # distinctive auth-reject signature (docs/PRODUCTION_HARDENING_PLAN.md #9).
  local dir="$STATE_DIR/hysteria/masquerade"
  install -d -m 0750 -o root -g sing-box "$dir"
  if [ ! -f "$dir/index.html" ]; then
    printf '<!doctype html><html><head><title>404</title></head><body><h1>Not Found</h1></body></html>\n' \
      > "$dir/index.html"
    chown root:sing-box "$dir/index.html"
    chmod 0640 "$dir/index.html"
  fi
}

certificates_stage() {
  stage 6 "certificates"
  create_hysteria_masquerade_placeholder
  require_hysteria_tls
  require_subscription_tls
}

# ---------------------------------------------------------------------
# [7] REALITY keys
# ---------------------------------------------------------------------
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

init_reality_keys() {
  log "generating/verifying compatibility secrets (REALITY keypair)..."
  "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" init
  # vpn-admin writes private.key root:root 0600 by default (safe for
  # root-only tooling); re-own to root:sing-box 0640 so the sing-box
  # process (not root) can actually read it, and re-own the
  # subscription-facing public key/short_id to vpn-subscription. No
  # `|| true` here — getting this ownership wrong is exactly the secret
  # -exposure bug this installer exists to prevent, so a failure here
  # must abort install, not be silently ignored.
  chown root:sing-box "$STATE_DIR/reality/private.key"
  chmod 0640 "$STATE_DIR/reality/private.key"
  chown root:vpn-subscription "$STATE_DIR/reality/public.key" "$STATE_DIR/reality/short_id.txt"
  chmod 0640 "$STATE_DIR/reality/public.key" "$STATE_DIR/reality/short_id.txt"
}

reality_keys_stage() {
  stage 7 "REALITY keys"
  render_deployment_toml
  init_reality_keys
}

# ---------------------------------------------------------------------
# [8] server config
# ---------------------------------------------------------------------
render_server_config() {
  log "rendering initial sing-box config..."
  # No `|| true` here (docs/PRODUCTION_HARDENING_PLAN.md #22/#27): a
  # config that fails to render/validate must abort installation, not
  # be silently skipped while the installer proceeds to start a
  # service with no valid config.
  "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config
  chown -R root:sing-box "$STATE_DIR/sing-box"
  chmod 0750 "$STATE_DIR/sing-box"
}

server_config_stage() {
  stage 8 "server config"
  render_server_config
}

# ---------------------------------------------------------------------
# [9] nginx / subscription HTTPS
# ---------------------------------------------------------------------
configure_nginx() {
  if [ "${SUBSCRIPTION_TLS_READY:-0}" -ne 1 ]; then
    log "skipping nginx vhost: subscription TLS certificate not yet present (see stage 6 output)."
    return
  fi
  local host="${SUBSCRIPTION_HOST:-$PUBLIC_HOST}"
  sed -e "s/{{SUBSCRIPTION_HOST}}/$host/" -e "s/{{PUBLIC_HOST}}/${PUBLIC_HOST:-$host}/" \
    "$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template" >"$NGINX_CONF.tmp"
  install -m 0644 "$NGINX_CONF.tmp" "$NGINX_CONF"
  rm -f "$NGINX_CONF.tmp"
  nginx -t || die "nginx config validation failed (nginx -t) — not reloading nginx with a broken config."
  systemctl enable --now nginx >/dev/null 2>&1 || systemctl reload nginx
  log "nginx vhost installed and validated: $NGINX_CONF"
}

nginx_stage() {
  stage 9 "nginx/subscription HTTPS"
  configure_nginx
}

# ---------------------------------------------------------------------
# [10] firewall
# ---------------------------------------------------------------------
configure_firewall() {
  "$REPO_ROOT/deploy/almalinux/firewall.sh"
}

firewall_stage() {
  stage 10 "firewall"
  configure_firewall
}

# ---------------------------------------------------------------------
# [11] SELinux
# ---------------------------------------------------------------------
configure_selinux() {
  # sing-box binds 443/tcp+udp and reads config/certs from
  # /etc/vpn/compat; /usr/local/bin and /etc/vpn aren't in the default
  # SELinux policy's search paths for network services in all
  # configurations. Label the binary AND the secret-serving directories
  # it reads from (docs/PRODUCTION_HARDENING_PLAN.md #25) rather than
  # assuming labeling the binary alone is sufficient. `setenforce 0` is
  # NOT an acceptable production fix — see docs/ALMALINUX_DEPLOYMENT.md
  # for `ausearch -m AVC -ts recent` troubleshooting if AVC denials
  # appear despite this labeling.
  if command -v semanage >/dev/null 2>&1; then
    semanage fcontext -a -t bin_t "$SINGBOX_BIN" 2>/dev/null || true
    restorecon -v "$SINGBOX_BIN" >/dev/null 2>&1 || true
    semanage fcontext -a -t etc_t "$STATE_DIR/sing-box(/.*)?" 2>/dev/null || true
    semanage fcontext -a -t cert_t "$STATE_DIR/hysteria(/.*)?" 2>/dev/null || true
    restorecon -Rv "$STATE_DIR" >/dev/null 2>&1 || true
  else
    log "semanage not found; skipping explicit SELinux file context (policycoreutils-python-utils should have installed it)."
  fi
}

selinux_stage() {
  stage 11 "SELinux"
  configure_selinux
}

# ---------------------------------------------------------------------
# [12] systemd
# ---------------------------------------------------------------------
install_systemd_units() {
  log "installing systemd units..."
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/sing-box.service" /etc/systemd/system/sing-box.service
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/vpn-subscription.service" /etc/systemd/system/vpn-subscription.service
  systemctl daemon-reload
}

systemd_stage() {
  stage 12 "systemd"
  install_systemd_units
}

# ---------------------------------------------------------------------
# [13] validation
# ---------------------------------------------------------------------
validate_before_start() {
  "$SINGBOX_BIN" check -c "$STATE_DIR/sing-box/config.json" \
    || die "sing-box check failed against the rendered config — refusing to start services."
  log "sing-box config validated."
}

validation_stage() {
  stage 13 "validation"
  validate_before_start
}

# ---------------------------------------------------------------------
# [14] start
# ---------------------------------------------------------------------
enable_and_start_services() {
  systemctl enable --now sing-box.service
  systemctl enable --now vpn-subscription.service
}

start_stage() {
  stage 14 "start"
  enable_and_start_services
  install -m 0755 "$REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check"
}

# ---------------------------------------------------------------------
# [15] acceptance test
# ---------------------------------------------------------------------
acceptance_stage() {
  stage 15 "acceptance test"
  if ! "$BIN_DIR/vpn-health-check"; then
    die "post-install health check failed — see output above. Installation did not complete cleanly."
  fi
  log "health check passed."
}

print_status() {
  cat <<EOF

Install complete.

STATUS:
  compatibility transport (sing-box VLESS+REALITY / Hysteria2): INSTALLED and ACTIVE
  SUBSCRIPTION HTTPS (nginx, $NGINX_CONF): $([ "${SUBSCRIPTION_TLS_READY:-0}" -eq 1 ] && echo CONFIGURED || echo "NOT CONFIGURED — provision a cert (see stage 6 output above) and re-run install.sh")

Next steps:
  1. Create your first user:
       sudo vpn-admin --config $DEPLOYMENT_TOML user create --name test
  2. Run the health check any time:
       sudo $BIN_DIR/vpn-health-check
  3. Import the printed subscription URL into Hiddify on Android — see
     docs/HIDDIFY_ANDROID.md and docs/CLIENT_COMPATIBILITY.md.

EOF
}

main() {
  prerequisites
  rust_build_stage
  singbox_install_stage
  users_groups_stage
  directories_stage
  certificates_stage
  reality_keys_stage
  server_config_stage
  nginx_stage
  firewall_stage
  selinux_stage
  systemd_stage
  validation_stage
  start_stage
  acceptance_stage
  print_status
}

main "$@"
