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
# The backend port is configurable ([subscription] listen_port) and the
# deployment.toml template explicitly invites hand-editing, but this probe
# hardcoded 9100 — so changing the port made a HEALTHY deployment fail here
# (and made install.sh's equivalent probe abort a healthy install).
SUBSCRIPTION_BACKEND_PORT="$(awk '/^\[subscription\]/{s=1;next} /^\[/{s=0} s && /^[[:space:]]*listen_port[[:space:]]*=/{gsub(/[^0-9]/,"",$0); print; exit}' "$DEPLOYMENT_TOML" 2>/dev/null || true)"
: "${SUBSCRIPTION_BACKEND_PORT:=9100}"
BIN_DIR="/usr/local/bin"
# Single authoritative source for SINGBOX_VERSION/SINGBOX_SHA256_AMD64/
# SINGBOX_SHA256_ARM64/SUPPORTED_ARCH — see deploy/lib/versions.env's own
# header. Never redefine these values here; edit that file instead so
# install.sh, CI's singbox-validate job, and deploy/lib/fast-gate.sh
# cannot drift apart.
#
# 1.13.14 -> 1.13.18 change audit (docs/PERFORMANCE_OPTIMIZATION_PLAN.md
# has the full write-up): no REALITY changes, no Hysteria2/vless/reality
# config-schema changes affecting this generator's fields, two
# QUIC-adjacent stability fixes (quic-go write leak; sing-quic UDP
# sessions not closed on connection close — the latter net-positive for
# Hysteria2 reliability), one unrelated AnyTLS privacy fix. No known
# regressions found for this range. v1.13.17 does not exist as a stable
# release (SagerNet went 1.13.16 -> 1.13.18 directly).
VERSIONS_ENV="$REPO_ROOT/deploy/lib/versions.env"
[ -f "$VERSIONS_ENV" ] || { echo "[install] ERROR: missing $VERSIONS_ENV — cannot resolve pinned sing-box version/checksums." >&2; exit 1; }
# shellcheck source=/dev/null
. "$VERSIONS_ENV"
for v in SINGBOX_VERSION SINGBOX_SHA256_AMD64 SINGBOX_SHA256_ARM64 SUPPORTED_ARCH; do
  [ -n "${!v:-}" ] || { echo "[install] ERROR: $v missing from $VERSIONS_ENV." >&2; exit 1; }
done
SINGBOX_BIN="$BIN_DIR/sing-box"
NGINX_CONF="/etc/nginx/conf.d/vpn-subscription.conf"
VPN1_VERSION="${VPN1_VERSION:-}"
VPN1_RELEASE_REPO="${VPN1_RELEASE_REPO:-David610/vpn1}"

# Independent component status, set ONLY at the point each component
# actually confirms success (docs/FINAL_PRODUCTION_AUDIT.md P0-14) — never
# inferred from an unrelated variable. print_status() reads only these.
VLESS_REALITY_OK=0
HYSTERIA2_OK=0
SUBSCRIPTION_BACKEND_OK=0
SUBSCRIPTION_HTTPS_OK=0
NGINX_OK=0
FIREWALL_OK=0

log() { echo "[install] $*"; }
warn() { echo "[install] WARNING: $*" >&2; }
die() {
  echo "[install] ERROR: $*" >&2
  echo "[install] Useful diagnostics:" >&2
  echo "  journalctl -u sing-box -u vpn-subscription --no-pager -n 100" >&2
  echo "  vpn doctor" >&2
  exit 1
}
stage() { echo; echo "[install] === [$1/18] $2 ==="; }

# Shared curl flags for network fetches below (sing-box release asset,
# checksums, prebuilt vpn1 release). `--retry` alone does not protect
# against a connection that opens fine but then stalls at zero
# throughput partway through — curl only retries a *completed* failure,
# so a stalled transfer just hangs forever with no error. Observed for
# real on a flaky VPS network. `--speed-limit`/`--speed-time` makes
# curl itself detect and abort a stalled transfer so `--retry` gets a
# chance to run; `--connect-timeout`/`--max-time` bound the rest.
CURL_NET_FLAGS=(--connect-timeout 10 --max-time 300 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2)

# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/os.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/preflight.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/perf-tuning.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/ownership.sh"

# ---------------------------------------------------------------------
# CLI flags (all optional; env var equivalents also work, e.g. for
# `curl | sudo bash -s -- --domain vpn.example.com`). The normal
# zero-argument invocation never requires any of these.
# ---------------------------------------------------------------------
NONINTERACTIVE="${NONINTERACTIVE:-0}"
# Set definitively inside preflight_stage once existing_install_present()
# has actually been checked. Defaults to 0 (repair) so that IF the
# on_fatal_error trap somehow fires before preflight_stage reaches that
# check, it fails safe: it does NOT auto-uninstall on the (safer)
# assumption that something might already exist, rather than risking a
# full teardown of a live deployment it never actually verified was new.
IS_FRESH_INSTALL=0
print_install_help() {
  cat <<'USAGE'
vpn1 installer (deploy/almalinux/install.sh).

Normal usage takes no arguments at all:
  curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash

Optional flags (all have environment-variable equivalents):
  --domain HOST                    same as PUBLIC_HOST; accepts a Unicode
                                    (IDN) domain such as чёрт.com and
                                    converts it to punycode automatically
  --reality-handshake-server HOST  same as REALITY_HANDSHAKE_SERVER; a TLS
                                    1.3 hostname to use as the REALITY decoy.
                                    Required in non-interactive mode — there
                                    is no safe default.
  --subscription-port PORT         same as SUBSCRIPTION_PORT (default 8443)
  --non-interactive                never prompt on /dev/tty; fail fast with
                                    a precise error instead of blocking if a
                                    required value (e.g. the REALITY decoy)
                                    is missing. Implied automatically when no
                                    TTY is attached.
  -h, --help                       show this message and exit
USAGE
}

parse_cli_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --domain) PUBLIC_HOST="$2"; shift 2 ;;
      --domain=*) PUBLIC_HOST="${1#*=}"; shift ;;
      --reality-handshake-server) REALITY_HANDSHAKE_SERVER="$2"; shift 2 ;;
      --reality-handshake-server=*) REALITY_HANDSHAKE_SERVER="${1#*=}"; shift ;;
      --subscription-port) SUBSCRIPTION_PORT="$2"; shift 2 ;;
      --subscription-port=*) SUBSCRIPTION_PORT="${1#*=}"; shift ;;
      --non-interactive) NONINTERACTIVE=1; shift ;;
      -h|--help) print_install_help; exit 0 ;;
      *) die "unknown argument: $1 (see --help)" ;;
    esac
  done
}

# ---------------------------------------------------------------------
# Transactional/rollback-safe installation: a failed FRESH install must
# never leave the VPS half-installed. The ownership manifest
# (deploy/lib/ownership.sh) is written incrementally, starting in stage
# 1 — not only at the end — so at ANY point after preflight begins, it
# accurately reflects everything mutated so far. If any stage fails
# fatally (any unhandled non-zero exit under `set -Eeuo pipefail`) DURING
# A FRESH INSTALL (nothing existed before this run started), this trap
# automatically runs the same uninstaller a manual "remove vpn1" run
# would use, so a failed fresh install is cleaned back up to a pristine
# state without operator intervention.
#
# Deliberately NOT applied to a failed REPAIR/upgrade run (one where
# existing_install_present() was already true when this run started,
# i.e. IS_FRESH_INSTALL=0): auto-uninstalling in that case would destroy
# a previously-working deployment's live users/keys/certificates over
# what might be a purely transient failure, which is strictly worse than
# leaving the (still mostly-working) host as-is for the operator to
# retry or investigate.
#
# Set VPN1_NO_AUTO_ROLLBACK=1 to disable automatic rollback entirely
# (fresh installs included) and leave the partial install in place for
# debugging.
# ---------------------------------------------------------------------
on_fatal_error() {
  local exit_code=$?
  trap - ERR
  echo "[install] ERROR: installation failed (exit $exit_code)." >&2
  if [ "${VPN1_NO_AUTO_ROLLBACK:-0}" -eq 1 ] 2>/dev/null; then
    echo "[install] VPN1_NO_AUTO_ROLLBACK=1 set — leaving the host as-is for inspection. Run '$REPO_ROOT/deploy/almalinux/uninstall.sh' to remove everything vpn1 created so far." >&2
    exit "$exit_code"
  fi
  if ! ownership_is_marked INSTALL_ATTEMPTED; then
    # Nothing was mutated yet (failure happened before stage 1 finished
    # resolving input) — nothing to roll back.
    exit "$exit_code"
  fi
  if [ "$IS_FRESH_INSTALL" -ne 1 ]; then
    # This run was a REPAIR/upgrade of an existing, previously-working
    # installation (existing_install_present() was already true when
    # this run started) — automatically running a COMPLETE uninstall
    # here would destroy a live deployment's users/keys/certificates
    # over what might be a purely transient failure (a network blip
    # mid-repair, for example). That is a strictly worse outcome than
    # leaving the host in its current (partially-repaired, but still
    # running with its previous working state underneath) condition.
    # Automatic rollback is only safe — and only attempted — for a
    # FRESH install that had nothing to lose.
    echo "[install] this was a REPAIR of an existing installation, not a fresh install — NOT auto-rolling-back (that would risk destroying a previously-working deployment's users/keys/certificates over what may be a transient failure)." >&2
    echo "[install] the previous working installation should still be intact; re-run install.sh to retry the repair, or investigate with 'vpn doctor' / 'journalctl -u sing-box -u vpn-subscription'." >&2
    exit "$exit_code"
  fi
  echo "[install] this was a FRESH install (nothing existed before this run) — rolling back everything vpn1 created during this failed attempt..." >&2
  if [ -x "$REPO_ROOT/deploy/almalinux/uninstall.sh" ]; then
    bash "$REPO_ROOT/deploy/almalinux/uninstall.sh" \
      || echo "[install] WARNING: automatic rollback itself hit an error — inspect the host and re-run '$REPO_ROOT/deploy/almalinux/uninstall.sh' manually." >&2
  else
    echo "[install] WARNING: could not find uninstall.sh to auto-rollback; run it manually once available." >&2
  fi
  exit "$exit_code"
}
trap on_fatal_error ERR

# ---------------------------------------------------------------------
# [1] preflight
# ---------------------------------------------------------------------
existing_install_present() {
  # Only the manifest proves vpn1 completed and owns the listeners. A
  # foreign sing-box binary or an interrupted partial install must not make
  # us skip conflict checks and overwrite another service.
  [ -f /var/lib/vpn1/install-state.json ]
}

