#!/usr/bin/env bash
# Production installer for the Hiddify/VLESS-REALITY/Hysteria2
# compatibility stack. Idempotent where realistically possible:
# re-running skips already-generated secrets and already-installed
# packages. See docs/ALMALINUX_DEPLOYMENT.md.
#
# Despite the directory name (kept for backwards compatibility with
# existing docs/links), this script now supports the RHEL family
# (AlmaLinux, Rocky Linux, RHEL) and the Debian family (Ubuntu, Debian) —
# see deploy/lib/os.sh. Package-manager/firewall logic is branched
# per-family; nothing else about the deployment changes between them.
#
# Explicitly out of scope here (per docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md):
# the native direct-tls/noise-quic stack (rendezvous/relay-agent) is not
# deployed by this script — it remains the `deploy/local/` dev slice.
# This installer only stands up the compatibility (sing-box) data plane
# and its Rust control-plane pieces (vpn-admin, subscription service).
#
# Explicit stages (see docs/PRODUCTION_HARDENING_PLAN.md #22): each is
# logged as it runs. `set -Eeuo pipefail` means any unhandled failure in
# any stage aborts the whole script BEFORE "Install complete." is ever
# printed — this script does not claim success on partial completion.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STATE_DIR="/etc/vpn/compat"
DEPLOYMENT_TOML="/etc/vpn/deployment.toml"
BIN_DIR="/usr/local/bin"
SINGBOX_VERSION="1.13.14"
SINGBOX_BIN="$BIN_DIR/sing-box"
NGINX_CONF="/etc/nginx/conf.d/vpn-subscription.conf"
VPN1_VERSION="${VPN1_VERSION:-}"
VPN1_RELEASE_REPO="${VPN1_RELEASE_REPO:-David610/vpn1}"

log() { echo "[install] $*"; }
warn() { echo "[install] WARNING: $*" >&2; }
die() {
  echo "[install] ERROR: $*" >&2
  echo "[install] Useful diagnostics:" >&2
  echo "  journalctl -u sing-box -u vpn-subscription --no-pager -n 100" >&2
  echo "  vpn doctor" >&2
  exit 1
}
stage() { echo; echo "[install] === [$1/17] $2 ==="; }

# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/os.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"

# ---------------------------------------------------------------------
# [1] preflight
# ---------------------------------------------------------------------
existing_install_present() {
  [ -f "$DEPLOYMENT_TOML" ] || [ -x "$SINGBOX_BIN" ]
}

check_ports_free() {
  # Skip the check for ports vpn1 itself already owns on a re-run — an
  # already-installed vpn1 legitimately holds 443/tcp+udp.
  if existing_install_present; then
    log "existing installation detected — skipping port-conflict checks (vpn1 owns these ports already)."
    return
  fi
  local failed=0
  preflight_check_port_free tcp 443 || failed=1
  preflight_check_port_free udp 443 || failed=1
  preflight_check_port_free tcp 8443 || failed=1
  if [ "$failed" -eq 1 ]; then
    die "vpn1 cannot safely continue while a required port is occupied by another service. Free the port(s) above (or move the other service) and re-run."
  fi
  log "required ports (443/tcp, 443/udp, 8443/tcp) are free."
}

# When invoked via the curl|bash bootstrap, $REPO_ROOT points at a
# temporary extraction directory that the bootstrap deletes on exit —
# any path we print for later use (update.sh, uninstall.sh, a manual
# re-run) must survive past this process. Install a persistent copy of
# the source tree once, and repoint $REPO_ROOT at it for the rest of
# this run. A no-op when already running from that persistent copy
# (e.g. someone `cd /opt/vpn1 && ./deploy/almalinux/install.sh`).
PERSIST_DIR="/opt/vpn1"
persist_source_tree() {
  if [ "$REPO_ROOT" = "$PERSIST_DIR" ]; then
    return
  fi
  log "installing a persistent copy of the vpn1 source to $PERSIST_DIR (for future updates/uninstall)..."
  mkdir -p "$PERSIST_DIR"
  ( cd "$REPO_ROOT" && tar --exclude=target --exclude=.git -cf - . ) | ( cd "$PERSIST_DIR" && tar -xf - )
  REPO_ROOT="$PERSIST_DIR"
}