# The subscription HTTPS port is operator-configurable (SUBSCRIPTION_PORT,
# default 8443) — some VPSes already run something else on 8443. Resolved
# early (before the stage-1 port-conflict check needs it) and reused as-is
# by every later stage (deployment.toml, nginx vhost, firewall rules) so
# the whole install is self-consistent. On a re-run against an existing
# install, the already-committed port always wins — changing it out from
# under an already-configured nginx vhost/firewall rule would break the
# deployment, not repair it, exactly like PUBLIC_HOST above.
resolve_subscription_port() {
  if [ -n "${SUBSCRIPTION_PORT:-}" ] && [ -f "$DEPLOYMENT_TOML" ]; then
    # The committed port always wins on a re-run — `render_deployment_toml`
    # refuses to rewrite an existing deployment.toml, so honouring the env
    # override here pointed nginx, the firewall and the SELinux label at one
    # port while the subscription service and every generated subscription
    # URL still used the committed one. Refuse rather than split-brain.
    local committed
    committed="$(awk -F'=' '/^[[:space:]]*public_port[[:space:]]*=/ {gsub(/[^0-9]/,"",$2); print $2; exit}' "$DEPLOYMENT_TOML" 2>/dev/null || true)"
    if [ -n "$committed" ] && [ "$committed" != "$SUBSCRIPTION_PORT" ]; then
      die "SUBSCRIPTION_PORT=$SUBSCRIPTION_PORT was supplied, but this deployment is already committed to port $committed in $DEPLOYMENT_TOML.
Changing the port is not a re-run: it needs deployment.toml updated AND vpn-subscription restarted (it caches the port at startup) AND the old firewall rule removed.
Either re-run without SUBSCRIPTION_PORT to keep $committed, or change public_port in $DEPLOYMENT_TOML first."
    fi
  elif [ -f "$DEPLOYMENT_TOML" ]; then
    local existing_port
    existing_port="$(grep -E '^public_port' "$DEPLOYMENT_TOML" | sed -E 's/^public_port *= *([0-9]+).*/\1/')"
    [ -n "$existing_port" ] && SUBSCRIPTION_PORT="$existing_port"
  fi
  SUBSCRIPTION_PORT="${SUBSCRIPTION_PORT:-8443}"
  preflight_validate_port "$SUBSCRIPTION_PORT" "SUBSCRIPTION_PORT" || die "invalid SUBSCRIPTION_PORT — refusing to interpolate it into deployment.toml/nginx config/firewall rules."
  [ "$SUBSCRIPTION_PORT" != "443" ] \
    || die "SUBSCRIPTION_PORT=443 collides with the VLESS+REALITY listener. Choose a different HTTPS port."
  [ "$SUBSCRIPTION_PORT" != "$SUBSCRIPTION_BACKEND_PORT" ] \
    || die "SUBSCRIPTION_PORT=$SUBSCRIPTION_PORT collides with the local vpn-subscription backend. Choose a different public HTTPS port."
  export SUBSCRIPTION_PORT
}

check_ports_free() {
  resolve_subscription_port
  # Skip the check for ports vpn1 itself already owns on a re-run — an
  # already-installed vpn1 legitimately holds 443/tcp+udp and its
  # already-committed SUBSCRIPTION_PORT.
  if existing_install_present; then
    log "existing installation detected — skipping port-conflict checks (vpn1 owns these ports already)."
    return
  fi
  local failed=0
  preflight_check_port_free tcp 443 || failed=1
  preflight_check_port_free udp 443 || failed=1
  preflight_check_port_free tcp "$SUBSCRIPTION_PORT" || failed=1
  if [ "$failed" -eq 1 ]; then
    die "vpn1 cannot safely continue while a required port is occupied by another service. Free the port(s) above (or move the other service, or set SUBSCRIPTION_PORT=<free-port> to relocate the subscription HTTPS endpoint) and re-run."
  fi
  log "required ports (443/tcp, 443/udp, ${SUBSCRIPTION_PORT}/tcp) are free."
}

# When invoked via the curl|bash bootstrap, $REPO_ROOT points at a
# temporary extraction directory that the bootstrap deletes on exit —
# any path we print for later use (update.sh, uninstall.sh, a manual
# re-run) must survive past this process. Install a persistent copy of
# the source tree once, and repoint $REPO_ROOT at it for the rest of
# this run. A no-op when already running from that persistent copy
# (e.g. someone `cd /opt/vpn1 && ./deploy/almalinux/install.sh`).
PERSIST_DIR="/opt/vpn1"
OPT_VPN1_PRE_EXISTED=0
[ -e "$PERSIST_DIR" ] && OPT_VPN1_PRE_EXISTED=1
persist_source_tree() {
  if [ "$REPO_ROOT" = "$PERSIST_DIR" ]; then
    return
  fi
  log "installing a persistent copy of the vpn1 source to $PERSIST_DIR (for future updates/uninstall)..."
  mkdir -p "$PERSIST_DIR"
  ( cd "$REPO_ROOT" && tar --exclude=target --exclude=.git -cf - . ) | ( cd "$PERSIST_DIR" && tar -xf - )
  REPO_ROOT="$PERSIST_DIR"
}

acquire_installer_lock() {
  # Mutual exclusion against a concurrent install.sh/update.sh run
  # (docs/FINAL_PRODUCTION_AUDIT.md P0-4: "two concurrent administrators
  # must not lose each other's changes"). Deliberately a SEPARATE lock
  # file from vpn-admin's own /run/lock/vpn1.lock (apps/admin/src/
  # lock.rs) — this script shells out to `vpn-admin` multiple times
  # below, and each of those calls acquires that lock itself for its own
  # duration; holding the same lock around this whole script would
  # deadlock the instant it did so.
  mkdir -p /run/lock
  exec 200>/run/lock/vpn1-installer.lock
  # Bounded, and say so BEFORE blocking: an unbounded `flock` produced a
  # completely silent hang at stage 1 with no output at all. The lock fd is
  # inherited by children (dnf/cargo/certbot/curl), so an orphaned child
  # that outlives a `kill -9`'d installer keeps holding it — hence the
  # pointer to `fuser` in the failure message.
  if ! flock -x -w 600 200; then
    die "another install.sh/update.sh appears to be running and did not finish within 10 minutes.
If none is running, an orphaned child process may still be holding the lock:
  fuser -v /run/lock/vpn1-installer.lock
and kill whatever it reports before retrying."
  fi
  log "acquired installer lock (/run/lock/vpn1-installer.lock) — no other install.sh/update.sh can run concurrently."
}

# Best-effort: enables real Unicode->punycode conversion for an IDN
# domain (PUBLIC_HOST/SUBSCRIPTION_HOST/REALITY_HANDSHAKE_SERVER, e.g.
# чёрт.com). Optional and non-fatal — derive_punycode_host() already
# falls back to leaving non-ASCII input unchanged (which then correctly
# fails preflight_validate_hostname with a clear error, rather than
# silently mis-encoding it) when no converter is available, so a
# failure/absence here never blocks installation. Run once, early in
# preflight — BEFORE any operator-supplied hostname is normalized —
# rather than in packages_stage, since by the time packages_stage runs
# the hostnames must already be resolved and validated (see
# resolve_reality_handshake_server/resolve_host_config, now called from
# preflight_stage itself).
install_idn_support() {
  command -v idn2 >/dev/null 2>&1 && return 0
  local pkg=""
  case "$OS_FAMILY" in
    rhel) pkg="libidn2" ;;
    debian) pkg="idn2" ;;
    *) return 0 ;;
  esac
  case "$OS_FAMILY" in
    rhel) dnf install -y --setopt=install_weak_deps=False "$pkg" >/dev/null 2>&1 && ownership_list_add PKGS_INSTALLED_BY_VPN1 "$pkg" ;;
    debian) apt-get install -y --no-install-recommends "$pkg" >/dev/null 2>&1 && ownership_list_add PKGS_INSTALLED_BY_VPN1 "$pkg" ;;
  esac
  return 0
}

preflight_stage() {
  stage 1 "preflight"
  preflight_require_root
  acquire_installer_lock
  detect_os || die "unsupported operating system."
  log "detected OS: $OS_PRETTY_NAME (family=$OS_FAMILY, support=$OS_SUPPORT)"
  # Three OS_SUPPORT tiers (see deploy/lib/os.sh) get three distinct
  # messages — do not collapse "ci-tested" into either "tested" (overclaims)
  # or the generic "untested" warning (underclaims; it has real automated
  # coverage the generic message wouldn't mention).
  case "$OS_SUPPORT" in
    tested) ;;
    ci-tested)
      warn "this OS (Amazon Linux 2023) has automated static/unit test coverage (deploy/lib/tests/test-amazon-linux-2023.sh) but has NOT been verified end-to-end on a live host of this OS; continuing, but this is not the same guarantee as the tested matrix (AlmaLinux/Rocky/RHEL 9, Ubuntu 22.04/24.04, Debian 12/13)." ;;
    *)
      warn "this OS/version combination is not in the tested support matrix; continuing, but this is not a guarantee it works." ;;
  esac
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
    IS_FRESH_INSTALL=0
  else
    log "no existing installation detected — this will be a fresh install."
    IS_FRESH_INSTALL=1
  fi
  persist_source_tree
  # ---------------------------------------------------------------
  # Collect and validate EVERY required operator input and fatal
  # precondition HERE, before packages/certs/users/systemd/firewall/
  # sysctls/persistent files are touched. This is the fix for the
  # REALITY-decoy incident: REALITY_HANDSHAKE_SERVER (and PUBLIC_HOST)
  # used to only be validated in stage 10 (render_deployment_toml),
  # after packages, sing-box, users, directories and a real Let's
  # Encrypt certificate had already been created. Determine everything
  # up front instead, and fail immediately with zero host mutation if a
  # required value is missing/invalid in non-interactive mode.
  # ---------------------------------------------------------------
  install_idn_support
  resolve_host_config
  resolve_reality_handshake_server
  ownership_mark INSTALL_ATTEMPTED
  ownership_set_baseline_once OPT_VPN1_PRE_EXISTED "$OPT_VPN1_PRE_EXISTED"
  # Write the install-state manifest NOW (acceptance="installing"), not
  # only at the very end — a fatal failure at ANY later stage still
  # leaves repo_root/public_host/subscription_host/firewall_backend on
  # disk for the automatic-rollback trap (on_fatal_error) and for a
  # manual uninstall.sh run to find, instead of only self-describing
  # once the whole install already succeeded.
  write_install_state_manifest "installing"
}

# ---------------------------------------------------------------------
# [2] OS packages
# ---------------------------------------------------------------------
# Records, for every package in $1 (name-per-word), whether it was
# ALREADY installed before this run — via the ownership manifest's
# PKGS_INSTALLED_BY_VPN1 list — so uninstall.sh can later remove exactly
# the packages vpn1 introduced and leave everything the operator already
# had alone. Must be called with the exact package list BEFORE the
# package manager installs anything.
record_package_ownership_rhel() {
  local p
  for p in "$@"; do
    rpm -q "$p" >/dev/null 2>&1 || ownership_list_add PKGS_INSTALLED_BY_VPN1 "$p"
  done
}
record_package_ownership_debian() {
  local p
  for p in "$@"; do
    dpkg -s "$p" >/dev/null 2>&1 || ownership_list_add PKGS_INSTALLED_BY_VPN1 "$p"
  done
}