preflight_stage() {
  stage 1 "preflight"
  preflight_require_root
  detect_os || die "unsupported operating system."
  log "detected OS: $OS_PRETTY_NAME (family=$OS_FAMILY, support=$OS_SUPPORT)"
  [ "$OS_SUPPORT" = "tested" ] || warn "this OS/version combination is not in the tested support matrix; continuing, but this is not a guarantee it works."
  ARCH="$(detect_arch)" || die "unsupported CPU architecture: $(uname -m). vpn1 supports x86_64 and aarch64."
  log "detected architecture: $ARCH ($(uname -m))"
  preflight_require_systemd
  preflight_require_commands curl tar awk sed grep
  preflight_check_disk_space / 2048
  preflight_check_memory 1024
  preflight_check_connectivity "https://github.com"
  preflight_check_dns "github.com"
  check_ports_free
  if existing_install_present; then
    log "existing vpn1 installation detected at $DEPLOYMENT_TOML — this run will UPGRADE/REPAIR in place, preserving users and keys."
  else
    log "no existing installation detected — this will be a fresh install."
  fi
  persist_source_tree
}

# ---------------------------------------------------------------------
# [2] OS packages
# ---------------------------------------------------------------------
install_dependencies_rhel() {
  log "installing OS packages (dnf)..."
  dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ make pkgconf-pkg-config openssl-devel openssl \
    firewalld policycoreutils-python-utils tar curl jq nginx certbot >/dev/null
  systemctl enable --now firewalld >/dev/null
}

install_dependencies_debian() {
  log "installing OS packages (apt)..."
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y >/dev/null
  apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev \
    ufw tar curl jq nginx certbot ca-certificates >/dev/null
  systemctl enable --now ufw >/dev/null 2>&1 || true
}

install_packages() {
  case "$OS_FAMILY" in
    rhel) install_dependencies_rhel ;;
    debian) install_dependencies_debian ;;
    *) die "no package-manager logic for OS family '$OS_FAMILY'" ;;
  esac
}

packages_stage() {
  stage 2 "OS packages"
  install_packages
}