install_dependencies_rhel() {
  log "installing OS packages (dnf)..."
  local pkgs=(gcc gcc-c++ make pkgconf-pkg-config openssl-devel openssl
    firewalld policycoreutils-python-utils tar jq nginx certbot)
  # Amazon Linux 2023 ships `curl-minimal` preinstalled, providing
  # /usr/bin/curl already — it and the full `curl` package both own
  # /usr/bin/curl, so `dnf install curl` on top of it is a file conflict
  # that dnf refuses without --allowerasing (explicitly not allowed
  # here: it can silently remove/replace unrelated packages depending on
  # what else is installed, which is not an acceptable installer
  # shortcut). Same idempotency principle as everywhere else in this
  # script: only install a package if a working equivalent isn't already
  # present, rather than unconditionally forcing a specific package.
  if command -v curl >/dev/null 2>&1 && curl --version >/dev/null 2>&1; then
    log "usable curl already present ($(command -v curl)) — not adding 'curl' to the install list (avoids a curl-minimal/curl file conflict on Amazon Linux 2023 and similar images)."
  else
    pkgs+=(curl)
  fi
  # Baseline (once-ever) capture of pre-existing state, BEFORE any
  # package is installed or firewalld/nginx/certbot state changes —
  # uninstall.sh restores exactly these, never guesses them.
  ownership_set_baseline_once NGINX_PRE_INSTALLED "$(command -v nginx >/dev/null 2>&1 && echo 1 || echo 0)"
  ownership_set_baseline_once FIREWALLD_PRE_INSTALLED "$(command -v firewall-cmd >/dev/null 2>&1 && echo 1 || echo 0)"
  ownership_set_baseline_once FIREWALLD_PRE_ENABLED "$(systemctl is-enabled --quiet firewalld 2>/dev/null && echo 1 || echo 0)"
  record_package_ownership_rhel "${pkgs[@]}"
  dnf install -y --setopt=install_weak_deps=False "${pkgs[@]}" >/dev/null
  systemctl enable --now firewalld >/dev/null
}

install_dependencies_debian() {
  log "installing OS packages (apt)..."
  export DEBIAN_FRONTEND=noninteractive
  local pkgs=(build-essential pkg-config libssl-dev ufw tar curl jq nginx certbot ca-certificates)
  ownership_set_baseline_once NGINX_PRE_INSTALLED "$(command -v nginx >/dev/null 2>&1 && echo 1 || echo 0)"
  ownership_set_baseline_once UFW_PRE_ENABLED "$(ufw status 2>/dev/null | grep -q 'Status: active' && echo 1 || echo 0)"
  record_package_ownership_debian "${pkgs[@]}"
  apt-get update -y >/dev/null
  apt-get install -y --no-install-recommends "${pkgs[@]}" >/dev/null
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

derive_punycode_host() {
  # Best-effort ASCII/punycode normalization for a domain typed at the
  # interactive prompt (Cyrillic/other IDN input is common when copying
  # a domain out of a DNS dashboard). Falls back to the raw input
  # unchanged if no IDN converter is available or it is already ASCII —
  # preflight_validate_hostname() rejects whatever comes out of this if
  # it still isn't a syntactically valid ASCII hostname.
  local input="$1"
  case "$input" in
    *[!\ -~]*)
      # Force a UTF-8 locale for the converter regardless of this host's
      # own locale settings — a non-UTF-8 locale (common on a minimal
      # VPS image) makes idn2/idn fail to even parse the input, which
      # would otherwise silently fall through to returning the
      # unconverted Unicode string (and then correctly, but confusingly,
      # fail hostname validation instead of being converted).
      if command -v idn2 >/dev/null 2>&1; then
        LC_ALL=C.UTF-8 idn2 "$input" 2>/dev/null || LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 idn2 "$input" 2>/dev/null || echo "$input"
      elif command -v idn >/dev/null 2>&1; then
        LC_ALL=C.UTF-8 idn --quiet -a "$input" 2>/dev/null || LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 idn --quiet -a "$input" 2>/dev/null || echo "$input"
      else
        echo "$input"
      fi
      ;;
    *)
      echo "$input"
      ;;
  esac
}

# Interactively ask for a domain when none was supplied via
# PUBLIC_HOST/SUBSCRIPTION_HOST env vars and stdin isn't already the
# install script itself (the classic `curl | sudo bash` problem: bash's
# stdin is the piped script, not the terminal, so a plain `read` would
# silently read from the script body instead of the user). Reading from
# /dev/tty works even when invoked that way, as long as a real terminal
# is attached; non-interactive/CI invocations (no /dev/tty) fall back to
# the sslip.io auto-assigned hostname with an explicit log line, never
# silently hang.
prompt_for_public_host() {
  if [ ! -r /dev/tty ] || [ ! -w /dev/tty ]; then
    return 1
  fi
  local reply=""
  printf '\nUse your own domain for this VPN instead of an auto-assigned sslip.io hostname?\n' >/dev/tty
  printf 'Point its DNS A/AAAA record at this server first, then enter it (or press Enter to skip): ' >/dev/tty
  IFS= read -r reply </dev/tty || return 1
  [ -n "$reply" ] || return 1
  reply="$(derive_punycode_host "$reply")"
  printf '%s' "$reply"
}

resolve_host_config() {
  if load_existing_host_config; then
    log "using host configuration from existing $DEPLOYMENT_TOML: PUBLIC_HOST=$PUBLIC_HOST SUBSCRIPTION_HOST=$SUBSCRIPTION_HOST"
    export PUBLIC_HOST SUBSCRIPTION_HOST
    return
  fi

  local operator_supplied=0
  if [ -n "${PUBLIC_HOST:-}" ]; then
    operator_supplied=1
    PUBLIC_HOST="$(derive_punycode_host "$PUBLIC_HOST")"
  elif [ "$NONINTERACTIVE" -ne 1 ]; then
    local prompted
    prompted="$(prompt_for_public_host)" || prompted=""
    if [ -n "$prompted" ]; then
      PUBLIC_HOST="$prompted"
      operator_supplied=1
    fi
  fi

  if [ "$operator_supplied" -eq 1 ]; then
    log "using operator-supplied PUBLIC_HOST=$PUBLIC_HOST"
  else
    log "no PUBLIC_HOST set — detecting public IP for zero-touch install..."
    PUBLIC_IP="$(preflight_detect_public_ip)" || die "could not auto-detect this server's public IP. Re-run with PUBLIC_HOST=your.domain.com (or --domain your.domain.com) set explicitly."
    log "detected public IP: $PUBLIC_IP"
    PUBLIC_HOST="$(derive_auto_host "$PUBLIC_IP")"
    log "auto-assigned hostname: $PUBLIC_HOST (resolves to $PUBLIC_IP via sslip.io, no DNS setup needed)"
  fi
  SUBSCRIPTION_HOST="$(derive_punycode_host "${SUBSCRIPTION_HOST:-$PUBLIC_HOST}")"
  preflight_validate_hostname "$PUBLIC_HOST" "PUBLIC_HOST" || die "invalid PUBLIC_HOST — refusing to interpolate it into deployment.toml/nginx config."
  preflight_validate_hostname "$SUBSCRIPTION_HOST" "SUBSCRIPTION_HOST" || die "invalid SUBSCRIPTION_HOST — refusing to interpolate it into deployment.toml/nginx config."
  # A domain the operator explicitly typed/passed (as opposed to the
  # auto-assigned sslip.io hostname, which is correct by construction —
  # it literally encodes this host's own detected IP) must actually
  # resolve to THIS server before we go on to open firewall ports and
  # request a certificate for it. A mismatch here means either stale
  # DNS, a domain still pointed at a previous host, or the domain is
  # fronted by a proxy/CDN whose edge terminates TLS instead of this
  # VPS — REALITY/Hysteria2 need to terminate the raw TCP/UDP connection
  # themselves, so a CDN in front of it will never work correctly even
  # if DNS/cert issuance somehow succeeded anyway.
  if [ "$operator_supplied" -eq 1 ]; then
    local dns_rc=0
    preflight_check_hostname_resolves_here "$PUBLIC_HOST" || dns_rc=$?
    case "$dns_rc" in
      0) ;;
      1) die "PUBLIC_HOST=$PUBLIC_HOST does not resolve to this server. Point its DNS A/AAAA record directly at this VPS's public IP (not through a proxy/CDN — REALITY and Hysteria2 must terminate the raw connection on THIS host) and re-run once DNS has propagated. Refusing to open firewall ports or request a certificate for a domain that resolves elsewhere." ;;
      2) warn "could not conclusively verify that PUBLIC_HOST=$PUBLIC_HOST resolves to this server (no DNS tool, or DNS returned nothing yet) — continuing, but certificate issuance will fail later if it doesn't." ;;
    esac
  fi
  export PUBLIC_HOST SUBSCRIPTION_HOST
}

host_config_stage() {
  stage 3 "host configuration"
  # PUBLIC_HOST/SUBSCRIPTION_HOST were already resolved and validated in
  # preflight_stage (stage 1) — required so packages/certs/users are
  # never touched before a fatal input problem (bad hostname, DNS not
  # pointed here) is caught. This stage just confirms the values in the
  # per-stage log for operators following along.
  log "host configuration: PUBLIC_HOST=$PUBLIC_HOST SUBSCRIPTION_HOST=$SUBSCRIPTION_HOST"
}

# ---------------------------------------------------------------------
# REALITY decoy handshake server — resolved and validated in preflight
# (stage 1), BEFORE packages/sing-box/users/directories/certificates are
# ever touched. This is the fix for the incident this task was filed
# about: the installer used to only discover a missing
# REALITY_HANDSHAKE_SERVER deep in stage 10 (render_deployment_toml),
# after a real Let's Encrypt certificate had already been issued and
# every package/user/directory already created.
#
# There is intentionally NO universally safe default decoy — a bad
# choice (a host that blocks/detects REALITY probing, or one the
# operator doesn't actually control/intend) is a real security/
# reliability footgun, so this never silently invents one. Interactive
# mode asks once, early. Non-interactive mode requires
# --reality-handshake-server/REALITY_HANDSHAKE_SERVER and fails
# immediately, with zero host mutation, if it's absent.
# ---------------------------------------------------------------------
resolve_reality_handshake_server() {
  # A re-run against an already-committed deployment.toml already has a
  # validated, working value baked in — render_deployment_toml() leaves
  # an existing file untouched, so requiring the env var again here
  # would turn a harmless repair re-run into a hard failure.
  if [ -f "$DEPLOYMENT_TOML" ]; then
    return
  fi
  if [ -z "${REALITY_HANDSHAKE_SERVER:-}" ] && [ "$NONINTERACTIVE" -ne 1 ] \
      && [ -r /dev/tty ] && [ -w /dev/tty ]; then
    local reply=""
    printf '\nREALITY needs a TLS 1.3 "decoy" hostname — a real, already-running TLS\n1.3 server that you control or have deliberately chosen (NOT this VPS\nitself). There is no universally safe default.\nEnter a REALITY handshake server hostname (e.g. a domain you already run TLS 1.3 on): ' >/dev/tty
    IFS= read -r reply </dev/tty || reply=""
    [ -n "$reply" ] && REALITY_HANDSHAKE_SERVER="$reply"
  fi
  REALITY_HANDSHAKE_SERVER="$(derive_punycode_host "${REALITY_HANDSHAKE_SERVER:-}")"
  : "${REALITY_HANDSHAKE_SERVER:?REALITY_HANDSHAKE_SERVER is required and was not supplied. Pass --reality-handshake-server <host> (or set REALITY_HANDSHAKE_SERVER=<host>) to a TLS 1.3 hostname you control or have explicitly selected as the REALITY decoy. There is no universally safe default — vpn1 refuses to guess one, and refuses to make ANY change to this host without it. See docs/ALMALINUX_DEPLOYMENT.md for how to choose a decoy.}"
  preflight_validate_hostname "$REALITY_HANDSHAKE_SERVER" "REALITY_HANDSHAKE_SERVER" \
    || die "invalid REALITY_HANDSHAKE_SERVER — refusing to make any host changes."
  # Best-effort "is this candidate actually usable for REALITY" check,
  # as far as practical BEFORE any mutation. This is deliberately not
  # the full acceptance gate — the real go/no-go is stage 17's
  # 'vpn doctor --protocol' real sing-box handshake self-test, which
  # needs sing-box/keys/config that don't exist yet at this point — but
  # a quick outbound TLS 1.3 probe catches an obviously wrong/unreachable
  # host early, before packages/certs/users are touched.
  if command -v openssl >/dev/null 2>&1; then
    local probe_out
    probe_out="$(mktemp)"
    if timeout 8 openssl s_client -connect "${REALITY_HANDSHAKE_SERVER}:443" \
        -tls1_3 -servername "$REALITY_HANDSHAKE_SERVER" </dev/null >"$probe_out" 2>&1; then
      if grep -q 'Protocol.*TLSv1.3' "$probe_out"; then
        log "REALITY_HANDSHAKE_SERVER=$REALITY_HANDSHAKE_SERVER answered with TLS 1.3 on 443/tcp — looks usable as a decoy."
      else
        warn "REALITY_HANDSHAKE_SERVER=$REALITY_HANDSHAKE_SERVER is reachable on 443/tcp but did not confirm TLS 1.3 in this probe — REALITY requires a genuine TLS 1.3 server. Double-check this is really the host you intend before continuing (the real acceptance gate runs later, at the end of install)."
      fi
    else
      warn "could not reach REALITY_HANDSHAKE_SERVER=$REALITY_HANDSHAKE_SERVER on 443/tcp from this host (egress firewall, or the target itself, may be the cause). Continuing — the end-of-install protocol self-test is the real acceptance gate, but a decoy unreachable from this VPS will fail that test too."
    fi
    rm -f "$probe_out"
  fi
  export REALITY_HANDSHAKE_SERVER
}

# ---------------------------------------------------------------------
# [4] vpn1 binaries (prebuilt release, falling back to source build)
# ---------------------------------------------------------------------
fetch_release_binaries() {
  local target version base_url tmp
  target="$(rust_target_for_arch "$ARCH")" || return 1
  # The bootstrapper may intentionally download main/dev source, or fall
  # back to it when release-tag resolution is unavailable. In either case
  # VPN1_VERSION is empty and a "latest" binary would silently mix two
  # different revisions. Prebuilt assets are allowed only for the exact,
  # immutable tag the bootstrapper resolved.
  version="${VPN1_VERSION:-}"
  if [ -z "$version" ]; then
    log "no exact release tag is pinned — building binaries from this downloaded source tree."
    return 1
  fi
  base_url="https://github.com/$VPN1_RELEASE_REPO/releases/download/$version"
  tmp="$(mktemp -d)"
  local asset="vpn1-${target}.tar.gz"
  log "checking for a prebuilt release ($asset)..."
  if ! curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tmp/$asset" "$base_url/$asset" 2>/dev/null; then
    log "no prebuilt release available (this is expected until a release is tagged) — falling back to building from source."
    rm -rf "$tmp"
    return 1
  fi
  if curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tmp/SHA256SUMS" "$base_url/SHA256SUMS" 2>/dev/null; then
    ( cd "$tmp" && sha256sum --ignore-missing -c SHA256SUMS ) || die "checksum verification failed for $asset — refusing to install unverified binaries."
    log "checksum verified against release SHA256SUMS."
  else
    die "release asset $asset was found but SHA256SUMS was not — refusing to install a binary with no integrity verification."
  fi
  tar -xzf "$tmp/$asset" -C "$tmp"
  # release.yml packages binaries inside a top-level "vpn1-<target>/"
  # directory (the conventional tarball layout — avoids extracting loose
  # files into a shared tmp dir). Keep this extraction path and the
  # workflow's packaging step in lockstep: see the "release archive
  # contract" smoke test in .github/workflows/release.yml, which extracts
  # a real built archive with this exact same relative path and fails CI
  # if they ever diverge again (docs/FINAL_PRODUCTION_AUDIT.md P0-6).
  local extracted="$tmp/vpn1-${target}"
  [ -d "$extracted" ] || die "release asset $asset did not contain the expected vpn1-${target}/ directory — archive layout does not match what install.sh expects. This is a packaging bug, not a transient failure; see docs/FINAL_PRODUCTION_AUDIT.md P0-6."
  install -m 0755 "$extracted/vpn-admin" "$BIN_DIR/vpn-admin"
  install -m 0755 "$extracted/vpn-admin" "$BIN_DIR/vpn"
  install -m 0755 "$extracted/subscription" "$BIN_DIR/vpn-subscription-svc"
  rm -rf "$tmp"
  log "installed prebuilt vpn1 $version binaries ($target) — no Rust compiler needed."
  return 0
}

install_rustup_noninteractive() {
  log "cargo not found; installing a Rust toolchain via rustup (no prebuilt release was available)..."
  # Same stalled-transfer protection as every other download here: a
  # connection that is established and then goes quiet is not a failed
  # transfer, so `--max-time`/`--speed-limit` are what actually bound it.
  # This path was missed when the other call sites were fixed, and it is
  # the one that runs whenever no prebuilt release is available — i.e. the
  # default today.
  local rustup_script
  rustup_script="$(mktemp)"
  curl --proto '=https' --tlsv1.2 -sSf "${CURL_NET_FLAGS[@]}" \
    -o "$rustup_script" https://sh.rustup.rs
  # The downloaded bootstrap starts additional transfers of its own. Bound
  # the whole child, not only the first curl, so a stalled rustup-init fetch
  # cannot hang installation indefinitely.
  if ! timeout 900 sh "$rustup_script" -y --profile minimal \
      --default-toolchain stable >/dev/null; then
    rm -f "$rustup_script"
    die "rustup installation failed or exceeded its 15-minute hard deadline"
  fi
  rm -f "$rustup_script"
  # Track that VPN1 (not the operator) introduced this toolchain, so
  # uninstall can remove it later — but only when no toolchain was
  # already present (checked by build_binaries_from_source before it
  # ever calls this function).
  ownership_mark RUSTUP_INSTALLED_BY_VPN1
  ownership_set RUSTUP_HOME_DIR "$HOME"
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
}

build_binaries_from_source() {
  # A `curl | sudo bash` session starts with a minimal PATH that never
  # includes `~/.cargo/bin`, so `command -v cargo` fails even when an
  # earlier run of this same script already installed a toolchain via
  # rustup — that earlier install just isn't on PATH *yet*. Source
  # rustup's own env file first so an already-installed toolchain is
  # actually found before falling back to a full network reinstall
  # (which, being unnecessary AND a real network round-trip, is exactly
  # the kind of thing that can hang on a flaky connection for no reason).
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
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
  ownership_set_baseline_once SINGBOX_BIN_PRE_EXISTED "$([ -e "$SINGBOX_BIN" ] && echo 1 || echo 0)"
  local tmpdir tarball
  tmpdir="$(mktemp -d)"
  tarball="sing-box-${SINGBOX_VERSION}-linux-${ARCH}.tar.gz"
  local url="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/${tarball}"
  log "downloading pinned sing-box ${SINGBOX_VERSION} (${ARCH}) from official release assets..."
  curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tmpdir/$tarball" "$url" || die "download failed: $url"

  # Verify integrity before extracting/installing ANYTHING. Preferred:
  # upstream's own published checksums.txt when it exists for this
  # release. Fallback: a pinned expected digest for this exact
  # version+arch, hand-verified against the real upstream asset bytes
  # (see SINGBOX_SHA256_* above). If neither is available, abort —
  # printing a self-computed hash and calling that "verification" is not
  # verification (docs/FINAL_PRODUCTION_AUDIT.md P0-8); never silently
  # downgrade to an unverified install.
  local sums_url="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/sing-box_${SINGBOX_VERSION}_checksums.txt"
  local actual_sha256 expected_sha256=""
  if curl -fsSL "${CURL_NET_FLAGS[@]}" -o "$tmpdir/checksums.txt" "$sums_url" 2>/dev/null; then
    ( cd "$tmpdir" && sha256sum --ignore-missing -c checksums.txt ) || die "checksum verification failed for $tarball (upstream checksums.txt) — refusing to install."
    log "checksum verified against upstream checksums.txt."
  else
    case "$ARCH" in
      amd64) expected_sha256="$SINGBOX_SHA256_AMD64" ;;
      arm64) expected_sha256="$SINGBOX_SHA256_ARM64" ;;
    esac
    [ -n "$expected_sha256" ] || die "no upstream checksums.txt and no pinned expected SHA256 for sing-box ${SINGBOX_VERSION}/${ARCH} — refusing to install an unverified binary. Update SINGBOX_SHA256_* in this script if you intentionally changed SINGBOX_VERSION."
    actual_sha256="$(sha256sum "$tmpdir/$tarball" | awk '{print $1}')"
    [ "$actual_sha256" = "$expected_sha256" ] || die "checksum verification failed for $tarball: expected $expected_sha256, got $actual_sha256 — refusing to install a binary that does not match the pinned digest."
    log "checksum verified against pinned expected SHA256 (no upstream checksums.txt published for this release)."
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
# [6] systemd
# ---------------------------------------------------------------------
# Installed early (right after sing-box itself, well before the config
# render/reload in stage 11) so that any fix to these unit files is
# already on disk before anything ever tries to reload the service
# against them. Getting this ordering wrong is not hypothetical: it
# broke a real upgrade run where a stale, already-broken unit file from
# an earlier partial install was still in place when stage 11's
# `vpn-admin render-config` attempted `systemctl reload-or-restart
# sing-box` against it, hard-failing the whole installer even though
# the corrected unit was one stage away from being installed. These
# files are static installer-owned templates with no placeholders and
# no dependency on state created by any other stage, so installing them
# this early is safe.
install_systemd_units() {
  log "installing systemd units..."
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/sing-box.service" /etc/systemd/system/sing-box.service
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/vpn-subscription.service" /etc/systemd/system/vpn-subscription.service
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/vpn-expiry-reconcile.service" /etc/systemd/system/vpn-expiry-reconcile.service
  install -m 0644 "$REPO_ROOT/deploy/almalinux/systemd/vpn-expiry-reconcile.timer" /etc/systemd/system/vpn-expiry-reconcile.timer
  systemctl daemon-reload
}

systemd_stage() {
  stage 6 "systemd"
  install_systemd_units
}

# ---------------------------------------------------------------------
# [7] users/groups
# ---------------------------------------------------------------------
create_service_users() {
  id vpn-subscription >/dev/null 2>&1 || { useradd --system --no-create-home --shell /usr/sbin/nologin vpn-subscription; ownership_mark USER_VPNSUB_CREATED; }
  id sing-box >/dev/null 2>&1 || { useradd --system --no-create-home --shell /usr/sbin/nologin sing-box; ownership_mark USER_SINGBOX_CREATED; }
  # Shared traversal-only group: neither service's *files* become
  # readable by the other (that's still controlled by per-file group
  # ownership below), but both services' primary groups need to walk
  # through $STATE_DIR and $STATE_DIR/reality, which are shared by files
  # belonging to both services. Without this, sing-box (Group=sing-box)
  # cannot even stat() its way down to sing-box/config.json through a
  # vpn-subscription-owned parent, and vice versa for reality/public.key.
  getent group vpn-compat >/dev/null 2>&1 || { groupadd --system vpn-compat; ownership_mark GROUP_VPNCOMPAT_CREATED; }
  usermod -aG vpn-compat sing-box
  usermod -aG vpn-compat vpn-subscription
}