# ---------------------------------------------------------------------
# [3] host configuration (PUBLIC_HOST/SUBSCRIPTION_HOST auto-detection)
# ---------------------------------------------------------------------
# Re-runs must keep using whatever host is already committed to
# deployment.toml — re-detecting the public IP on every run and
# potentially changing PUBLIC_HOST out from under an already-issued
# certificate would break the deployment, not repair it.
load_existing_host_config() {
  [ -f "$DEPLOYMENT_TOML" ] || return 1
  local existing_public existing_sub
  existing_public="$(grep -E '^public_host' "$DEPLOYMENT_TOML" | sed -E 's/^public_host *= *"([^"]*)".*/\1/')"
  existing_sub="$(grep -E '^subscription_host' "$DEPLOYMENT_TOML" | sed -E 's/^subscription_host *= *"([^"]*)".*/\1/')"
  [ -n "$existing_public" ] || return 1
  PUBLIC_HOST="$existing_public"
  SUBSCRIPTION_HOST="${existing_sub:-$existing_public}"
  return 0
}

# sslip.io resolves "<ip-with-dashes>.sslip.io" to that literal IP with
# no DNS setup required — this lets certbot issue a REAL, CA-trusted
# certificate against a public IP alone, with zero manual domain/DNS
# steps (task requirement: no manual PUBLIC_HOST/TLS steps for the
# common case). If the operator supplies their own PUBLIC_HOST, it is
# always preferred and used as-is.
derive_auto_host() {
  local ip="$1"
  echo "${ip//./-}.sslip.io"
}

resolve_host_config() {
  if load_existing_host_config; then
    log "using host configuration from existing $DEPLOYMENT_TOML: PUBLIC_HOST=$PUBLIC_HOST SUBSCRIPTION_HOST=$SUBSCRIPTION_HOST"
    export PUBLIC_HOST SUBSCRIPTION_HOST
    AUTO_TLS_DOMAIN=0
    return
  fi

  if [ -n "${PUBLIC_HOST:-}" ]; then
    log "using operator-supplied PUBLIC_HOST=$PUBLIC_HOST"
    AUTO_TLS_DOMAIN=0
  else
    log "no PUBLIC_HOST set — detecting public IP for zero-touch install..."
    PUBLIC_IP="$(preflight_detect_public_ip)" || die "could not auto-detect this server's public IP. Re-run with PUBLIC_HOST=your.domain.com set explicitly."
    log "detected public IP: $PUBLIC_IP"
    PUBLIC_HOST="$(derive_auto_host "$PUBLIC_IP")"
    log "auto-assigned hostname: $PUBLIC_HOST (resolves to $PUBLIC_IP via sslip.io, no DNS setup needed)"
    AUTO_TLS_DOMAIN=1
  fi
  SUBSCRIPTION_HOST="${SUBSCRIPTION_HOST:-$PUBLIC_HOST}"
  export PUBLIC_HOST SUBSCRIPTION_HOST AUTO_TLS_DOMAIN
}

host_config_stage() {
  stage 3 "host configuration"
  resolve_host_config
}

# ---------------------------------------------------------------------
# [4] vpn1 binaries (prebuilt release, falling back to source build)
# ---------------------------------------------------------------------
fetch_release_binaries() {
  local target version base_url tmp
  target="$(rust_target_for_arch "$ARCH")" || return 1
  version="${VPN1_VERSION:-latest}"
  if [ "$version" = "latest" ]; then
    base_url="https://github.com/$VPN1_RELEASE_REPO/releases/latest/download"
  else
    base_url="https://github.com/$VPN1_RELEASE_REPO/releases/download/$version"
  fi
  tmp="$(mktemp -d)"
  local asset="vpn1-${target}.tar.gz"
  log "checking for a prebuilt release ($asset)..."
  if ! curl -fsSL -o "$tmp/$asset" "$base_url/$asset" 2>/dev/null; then
    log "no prebuilt release available (this is expected until a release is tagged) — falling back to building from source."
    rm -rf "$tmp"
    return 1
  fi
  if curl -fsSL -o "$tmp/SHA256SUMS" "$base_url/SHA256SUMS" 2>/dev/null; then
    ( cd "$tmp" && sha256sum --ignore-missing -c SHA256SUMS ) || die "checksum verification failed for $asset — refusing to install unverified binaries."
    log "checksum verified against release SHA256SUMS."
  else
    die "release asset $asset was found but SHA256SUMS was not — refusing to install a binary with no integrity verification."
  fi
  tar -xzf "$tmp/$asset" -C "$tmp"
  install -m 0755 "$tmp/vpn-admin" "$BIN_DIR/vpn-admin"
  install -m 0755 "$tmp/vpn-admin" "$BIN_DIR/vpn"
  install -m 0755 "$tmp/subscription" "$BIN_DIR/vpn-subscription-svc"
  rm -rf "$tmp"
  log "installed prebuilt vpn1 $version binaries ($target) — no Rust compiler needed."
  return 0
}

install_rustup_noninteractive() {
  log "cargo not found; installing a Rust toolchain via rustup (no prebuilt release was available)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
}

build_binaries_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    install_rustup_noninteractive
  fi
  log "building release binaries from source (admin, subscription)..."
  ( cd "$REPO_ROOT" && cargo build --release -p admin -p subscription )
  install -m 0755 "$REPO_ROOT/target/release/vpn-admin" "$BIN_DIR/vpn-admin"
  install -m 0755 "$REPO_ROOT/target/release/vpn" "$BIN_DIR/vpn"
  install -m 0755 "$REPO_ROOT/target/release/subscription" "$BIN_DIR/vpn-subscription-svc"
}

binaries_stage() {
  stage 4 "vpn1 binaries"
  fetch_release_binaries || build_binaries_from_source
}

# ---------------------------------------------------------------------
# [5] sing-box installation
# ---------------------------------------------------------------------
install_singbox() {
  if [ -x "$SINGBOX_BIN" ] && "$SINGBOX_BIN" version 2>/dev/null | grep -q "$SINGBOX_VERSION"; then
    log "sing-box $SINGBOX_VERSION already installed, skipping."
    return
  fi
  local tmpdir tarball
  tmpdir="$(mktemp -d)"
  tarball="sing-box-${SINGBOX_VERSION}-linux-${ARCH}.tar.gz"
  local url="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/${tarball}"
  log "downloading pinned sing-box ${SINGBOX_VERSION} (${ARCH}) from official release assets..."
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
  local extracted_dir="$tmpdir/sing-box-${SINGBOX_VERSION}-linux-${ARCH}"
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
  stage 5 "sing-box installation"
  install_singbox
}

# ---------------------------------------------------------------------
# [6] users/groups
# ---------------------------------------------------------------------
create_service_users() {
  id vpn-subscription >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin vpn-subscription
  id sing-box >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin sing-box
}

users_groups_stage() {
  stage 6 "users/groups"
  create_service_users
}

# ---------------------------------------------------------------------
# [7] directories
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
  stage 7 "directories"
  create_directories
}

# ---------------------------------------------------------------------
# [8] certificates
# ---------------------------------------------------------------------
# Hysteria2 requires a valid TLS cert/key BEFORE sing-box can start.
# When AUTO_TLS_DOMAIN=1 (no operator-supplied domain), we issue a real
# certificate automatically for the sslip.io hostname assigned in stage
# 3 via certbot's HTTP-01 challenge — no manual DNS/domain steps, no
# hand-rolled ACME client (still using certbot). This only requires
# outbound HTTPS + port 80 briefly free, both true on a fresh VPS. If it
# fails (egress blocked, port 80 unavailable, etc.) we stop with the
# exact manual recovery commands rather than silently degrading to a
# cert that client apps will reject.
attempt_automatic_certbot() {
  local host="$1"
  command -v certbot >/dev/null 2>&1 || { warn "certbot not installed; cannot auto-provision a certificate for $host."; return 1; }
  if ! preflight_check_port_free tcp 80 >/dev/null 2>&1; then
    warn "port 80/tcp is occupied; certbot's standalone HTTP-01 challenge cannot run automatically for $host."
    return 1
  fi
  log "requesting a Let's Encrypt certificate for $host via certbot (HTTP-01, standalone)..."
  systemctl stop nginx 2>/dev/null || true
  if certbot certonly --standalone -d "$host" --non-interactive --agree-tos \
      -m "admin@$host" --no-eff-email >/dev/null 2>&1; then
    log "certificate issued for $host."
    return 0
  fi
  warn "automatic certbot issuance failed for $host."
  return 1
}

ensure_tls_material() {
  local host="$1"
  local le_cert="/etc/letsencrypt/live/$host/fullchain.pem"
  if [ -s "$le_cert" ]; then
    return 0
  fi
  if [ "${AUTO_TLS_DOMAIN:-0}" -eq 1 ] || [ -n "${VPN1_AUTO_CERTBOT:-}" ]; then
    attempt_automatic_certbot "$host" && return 0
  fi
  return 1
}

require_hysteria_tls() {
  local cert="$STATE_DIR/hysteria/cert.pem"
  local key="$STATE_DIR/hysteria/key.pem"
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    if ensure_tls_material "$PUBLIC_HOST"; then
      install -d -m 0750 -o root -g sing-box "$STATE_DIR/hysteria"
      install -m 0640 -o root -g sing-box \
        "/etc/letsencrypt/live/$PUBLIC_HOST/fullchain.pem" "$cert"
      install -m 0640 -o root -g sing-box \
        "/etc/letsencrypt/live/$PUBLIC_HOST/privkey.pem" "$key"
    fi
  fi
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    cat >&2 <<EOF
[install] ERROR: Hysteria2 TLS certificate/key missing.

  Hysteria2 TLS certificate missing: $cert
  Hysteria2 TLS key missing:         $key

Provision a certificate for ${PUBLIC_HOST:-<PUBLIC_HOST>} first, e.g. with certbot:

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

# Same requirement for the subscription HTTPS vhost (nginx).
require_subscription_tls() {
  local host="${SUBSCRIPTION_HOST:-$PUBLIC_HOST}"
  local cert="/etc/letsencrypt/live/$host/fullchain.pem"
  local key="/etc/letsencrypt/live/$host/privkey.pem"
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    ensure_tls_material "$host" || true
  fi
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
  stage 8 "certificates"
  create_hysteria_masquerade_placeholder
  require_hysteria_tls
  require_subscription_tls
  if command -v nginx >/dev/null 2>&1; then systemctl start nginx >/dev/null 2>&1 || true; fi
}

# ---------------------------------------------------------------------
# [9] REALITY keys
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
  stage 9 "REALITY keys"
  render_deployment_toml
  init_reality_keys
}

# ---------------------------------------------------------------------
# [10] server config
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
  stage 10 "server config"
  render_server_config
}

# ---------------------------------------------------------------------
# [11] nginx / subscription HTTPS
# ---------------------------------------------------------------------
configure_nginx() {
  if [ "${SUBSCRIPTION_TLS_READY:-0}" -ne 1 ]; then
    log "skipping nginx vhost: subscription TLS certificate not yet present (see stage 8 output)."
    return
  fi
  local host="${SUBSCRIPTION_HOST:-$PUBLIC_HOST}"
  sed -e "s/{{SUBSCRIPTION_HOST}}/$host/" -e "s/{{PUBLIC_HOST}}/${PUBLIC_HOST:-$host}/" \
    "$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template" >"$NGINX_CONF.tmp"
  install -d -m 0755 "$(dirname "$NGINX_CONF")"
  install -m 0644 "$NGINX_CONF.tmp" "$NGINX_CONF"
  rm -f "$NGINX_CONF.tmp"
  nginx -t || die "nginx config validation failed (nginx -t) — not reloading nginx with a broken config."
  systemctl enable --now nginx >/dev/null 2>&1 || systemctl reload nginx
  log "nginx vhost installed and validated: $NGINX_CONF"
}

nginx_stage() {
  stage 11 "nginx/subscription HTTPS"
  configure_nginx
}

# ---------------------------------------------------------------------
# [12] firewall
# ---------------------------------------------------------------------
configure_firewall() {
  case "$FIREWALL_BACKEND" in
    firewalld) "$REPO_ROOT/deploy/almalinux/firewall.sh" ;;
    ufw) "$REPO_ROOT/deploy/almalinux/firewall-ufw.sh" ;;
    *) die "no firewall logic for backend '$FIREWALL_BACKEND'" ;;
  esac
}