users_groups_stage() {
  stage 7 "users/groups"
  create_service_users
}

# ---------------------------------------------------------------------
# [8] directories
# ---------------------------------------------------------------------
# Ownership matrix (docs/PRODUCTION_HARDENING_PLAN.md #1, revised —
# docs/FINAL_PRODUCTION_AUDIT.md P0-1): every FILE is owned by the group
# that actually needs to read its contents; every DIRECTORY that is a
# shared parent for files belonging to more than one service is owned by
# the shared `vpn-compat` traversal group instead, with the setgid bit so
# atomic replacements (rename()) of files inside always inherit the
# right group even though a rename() itself never changes ownership
# (crates/compat-config/src/store.rs and server.rs additionally fchown
# each written file explicitly — belt and suspenders, since setgid only
# affects *newly created* files, not renamed-in ones).
#
#   $STATE_DIR            root:vpn-compat 02750  (both services traverse)
#   reality/               root:vpn-compat 02750  (both services traverse)
#     private.key          root:sing-box    0640  (sing-box only)
#     public.key           root:vpn-subscription 0640
#     short_id.txt          root:vpn-subscription 0640
#     hysteria_obfs_password.txt root:vpn-subscription 0640
#       (deliberately lives here, not under hysteria/, so
#       vpn-subscription can read it — see
#       compat_config::deployment::hysteria_obfs_password_file's doc
#       comment; hysteria/ itself stays sing-box-only below)
#   hysteria/               root:sing-box   02750  (sing-box only)
#   users/                  root:vpn-subscription 02750  (vpn-subscription only)
#   sing-box/               root:sing-box   02750  (sing-box only)
create_directories() {
  install -d -m 0755 /etc/vpn
  install -d -m 02750 -o root -g vpn-compat "$STATE_DIR"
  install -d -m 02750 -o root -g vpn-compat "$STATE_DIR/reality"
  install -d -m 02750 -o root -g sing-box "$STATE_DIR/hysteria"
  # vpn-subscription reads users.json (to verify tokens) but never
  # writes it — only vpn-admin (run as root) does. Group-readable, not
  # group-writable.
  install -d -m 02750 -o root -g vpn-subscription "$STATE_DIR/users"
  install -d -m 02750 -o sing-box -g sing-box "$STATE_DIR/sing-box"
}

directories_stage() {
  stage 8 "directories"
  create_directories
}

# ---------------------------------------------------------------------
# [9] certificates
# ---------------------------------------------------------------------
# Hysteria2 requires a valid TLS cert/key BEFORE sing-box can start. We
# always attempt to issue a real certificate automatically via certbot's
# HTTP-01 challenge — for the auto-assigned sslip.io hostname (stage 3)
# just as much as for an operator-supplied PUBLIC_HOST whose DNS record
# already points at this server (attempt_automatic_certbot's own guards
# — certbot installed, port 80 free, firewall opened/closed cleanly
# either way — make this safe to attempt unconditionally; a
# misconfigured/not-yet-propagated custom domain just fails the HTTP-01
# challenge and falls through to the manual instructions below, exactly
# like any other automatic-issuance failure). This only requires
# outbound HTTPS + port 80 briefly free, both true on a fresh VPS. If it
# fails (egress blocked, port 80 unavailable, DNS not pointed here yet,
# etc.) we stop with the exact manual recovery commands rather than
# silently degrading to a cert that client apps will reject.
# Temporarily allow inbound TCP/80 for certbot's HTTP-01 challenge, and
# remove EXACTLY that rule again afterwards — never touching any
# pre-existing firewall rule vpn1 did not add itself
# (docs/FINAL_PRODUCTION_AUDIT.md P0-9). firewalld/ufw are already
# enabled by packages_stage (stage 2) with distro-default deny rules by
# the time this runs, and vpn1's own permanent rules (443/tcp+udp plus
# SUBSCRIPTION_PORT, never 80) aren't added until firewall_stage (stage
# 13) — so without
# this, certbot's challenge has no way to receive inbound traffic.
TEMP_PORT80_ADDED=0
firewall_open_port_80_temp() {
  TEMP_PORT80_ADDED=0
  case "$FIREWALL_BACKEND" in
    firewalld)
      command -v firewall-cmd >/dev/null 2>&1 || return 1
      systemctl is-active --quiet firewalld 2>/dev/null || return 1
      firewall-cmd --query-port=80/tcp >/dev/null 2>&1 && return 0
      firewall-cmd --add-port=80/tcp >/dev/null 2>&1 # runtime-only, not --permanent: gone on next reload/reboot even if cleanup below is skipped
      TEMP_PORT80_ADDED=1
      ;;
    ufw)
      command -v ufw >/dev/null 2>&1 || return 1
      ufw status 2>/dev/null | grep -Eq '^80/tcp[[:space:]]+ALLOW' && return 0
      ufw allow 80/tcp >/dev/null 2>&1
      TEMP_PORT80_ADDED=1
      ;;
    *) return 1 ;;
  esac
}

firewall_close_port_80_temp() {
  case "$FIREWALL_BACKEND" in
    firewalld)
      command -v firewall-cmd >/dev/null 2>&1 || return 0
      firewall-cmd --remove-port=80/tcp >/dev/null 2>&1 || true
      ;;
    ufw)
      command -v ufw >/dev/null 2>&1 || return 0
      ufw delete allow 80/tcp >/dev/null 2>&1 || true
      ;;
  esac
}

attempt_automatic_certbot() {
  local host="$1" opened_port_80=0
  command -v certbot >/dev/null 2>&1 || { warn "certbot not installed; cannot auto-provision a certificate for $host."; return 1; }
  # Stop nginx FIRST, then check. On the Debian family `apt-get install
  # nginx` starts nginx immediately with a default :80 vhost, so checking
  # before stopping made this return 1 on every fresh Ubuntu/Debian install
  # — the certificate stage then aborted the whole installer, and nginx was
  # left running so a re-run failed identically. `certificates_stage`
  # restarts nginx afterwards on the success path; `nginx_was_stopped`
  # records that we are responsible for it on the failure paths too.
  local nginx_was_stopped=0
  acme_cleanup() {
    if [ "$opened_port_80" -eq 1 ]; then
      firewall_close_port_80_temp
      opened_port_80=0
      log "removed the temporary TCP/80 rule vpn1 added for the ACME challenge."
    fi
    if [ "$nginx_was_stopped" -eq 1 ]; then
      systemctl start nginx >/dev/null 2>&1 || warn "could not restore nginx after ACME attempt"
      nginx_was_stopped=0
    fi
  }
  trap 'acme_cleanup; exit 130' INT
  trap 'acme_cleanup; exit 143' TERM
  if systemctl is-active --quiet nginx 2>/dev/null; then
    systemctl stop nginx 2>/dev/null || true
    nginx_was_stopped=1
  fi
  if ! preflight_check_port_free tcp 80 >/dev/null 2>&1; then
    warn "port 80/tcp is occupied by something other than nginx; certbot's standalone HTTP-01 challenge cannot run automatically for $host."
    acme_cleanup
    trap - INT TERM
    return 1
  fi
  if firewall_open_port_80_temp; then
    opened_port_80="$TEMP_PORT80_ADDED"
    [ "$opened_port_80" -eq 1 ] \
      && log "temporarily allowed inbound TCP/80 for the ACME HTTP-01 challenge."
  fi
  log "requesting a Let's Encrypt certificate for $host via certbot (HTTP-01, standalone)..."
  local rc=0 certbot_output
  certbot_output="$(certbot certonly --standalone -d "$host" --non-interactive --agree-tos \
      -m "admin@$host" --no-eff-email 2>&1)" || rc=$?
  acme_cleanup
  trap - INT TERM
  if [ "$rc" -eq 0 ]; then
    log "certificate issued for $host."
    ownership_list_add CERT_LINEAGES_CREATED_BY_VPN1 "$host"
    return 0
  fi
  # Local port 80 being free (checked above) is NOT the same as port 80
  # being reachable from the internet — HTTP-01 requires Let's Encrypt's
  # servers to actually connect in from outside. vpn1 only manages the
  # HOST firewall (firewalld/ufw); it cannot see or touch a cloud
  # provider's separate network-level firewall (AWS security groups,
  # etc.), and a security group with no inbound TCP/80 rule silently
  # blocks the challenge with no error on this host at all — the most
  # common real-world cause of this failure on a fresh cloud VPS.
  warn "automatic certbot issuance failed for $host."
  cat >&2 <<EOF
[install] Common causes, in likely order:
  1. DNS for '$host' does not yet resolve to THIS server's public IP
     (check: dig +short $host   — or: getent hosts $host).
  2. TCP/80 is free locally (already checked) but blocked from the
     PUBLIC internet by your cloud provider's separate network firewall
     (e.g. an AWS/EC2 security group, a GCP firewall rule, an Azure NSG)
     — vpn1 only manages this host's own firewall (firewalld/ufw) and
     cannot see or change a cloud-provider-level firewall.
     For automatic certificate issuance (Let's Encrypt HTTP-01), TCP/80
     must be reachable from the public Internet (0.0.0.0/0) — Let's
     Encrypt's validation servers do not originate from your own IP, so
     restricting the rule to your IP will NOT make issuance succeed.
     If you only want to manually verify reachability from your own
     machine first, a rule scoped to your IP is fine for that test
     alone, but must be widened to 0.0.0.0/0 (or removed) before
     certbot will succeed. Add the appropriate inbound rule in that
     provider's console, then re-run.
  3. Verify external reachability directly, from a DIFFERENT machine
     (not this server — a check from here would only prove loopback
     works):
       curl -sS --max-time 5 http://$host/ -o /dev/null -w '%{http_code}\n'
     Any HTTP response (even 404) means TCP/80 is reachable from
     outside; a timeout/connection-refused means it is not.
  4. certbot's own log has the exact reason:
       tail -n 50 /var/log/letsencrypt/letsencrypt.log

Certbot output:
$certbot_output
EOF
  return 1
}

ensure_tls_material() {
  local host="$1"
  local le_cert="/etc/letsencrypt/live/$host/fullchain.pem"
  if [ -s "$le_cert" ]; then
    return 0
  fi
  attempt_automatic_certbot "$host" && return 0
  return 1
}

require_hysteria_tls() {
  local cert="$STATE_DIR/hysteria/cert.pem"
  local key="$STATE_DIR/hysteria/key.pem"
  local lineage="/etc/letsencrypt/live/$PUBLIC_HOST"
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    if ensure_tls_material "$PUBLIC_HOST"; then
      install -d -m 02750 -o root -g sing-box "$STATE_DIR/hysteria"
      install -m 0640 -o root -g sing-box \
        "/etc/letsencrypt/live/$PUBLIC_HOST/fullchain.pem" "$cert"
      install -m 0640 -o root -g sing-box \
        "/etc/letsencrypt/live/$PUBLIC_HOST/privkey.pem" "$key"
    fi
  elif ! openssl x509 -in "$cert" -noout -checkend 0 >/dev/null 2>&1 \
      && [ -s "$lineage/fullchain.pem" ] && [ -s "$lineage/privkey.pem" ] \
      && openssl x509 -in "$lineage/fullchain.pem" -noout -checkend 0 >/dev/null 2>&1; then
    log "refreshing stale Hysteria2 copy from the current certbot lineage..."
    if [ -f "$STATE_DIR/sing-box/config.json" ] \
        && systemctl cat sing-box.service >/dev/null 2>&1; then
      RENEWED_LINEAGE="$lineage" RENEWED_DOMAINS="$PUBLIC_HOST" \
        "$REPO_ROOT/deploy/almalinux/certbot-deploy-hook.sh"
    else
      install -m 0640 -o root -g sing-box "$lineage/fullchain.pem" "$cert"
      install -m 0640 -o root -g sing-box "$lineage/privkey.pem" "$key"
    fi
  fi
  if [ ! -s "$cert" ] || [ ! -s "$key" ]; then
    cat >&2 <<EOF
[install] ERROR: Hysteria2 TLS certificate/key missing.

  Hysteria2 TLS certificate missing: $cert
  Hysteria2 TLS key missing:         $key

Automatic issuance (certbot HTTP-01) already failed above — see that
output for the specific reason. In summary, ALL of the following must be
true simultaneously for automatic issuance to succeed:
  - DNS for ${PUBLIC_HOST:-<PUBLIC_HOST>} resolves to this server's public IP.
  - TCP/80 is reachable from the public internet, not just locally free.
    vpn1 only manages the HOST firewall (firewalld/ufw) — a cloud
    provider's separate network firewall/security group (AWS, GCP,
    Azure, ...) may ALSO need an inbound TCP/80 rule; vpn1 cannot see or
    change that layer. Test from a DIFFERENT machine:
      curl -sS --max-time 5 http://${PUBLIC_HOST:-<PUBLIC_HOST>}/ -o /dev/null -w '%{http_code}\n'
  - certbot is installed and nothing else is holding TCP/80.

Provision a certificate for ${PUBLIC_HOST:-<PUBLIC_HOST>} manually once the above are confirmed, e.g. with certbot:

  certbot certonly --standalone -d ${PUBLIC_HOST:-<PUBLIC_HOST>} \\
    --non-interactive --agree-tos -m admin@${PUBLIC_HOST:-<PUBLIC_HOST>}

Then copy/link the issued files into place:

  install -d -m 02750 -o root -g sing-box $STATE_DIR/hysteria
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


# Installs the deploy hook that keeps $STATE_DIR/hysteria/{cert,key}.pem
# in sync across renewals (docs/FINAL_PRODUCTION_AUDIT.md P0-11), and
# makes sure certbot's own renewal timer is actually enabled — a
# certificate that renews correctly but whose timer never runs is just a
# slower version of the same "silently dies at ~90 days" bug.
install_certbot_renewal_hook() {
  command -v certbot >/dev/null 2>&1 || return 0
  install -d -m 0755 /etc/letsencrypt/renewal-hooks/deploy
  install -m 0755 "$REPO_ROOT/deploy/almalinux/certbot-deploy-hook.sh" \
    /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh
  log "installed certbot renewal deploy hook: /etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh"
  local renewal_unit="" candidate
  for candidate in certbot.timer certbot-renew.timer snap.certbot.renew.timer; do
    if systemctl cat "$candidate" >/dev/null 2>&1; then
      renewal_unit="$candidate"
      break
    fi
  done
  if [ -n "$renewal_unit" ]; then
    ownership_set_baseline_once CERTBOT_TIMER_UNIT "$renewal_unit"
    ownership_set_baseline_once CERTBOT_TIMER_PRE_ENABLED "$(systemctl is-enabled --quiet "$renewal_unit" 2>/dev/null && echo 1 || echo 0)"
    if systemctl enable --now "$renewal_unit" >/dev/null 2>&1; then
      log "$renewal_unit enabled."
    else
      warn "could not enable $renewal_unit — certificates will not auto-renew."
    fi
  else
    warn "no certbot renewal timer unit found (certbot.timer / certbot-renew.timer / snap.certbot.renew.timer). Verify with 'certbot renew --dry-run'."
  fi
}

certificates_stage() {
  stage 9 "certificates"
  create_hysteria_masquerade_placeholder
  require_hysteria_tls
  require_subscription_tls
  install_certbot_renewal_hook
  if command -v nginx >/dev/null 2>&1; then systemctl start nginx >/dev/null 2>&1 || true; fi
}

# ---------------------------------------------------------------------
# [10] REALITY keys
# ---------------------------------------------------------------------
render_deployment_toml() {
  if [ -f "$DEPLOYMENT_TOML" ]; then
    log "$DEPLOYMENT_TOML already exists, leaving it untouched."
    return
  fi
  : "${PUBLIC_HOST:?Set PUBLIC_HOST=vpn.example.com before running install.sh}"
  : "${SUBSCRIPTION_HOST:="$PUBLIC_HOST"}"
  : "${SUBSCRIPTION_PORT:=8443}"
  : "${REALITY_HANDSHAKE_SERVER:?Set REALITY_HANDSHAKE_SERVER to a TLS 1.3 decoy hostname you control or have explicitly selected. There is no universal safe default; the installer performs a real sing-box protocol acceptance test.}"
  preflight_validate_hostname "$REALITY_HANDSHAKE_SERVER" "REALITY_HANDSHAKE_SERVER" || die "invalid REALITY_HANDSHAKE_SERVER."
  sed -e "s/{{PUBLIC_HOST}}/$PUBLIC_HOST/" \
      -e "s/{{SUBSCRIPTION_HOST}}/$SUBSCRIPTION_HOST/" \
      -e "s/{{SUBSCRIPTION_PORT}}/$SUBSCRIPTION_PORT/" \
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
  # Explicit, not inherited from whatever umask created them: these must be
  # group-readable or vpn-subscription cannot read the public key and short
  # id it serves. Relying on a 0644-by-umask default meant the correct mode
  # here was an accident of the writing process's umask.
  chmod 0640 "$STATE_DIR/reality/public.key" "$STATE_DIR/reality/short_id.txt"
  # Hysteria2 obfuscation password: same treatment as public.key/short_id.txt
  # above (vpn-subscription must be able to read it to serve subscriptions).
  # Present after a fresh `init` (obfuscation is on by default for new
  # installs); absent on an `init` re-run against an existing deployment
  # that predates this feature — that is a valid, expected state, not an
  # error, so this step is best-effort.
  if [ -f "$STATE_DIR/reality/hysteria_obfs_password.txt" ]; then
    chown root:vpn-subscription "$STATE_DIR/reality/hysteria_obfs_password.txt"
    chmod 0640 "$STATE_DIR/reality/hysteria_obfs_password.txt"
  fi
}

reality_keys_stage() {
  stage 10 "REALITY keys"
  render_deployment_toml
  init_reality_keys
}

# ---------------------------------------------------------------------
# [11] server config
# ---------------------------------------------------------------------
render_server_config() {
  log "rendering initial sing-box config..."
  # No `|| true` here (docs/PRODUCTION_HARDENING_PLAN.md #22/#27): a
  # config that fails to render/validate must abort installation, not
  # be silently skipped while the installer proceeds to start a
  # service with no valid config.
  "$BIN_DIR/vpn-admin" --config "$DEPLOYMENT_TOML" render-config
  chown -R root:sing-box "$STATE_DIR/sing-box"
  chmod 02750 "$STATE_DIR/sing-box"
}

server_config_stage() {
  stage 11 "server config"
  render_server_config
}

# ---------------------------------------------------------------------
# [12] nginx / subscription HTTPS
# ---------------------------------------------------------------------
configure_nginx() {
  if [ "${SUBSCRIPTION_TLS_READY:-0}" -ne 1 ]; then
    log "skipping nginx vhost: subscription TLS certificate not yet present (see stage 9 output)."
    return
  fi
  local host="${SUBSCRIPTION_HOST:-$PUBLIC_HOST}"
  local port="${SUBSCRIPTION_PORT:-8443}"
  install -d -m 0755 "$(dirname "$NGINX_CONF")"
  sed -e "s/{{SUBSCRIPTION_HOST}}/$host/" -e "s/{{PUBLIC_HOST}}/${PUBLIC_HOST:-$host}/" \
      -e "s/{{SUBSCRIPTION_PORT}}/$port/" \
      -e "s/{{SUBSCRIPTION_BACKEND_PORT}}/$SUBSCRIPTION_BACKEND_PORT/" \
    "$REPO_ROOT/deploy/almalinux/templates/nginx-vpn-subscription.conf.template" >"$NGINX_CONF.tmp"
  # SELinux (RHEL family) only lets nginx (httpd_t) bind() to a fixed
  # allow-list of ports labeled http_port_t (80/443/8080/8443/etc. are
  # on it by default) — a custom SUBSCRIPTION_PORT is very likely NOT on
  # that list, so the kernel silently denies the bind with a bare
  # "Permission denied" that looks identical to any other bind failure.
  # Caught on a real AlmaLinux install (`ausearch -m avc` showed
  # `denied { name_bind } ... tcontext=...unreserved_port_t`): nginx
  # then quietly kept running its OLD config instead of crashing, so
  # `systemctl status` reported "active" the whole time even though
  # nothing was listening on the new port at all. Label the port for
  # httpd_t before ever trying to bind it — modify the existing mapping
  # if some other type already claims this port, add a new one
  # otherwise.
  if [ "$OS_FAMILY" = "rhel" ] && command -v semanage >/dev/null 2>&1; then
    if semanage port -l 2>/dev/null | awk '/^http_port_t/' | grep -qw "$port"; then
      : # already labeled http_port_t — vpn1 did not add this mapping, nothing to undo
    elif semanage port -l 2>/dev/null | grep -w tcp | grep -qw "$port"; then
      die "SELinux port ${port}/tcp is already owned by a non-http type. Refusing to relabel another service's port; choose another SUBSCRIPTION_PORT."
    else
      semanage port -a -t http_port_t -p tcp "$port" \
        && ownership_list_add SELINUX_PORT_LABELS_ADDED "${port}/tcp" \
        || warn "could not add SELinux port context for ${port}/tcp — nginx may fail to bind it."
    fi
  fi
  # SELinux also blocks httpd_t from making ANY outbound network
  # connection by default (the httpd_can_network_connect boolean is off
  # out of the box on RHEL/AlmaLinux) — this vhost's whole purpose is
  # `proxy_pass http://127.0.0.1:9100`, so without this the reload
  # itself succeeds (bind on SUBSCRIPTION_PORT is fine once labeled
  # above) but every request 502s because nginx is denied when it tries
  # to connect to the backend. Caught on the same real AlmaLinux
  # install, immediately after fixing the port-bind denial above.
  if [ "$OS_FAMILY" = "rhel" ] && command -v setsebool >/dev/null 2>&1 && command -v getsebool >/dev/null 2>&1; then
    local prior
    prior="$(getsebool httpd_can_network_connect 2>/dev/null | awk '{print $NF}')"
    ownership_set_baseline_once SELINUX_HTTPD_NETCONNECT_PRE "${prior:-unknown}"
    setsebool -P httpd_can_network_connect 1 || warn "could not enable SELinux boolean httpd_can_network_connect — nginx's proxy_pass to the subscription backend may 502."
  fi
  # Swap and reload as one recoverable transaction. `nginx -t` cannot
  # validate this conf.d candidate in isolation, so preserve the live file,
  # install the candidate, and restore the exact predecessor on either
  # validation or reload failure.
  local nginx_backup="${NGINX_CONF}.install-bak"
  [ ! -e "$nginx_backup" ] || die "stale nginx transaction backup exists at $nginx_backup; inspect/recover it before retrying."
  local nginx_had_previous=0
  if [ -f "$NGINX_CONF" ]; then
    cp -a "$NGINX_CONF" "$nginx_backup"
    nginx_had_previous=1
  fi
  ownership_set_baseline_once NGINX_PRE_ENABLED "$(systemctl is-enabled --quiet nginx 2>/dev/null && echo 1 || echo 0)"
  install -m 0644 "$NGINX_CONF.tmp" "$NGINX_CONF"
  rm -f "$NGINX_CONF.tmp"
  # `systemctl enable --now` is a no-op on an already-active nginx — it
  # does NOT reload newly-written config into the running process. On a
  # repair/upgrade run nginx is very likely already active (e.g. started
  # earlier in this same run for the certbot dance), so relying on
  # `enable --now` alone silently leaves the OLD vhost config (old
  # SUBSCRIPTION_PORT, old hostname, etc.) live while the new file just
  # sits on disk unused — caught on a real AlmaLinux install where a
  # SUBSCRIPTION_PORT change never actually took effect. Always
  # explicitly reload-or-restart after enabling, regardless of prior
  # state.
  if nginx -t && systemctl enable nginx >/dev/null 2>&1 \
      && systemctl reload-or-restart nginx; then
    rm -f "$nginx_backup"
  else
    warn "new nginx vhost failed validation/reload; restoring previous file"
    if [ "$nginx_had_previous" -eq 1 ]; then
      mv -f "$nginx_backup" "$NGINX_CONF"
    else
      rm -f "$NGINX_CONF"
    fi
    if nginx -t; then
      systemctl reload-or-restart nginx >/dev/null 2>&1 || true
    fi
    die "nginx vhost update failed; previous configuration was restored."
  fi
  systemctl is-active --quiet nginx || die "nginx is not active after configuring the subscription vhost."
  # `is-active` alone is not proof the NEW config actually took effect:
  # on a failed reload (e.g. the SELinux bind denial above, before this
  # fix existed) nginx logs an [emerg] and keeps running its OLD config
  # instead of crashing, so `is-active` stays true while the port this
  # vhost is supposed to serve was never actually bound — exactly what
  # happened on a real install. Verify the listen socket directly.
  if command -v ss >/dev/null 2>&1; then
    local tries=0 bound=0
    while [ "$tries" -lt 10 ]; do
      ss -tln 2>/dev/null | grep -q ":${port} " && { bound=1; break; }
      tries=$((tries + 1))
      sleep 0.3
    done
    [ "$bound" -eq 1 ] || die "nginx is active but is NOT actually listening on ${port}/tcp after reload — the new config did not take effect (check: journalctl -u nginx --no-pager -n 50, /var/log/nginx/error.log, and 'ausearch -m avc -ts recent' if SELinux is enforcing)."
  fi
  NGINX_OK=1
  SUBSCRIPTION_HTTPS_OK=1
  log "nginx vhost installed and validated: $NGINX_CONF"
}

nginx_stage() {
  stage 12 "nginx/subscription HTTPS"
  configure_nginx
}

# ---------------------------------------------------------------------
# [13] firewall
# ---------------------------------------------------------------------
configure_firewall() {
  case "$FIREWALL_BACKEND" in
    firewalld) "$REPO_ROOT/deploy/almalinux/firewall.sh" ;;
    ufw) "$REPO_ROOT/deploy/almalinux/firewall-ufw.sh" ;;
    *) die "no firewall logic for backend '$FIREWALL_BACKEND'" ;;
  esac
}