firewall_stage() {
  stage 12 "firewall"
  configure_firewall
}

# ---------------------------------------------------------------------
# [13] SELinux (RHEL family only)
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
  stage 13 "SELinux"
  if [ "$OS_FAMILY" = "rhel" ]; then
    configure_selinux
  else
    log "skipping (not a RHEL-family host)."
  fi
}

# ---------------------------------------------------------------------
# [14] systemd
# ---------------------------------------------------------------------
install_systemd_units() {
  log "installing systemd units..."
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/sing-box.service" /etc/systemd/system/sing-box.service
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/vpn-subscription.service" /etc/systemd/system/vpn-subscription.service
  systemctl daemon-reload
}

systemd_stage() {
  stage 14 "systemd"
  install_systemd_units
}

# ---------------------------------------------------------------------
# [15] validation + start
# ---------------------------------------------------------------------
validate_before_start() {
  "$SINGBOX_BIN" check -c "$STATE_DIR/sing-box/config.json" \
    || die "sing-box check failed against the rendered config — refusing to start services."
  log "sing-box config validated."
}

enable_and_start_services() {
  systemctl enable --now sing-box.service
  systemctl enable --now vpn-subscription.service
}

start_stage() {
  stage 15 "validation + start"
  validate_before_start
  enable_and_start_services
  install -m 0755 "$REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check"
}