firewall_stage() {
  stage 14 "firewall"
  configure_firewall
  FIREWALL_OK=1
}

# ---------------------------------------------------------------------
# [13] kernel network tuning
# ---------------------------------------------------------------------
# Runs before the firewall stage so a re-run's port-open behavior is
# unaffected by tuning ordering either way; placement here is otherwise
# arbitrary among the post-nginx stages. See deploy/lib/perf-tuning.sh
# for exactly what is (and is not) changed, and why — this stage only
# calls into that module, it contains no tuning logic itself.
perf_tuning_stage() {
  stage 13 "kernel network tuning"
  perf_tuning_apply
}

# ---------------------------------------------------------------------
# [15] SELinux (RHEL family only)
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
    # These three fcontext rules are always vpn1-owned (they name only
    # vpn1's own paths, which cannot have pre-existed before vpn1 did) —
    # unconditional, not baseline-gated.
    ownership_mark SELINUX_FCONTEXT_RULES_ADDED
  else
    log "semanage not found; skipping explicit SELinux file context (policycoreutils-python-utils should have installed it)."
  fi
}

selinux_stage() {
  stage 15 "SELinux"
  if [ "$OS_FAMILY" = "rhel" ]; then
    configure_selinux
  else
    log "skipping (not a RHEL-family host)."
  fi
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
  # `systemctl enable --now` is a no-op on an already-active unit — it
  # does NOT restart it to pick up a binary that binaries_stage (stage 4)
  # may have just rebuilt/reinstalled, or REALITY key material that
  # changed since the unit last started (same class of gotcha already
  # documented and fixed for nginx above, in configure_nginx). On a
  # fresh install both units are inactive so `enable --now` starts them
  # for the first time either way; on a repair/upgrade re-run of this
  # script both are very likely already active from before, so without
  # an explicit restart here vpn-subscription in particular would keep
  # serving whatever binary/REALITY state it loaded at its *previous*
  # start indefinitely — the exact "server and subscription service
  # silently disagree" incident class this installer must not
  # reintroduce on every repair run. sing-box itself is already
  # explicitly reloaded by server_config_stage (stage 11, via
  # `vpn-admin render-config`), so restarting it again here is a cheap,
  # idempotent no-op, not redundant risk — it's still restarted
  # explicitly rather than relying on that earlier reload alone, so this
  # function's behavior does not depend on stage ordering elsewhere.
  systemctl enable sing-box.service vpn-subscription.service vpn-expiry-reconcile.timer
  systemctl reload-or-restart sing-box.service \
    || die "sing-box failed to (re)start — check: journalctl -u sing-box --no-pager -n 100"
  systemctl reload-or-restart vpn-subscription.service \
    || die "vpn-subscription failed to (re)start — check: journalctl -u vpn-subscription --no-pager -n 100"
  systemctl start vpn-expiry-reconcile.timer \
    || die "credential-expiry reconciliation timer failed to start"
}

# Each protocol's status is confirmed independently by actually observing
# its own listener/socket — never inferred from the other protocol or
# from an unrelated stage's variable (docs/FINAL_PRODUCTION_AUDIT.md
# P0-14: sharing one sing-box process does not guarantee both inbounds
# bound successfully, e.g. one address family failing while the other
# succeeds).
confirm_data_plane_listening() {
  systemctl is-active --quiet sing-box || die "sing-box is not active after start — cannot verify data plane."
  local tries=0
  while [ "$tries" -lt 10 ]; do
    if command -v ss >/dev/null 2>&1; then
      ss -tln 2>/dev/null | grep -q ':443 ' && VLESS_REALITY_OK=1
      ss -uln 2>/dev/null | grep -q ':443 ' && HYSTERIA2_OK=1
    fi
    [ "$VLESS_REALITY_OK" -eq 1 ] && [ "$HYSTERIA2_OK" -eq 1 ] && break
    tries=$((tries + 1))
    sleep 0.5
  done
  [ "$VLESS_REALITY_OK" -eq 1 ] || die "VLESS+REALITY is not listening on 443/tcp after start — sing-box.service is active but the inbound never bound. Check: journalctl -u sing-box --no-pager -n 100"
  [ "$HYSTERIA2_OK" -eq 1 ] || die "Hysteria2 is not listening on 443/udp after start — sing-box.service is active but the inbound never bound. Check: journalctl -u sing-box --no-pager -n 100"
}

confirm_subscription_backend() {
  systemctl is-active --quiet vpn-subscription || die "vpn-subscription is not active after start."
  local tries=0
  while [ "$tries" -lt 10 ]; do
    curl -fsS --connect-timeout 5 --max-time 10 -o /dev/null http://127.0.0.1:${SUBSCRIPTION_BACKEND_PORT}/healthz 2>/dev/null && { SUBSCRIPTION_BACKEND_OK=1; break; }
    tries=$((tries + 1))
    sleep 0.5
  done
  [ "$SUBSCRIPTION_BACKEND_OK" -eq 1 ] || die "subscription backend did not answer /healthz after start. Check: journalctl -u vpn-subscription --no-pager -n 100"
}

start_stage() {
  stage 16 "validation + start"
  validate_before_start
  enable_and_start_services
  confirm_data_plane_listening
  confirm_subscription_backend
  install -m 0755 "$REPO_ROOT/deploy/almalinux/health-check.sh" "$BIN_DIR/vpn-health-check"
  install_vpn_benchmark
  # Write the manifest NOW, not only after acceptance_stage succeeds
  # (see write_install_state_manifest's doc comment) — the data plane
  # and subscription backend are confirmed listening at this point, so
  # a re-run after a later acceptance-only failure correctly detects
  # this as an existing install to repair, not a fresh one.
  write_install_state_manifest "pending"
}

# `vpn-benchmark` sources `vpn-benchmark-lib.sh` from its own directory
# (`$(dirname "${BASH_SOURCE[0]}")` — see deploy/lib/vpn-benchmark.sh) so
# it works correctly from a plain repo checkout without any install step
# at all; installed as a flat copy (not a symlink back into $REPO_ROOT,
# which could go stale or dangle if the source tree ever moves/is
# removed) into $BIN_DIR, the lib file must be installed alongside it in
# that same directory, or the installed copy fails immediately with
# "No such file or directory" the first time anyone runs `vpn-benchmark`.
install_vpn_benchmark() {
  install -m 0755 "$REPO_ROOT/deploy/lib/vpn-benchmark.sh" "$BIN_DIR/vpn-benchmark"
  install -m 0644 "$REPO_ROOT/deploy/lib/vpn-benchmark-lib.sh" "$BIN_DIR/vpn-benchmark-lib.sh"
}

# ---------------------------------------------------------------------
# [16] first user + acceptance test
# ---------------------------------------------------------------------
DEFAULT_USER_NAME="default"
SUBSCRIPTION_URL=""
FIRST_USER_QR_OUTPUT=""

# The Rust admin CLI has its own terminal QR renderer (the `qrcode` crate
# — apps/admin/src/main.rs `print_qr`), so the one-command install path
# never needs to depend on a distro `qrencode` package being installed
# (docs/FINAL_PRODUCTION_AUDIT.md P0-15). Call `user create --qr` in its
# normal (non-JSON) human-output mode once — it already prints exactly
# the URL + QR + Hiddify onboarding steps a first-time user needs — and
# capture that whole block for print_status to show, instead of a second
# `--json` call (which would either skip the QR or, via `user qr`, mint
# and invalidate a second token for no reason).
ensure_first_user() {
  local existing
  existing="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" user list 2>/dev/null | tail -n +2 | grep -c . || true)"
  if [ "${existing:-0}" -gt 0 ]; then
    log "$existing existing user(s) found — not creating a default user (idempotent re-run)."
    return
  fi
  log "creating initial VPN user '$DEFAULT_USER_NAME'..."
  local out
  out="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" user create --name "$DEFAULT_USER_NAME" --qr 2>/dev/null)" \
    || { warn "failed to auto-create the default user; run 'vpn user create --name default --qr' manually."; return; }
  FIRST_USER_QR_OUTPUT="$out"
  SUBSCRIPTION_URL="$(echo "$out" | sed -n '/^Subscription:$/{n;p;}')"
}

acceptance_stage() {
  stage 17 "first user + acceptance test"
  ensure_first_user
  if ! "$BIN_DIR/vpn-health-check"; then
    die "post-install health check failed — see output above. Installation did not complete cleanly."
  fi
  # Capture doctor's own output rather than only its exit code: doctor
  # reports each check independently and prefixes failing lines with
  # "[FAIL]" (see apps/admin/src/main.rs report_check/CheckStatus), so
  # relaying that here tells the operator exactly which check(s) failed
  # — e.g. an unrelated IPv6 diagnostic vs. the REALITY handshake itself
  # — instead of a single generic message that blames "acceptance"
  # regardless of cause. A previous version of this message always said
  # "real VLESS+REALITY end-to-end acceptance failed" even when the
  # REALITY handshake itself (L5/L6) had actually passed and the real
  # failing check was something else entirely.
  local doctor_output doctor_rc=0
  doctor_output="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" doctor --protocol --require-protocol 2>&1)" || doctor_rc=$?
  if [ "$doctor_rc" -ne 0 ]; then
    echo "$doctor_output" >&2
    local fail_lines
    fail_lines="$(echo "$doctor_output" | grep -F '[FAIL]' || true)"
    if [ -n "$fail_lines" ]; then
      die "post-install acceptance check(s) failed — see the [FAIL] line(s) above for the specific cause (this is not necessarily the VLESS+REALITY handshake itself; re-run 'vpn doctor --protocol' any time to see current status):
$fail_lines"
    else
      die "'vpn doctor --protocol --require-protocol' exited non-zero (status $doctor_rc) but printed no [FAIL] line — see full output above. Re-run 'vpn doctor --protocol' for details; installation is not accepted."
    fi
  fi
  log "health check passed."
}

# ---------------------------------------------------------------------
# [17] summary
# ---------------------------------------------------------------------
# Source of truth for update/uninstall/reinstall-detection to key off
# (docs/FINAL_PRODUCTION_AUDIT.md P1 "install-state manifest") — never
# raw secrets, only metadata about what this install IS and what it
# owns. Best-effort `sing-box version` line included for support/debug
# purposes only.
# The manifest is the ONLY thing `existing_install_present()` checks to
# decide fresh-install vs. repair (docs/FINAL_PRODUCTION_AUDIT.md P1
# "install-state manifest"). It used to be written ONLY inside
# print_status(), which main() only reaches after acceptance_stage()
# succeeds — so an install that got as far as "services running and
# confirmed listening" (start_stage) but then failed the separate
# acceptance test (acceptance_stage — e.g. a flaky REALITY_HANDSHAKE_SERVER
# or an unrelated diagnostic) left NO manifest on disk at all. A re-run
# after that then looked exactly like a fresh install: it re-ran the
# stage-1 port-conflict checks against 443/tcp+udp and SUBSCRIPTION_PORT,
# which vpn1's own already-running services legitimately hold — turning
# a "just re-run it" repair into a hard failure at stage 1.
#
# Fix: write the manifest right after start_stage confirms the data
# plane + subscription backend are actually listening (an "acceptance:
# pending" state — this IS a real install, just not yet accepted), and
# update the same field to "accepted" once acceptance_stage also
# succeeds. existing_install_present() treats BOTH states as "an
# existing install exists, do a repair" — the distinction only matters
# for reporting. print_status's own hard asserts below are untouched by
# this change and still refuse to print a success banner unless
# VLESS_REALITY_OK/HYSTERIA2_OK/SUBSCRIPTION_BACKEND_OK are actually
# confirmed true in-memory in *this* run.
#
# $1 = acceptance status to record: "pending" or "accepted".
write_install_state_manifest() {
  local acceptance="${1:?write_install_state_manifest requires an acceptance status argument}"
  local manifest_dir="/var/lib/vpn1" manifest="/var/lib/vpn1/install-state.json"
  install -d -m 0755 "$manifest_dir"
  local singbox_version
  singbox_version="$("$SINGBOX_BIN" version 2>/dev/null | head -n1 | sed 's/"/\\"/g' || echo unknown)"
  local vpn1_version="${VPN1_VERSION:-main}"
  local pinned_singbox_sha256=""
  case "$ARCH" in
    amd64) pinned_singbox_sha256="$SINGBOX_SHA256_AMD64" ;;
    arm64) pinned_singbox_sha256="$SINGBOX_SHA256_ARM64" ;;
  esac
  cat > "$manifest.tmp" <<EOF
{
  "vpn1_version": "$vpn1_version",
  "vpn1_repo": "$VPN1_RELEASE_REPO",
  "sing_box_version": "$singbox_version",
  "sing_box_version_pinned": "$SINGBOX_VERSION",
  "sing_box_sha256_pinned": "$pinned_singbox_sha256",
  "arch": "$ARCH",
  "installed_at_unix": $(date +%s),
  "public_host": "$PUBLIC_HOST",
  "subscription_host": "${SUBSCRIPTION_HOST:-$PUBLIC_HOST}",
  "firewall_backend": "$FIREWALL_BACKEND",
  "os_family": "$OS_FAMILY",
  "repo_root": "$REPO_ROOT",
  "acceptance": "$acceptance"
}
EOF
  chmod 0644 "$manifest.tmp"
  mv -f "$manifest.tmp" "$manifest"
  log "wrote install-state manifest ($manifest, acceptance=$acceptance)"
}

print_status() {
  stage 18 "summary"
  write_install_state_manifest "accepted"
  # Never print a success banner claiming a component works when it
  # wasn't actually confirmed (docs/FINAL_PRODUCTION_AUDIT.md P0-14) —
  # these are the same booleans set at each stage's real confirmation
  # point above, re-checked here instead of trusting `set -e` alone to
  # have caught every path.
  [ "$VLESS_REALITY_OK" -eq 1 ] || die "internal: reached summary with VLESS+REALITY not confirmed — this should be unreachable (start_stage should have aborted first)."
  [ "$HYSTERIA2_OK" -eq 1 ] || die "internal: reached summary with Hysteria2 not confirmed — this should be unreachable (start_stage should have aborted first)."
  [ "$SUBSCRIPTION_BACKEND_OK" -eq 1 ] || die "internal: reached summary with subscription backend not confirmed — this should be unreachable (start_stage should have aborted first)."

  cat <<BANNER

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 vpn1 installation complete
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Server
  Address: ${PUBLIC_HOST}
  Status:  running
Components (each independently confirmed, not inferred)
  VLESS + REALITY (443/tcp)     $([ "$VLESS_REALITY_OK" -eq 1 ] && echo "✓ listening" || echo "✗")
  Hysteria2 (443/udp)           $([ "$HYSTERIA2_OK" -eq 1 ] && echo "✓ listening" || echo "✗")
  Subscription backend          $([ "$SUBSCRIPTION_BACKEND_OK" -eq 1 ] && echo "✓ healthy" || echo "✗")
  Subscription HTTPS (nginx)    $([ "$NGINX_OK" -eq 1 ] && [ "$SUBSCRIPTION_HTTPS_OK" -eq 1 ] && echo "✓ configured" || echo "not configured — see stage 8/11 output above")
  Firewall                      $([ "$FIREWALL_OK" -eq 1 ] && echo "✓ configured" || echo "✗")
User
  ${DEFAULT_USER_NAME}
BANNER
  if [ -n "$FIRST_USER_QR_OUTPUT" ]; then
    echo "$FIRST_USER_QR_OUTPUT"
  elif [ -n "$SUBSCRIPTION_URL" ]; then
    echo "Subscription URL"
    echo "  $SUBSCRIPTION_URL"
  else
    echo "Subscription URL"
    echo "  (not created automatically — run: vpn user create --name default --qr)"
  fi
  cat <<BANNER

Compatible clients
  Android / MagicOS: Hiddify
  iOS:                Hiddify
  Linux:              Hiddify / compatible sing-box client

Management
  vpn status
  vpn doctor                 # process/config/listeners/subscription-coherence (fast)
  vpn doctor --protocol      # + a real throwaway sing-box handshake self-test (slower)
  vpn doctor --performance   # host/kernel/network MEASUREMENTS (CPU, steal, buffers, ...)
  vpn-benchmark               # real throughput/latency benchmark (see docs/PERFORMANCE_OPTIMIZATION_PLAN.md)
  vpn user create --name NAME
  vpn user list
  vpn user rotate-token USER
  vpn user remove USER
  $REPO_ROOT/deploy/almalinux/update.sh
  $REPO_ROOT/deploy/almalinux/uninstall.sh

Documentation:
  https://github.com/$VPN1_RELEASE_REPO

BANNER
}


# Failure-injection testability hook (deploy/almalinux/lifecycle-acceptance.sh
# stage 10, "failed/interrupted install cleanup"): when
# VPN1_LIFECYCLE_GATE_ABORT_AFTER=<stage-name> is set, abort right after
# that stage completes, so the destructive lifecycle gate can
# deterministically exercise "install died partway through, does
# uninstall still clean up completely" without relying on a real,
# unreproducible mid-install crash. Unset in every real install path
# (bootstrap install.sh never sets it) — a no-op there.
lifecycle_gate_abort_hook() {
  [ "${VPN1_LIFECYCLE_GATE_ABORT_AFTER:-}" = "$1" ] || return 0
  die "VPN1_LIFECYCLE_GATE_ABORT_AFTER=$1 — deliberately aborting for lifecycle-gate testing."
}

main() {
  parse_cli_args "$@"
  preflight_stage
  packages_stage
  host_config_stage
  binaries_stage
  singbox_install_stage
  lifecycle_gate_abort_hook install_singbox
  systemd_stage
  users_groups_stage
  directories_stage
  certificates_stage
  reality_keys_stage
  server_config_stage
  nginx_stage
  perf_tuning_stage
  firewall_stage
  selinux_stage
  start_stage
  acceptance_stage
  print_status
}

# Guard against auto-execution when this file is `source`d (e.g. by
# deploy/lib/tests/test-install-manifest-idempotency.sh, to exercise the
# real existing_install_present()/write_install_state_manifest()
# functions without running the whole production installer). Every real
# invocation path runs this file as a script, never sources it:
#   - the top-level bootstrap (install.sh) always does `bash
#     "$SRC_DIR/deploy/almalinux/install.sh"` with a real file path, so
#     BASH_SOURCE[0] == $0 there;
#   - a manual `./deploy/almalinux/install.sh` or `bash
#     deploy/almalinux/install.sh` invocation has the same property.
# This script is never invoked via `curl | bash` directly (only the
# top-level bootstrap is; see install.sh's own comment on why $0/
# BASH_SOURCE are unreliable under stdin piping) — this file is always
# handed a real path, so this guard does not need to handle that case.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