# ---------------------------------------------------------------------
# [16] first user + acceptance test
# ---------------------------------------------------------------------
DEFAULT_USER_NAME="default"
SUBSCRIPTION_URL=""

ensure_first_user() {
  local existing
  existing="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" user list 2>/dev/null | tail -n +2 | grep -c . || true)"
  if [ "${existing:-0}" -gt 0 ]; then
    log "$existing existing user(s) found — not creating a default user (idempotent re-run)."
    return
  fi
  log "creating initial VPN user '$DEFAULT_USER_NAME'..."
  local out
  out="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" user create --name "$DEFAULT_USER_NAME" --json 2>/dev/null)" \
    || { warn "failed to auto-create the default user; run 'vpn user create --name default' manually."; return; }
  SUBSCRIPTION_URL="$(echo "$out" | grep -o '"subscription_url":"[^"]*"' | sed -E 's/.*:"([^"]*)"/\1/')"
}

acceptance_stage() {
  stage 16 "first user + acceptance test"
  ensure_first_user
  if ! "$BIN_DIR/vpn-health-check"; then
    die "post-install health check failed — see output above. Installation did not complete cleanly."
  fi
  log "health check passed."
}

# ---------------------------------------------------------------------
# [17] summary
# ---------------------------------------------------------------------
print_status() {
  stage 17 "summary"
  local tls_status
  tls_status="$([ "${SUBSCRIPTION_TLS_READY:-0}" -eq 1 ] && echo "configured" || echo "NOT configured")"
  cat <<BANNER

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 vpn1 installation complete
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Server
  Address: ${PUBLIC_HOST}
  Status:  running
Protocols
  VLESS + REALITY     ✓
  Hysteria2           $([ "${SUBSCRIPTION_TLS_READY:-0}" -eq 1 ] && echo "✓" || echo "cert not ready — see above")
User
  ${DEFAULT_USER_NAME}
BANNER
  if [ -n "$SUBSCRIPTION_URL" ]; then
    echo "Subscription URL"
    echo "  $SUBSCRIPTION_URL"
    if command -v qrencode >/dev/null 2>&1; then
      echo
      echo "Scan this with Hiddify:"
      qrencode -t ANSIUTF8 "$SUBSCRIPTION_URL" 2>/dev/null || true
    fi
  else
    echo "Subscription URL"
    echo "  (not created automatically — run: vpn user create --name default)"
  fi
  cat <<BANNER

Compatible clients
  Android / MagicOS: Hiddify
  iOS:                Hiddify
  Linux:              Hiddify / compatible sing-box client

Management
  vpn status
  vpn doctor
  vpn user create --name NAME
  vpn user list
  vpn user rotate-token USER
  vpn user remove USER
  $REPO_ROOT/deploy/almalinux/update.sh
  $REPO_ROOT/deploy/almalinux/uninstall.sh

Subscription HTTPS (nginx): $tls_status

Documentation:
  https://github.com/$VPN1_RELEASE_REPO

BANNER
}

main() {
  preflight_stage
  packages_stage
  host_config_stage
  binaries_stage
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
  start_stage
  acceptance_stage
  print_status
}

main "$@"
