#!/usr/bin/env bash
# AmneziaWG node lifecycle for singbox-vpn: pinned source build, systemd
# hooks (addresses, forwarding, NAT, firewall) and reversible uninstall.
#
#   amneziawg.sh install        build + install pinned upstream binaries and the unit
#   amneziawg.sh pre-start      (unit) refuse to start on missing/unsafe config
#   amneziawg.sh post-start     (unit) setconf, addresses, MTU, forwarding, NAT
#   amneziawg.sh post-stop      (unit) remove NAT/forward rules owned by us
#   amneziawg.sh firewall-open  open the listen port (firewalld/ufw), recording ownership
#   amneziawg.sh firewall-close close it again only if we opened it
#   amneziawg.sh uninstall      stop, remove unit/binaries/rules we own, restore sysctls
#   amneziawg.sh verify         check installed binaries against recorded hashes
#
# Supply chain (docs/platform-v2/MIGRATION_PLAN.md §5): upstream publishes
# no Linux binaries for the pinned tags, so they are built here from
# source. The Go toolchain tarball is SHA-256 verified, each clone must
# resolve to exactly the pinned commit, Go modules are checked with
# `go mod verify` in an isolated module cache, and the resulting binary
# hashes are recorded for drift detection. Any mismatch aborts before
# anything is installed.
#
# Every path and external command is overridable through the environment
# purely so deploy/lib/tests/test-amneziawg-lib.sh can run this against a
# throwaway directory with mocked tools; production never sets them.
set -Eeuo pipefail

AWG_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${AWG_VERSIONS_ENV:=$AWG_SCRIPT_DIR/versions.env}"
: "${AWG_STATE_DIR:=/etc/vpn/compat/amneziawg}"
: "${AWG_RUNTIME_ENV:=$AWG_STATE_DIR/runtime.env}"
: "${AWG_BIN_DIR:=/usr/local/bin}"
: "${AWG_UNIT_SRC:=$AWG_SCRIPT_DIR/../almalinux/systemd/vpn-amneziawg.service}"
: "${AWG_UNIT_DST:=/etc/systemd/system/vpn-amneziawg.service}"
: "${AWG_LIB_STATE:=/var/lib/singbox-vpn}"
: "${AWG_SYSCTL_DROPIN:=/etc/sysctl.d/99-singbox-vpn-amneziawg.conf}"
: "${AWG_UAPI_DIR:=/var/run/amneziawg}"
: "${AWG_BUILD_ROOT:=}"
: "${AWG_SOCKET_WAIT_SECONDS:=10}"
: "${SYSTEMCTL:=systemctl}"

AWG_RULE_COMMENT="singbox-vpn-amneziawg"
AWG_NFT_TABLE="singbox_vpn_awg"

log() { echo "[amneziawg] $*"; }
warn() { echo "[amneziawg] WARNING: $*" >&2; }
die() { echo "[amneziawg] ERROR: $*" >&2; exit 1; }

load_versions() {
  [[ -r "$AWG_VERSIONS_ENV" ]] || die "pinned versions file $AWG_VERSIONS_ENV not readable"
  # shellcheck disable=SC1090
  source "$AWG_VERSIONS_ENV"
  local key
  for key in AMNEZIAWG_GO_VERSION AMNEZIAWG_GO_COMMIT AMNEZIAWG_TOOLS_VERSION AMNEZIAWG_TOOLS_COMMIT \
             GO_TOOLCHAIN_VERSION GO_TOOLCHAIN_SHA256_AMD64 GO_TOOLCHAIN_SHA256_ARM64; do
    [[ -n "${!key:-}" ]] || die "$key missing from $AWG_VERSIONS_ENV; refusing to build unpinned"
  done
  [[ "$AMNEZIAWG_GO_COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "AMNEZIAWG_GO_COMMIT is not a full commit id"
  [[ "$AMNEZIAWG_TOOLS_COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "AMNEZIAWG_TOOLS_COMMIT is not a full commit id"
}

load_runtime_env() {
  [[ -r "$AWG_RUNTIME_ENV" ]] || die "$AWG_RUNTIME_ENV missing; run \`vpn-admin transport enable amneziawg\`"
  # shellcheck disable=SC1090
  source "$AWG_RUNTIME_ENV"
  [[ "${AWG_INTERFACE:-}" =~ ^[A-Za-z0-9_.-]{1,15}$ ]] || die "invalid AWG_INTERFACE in $AWG_RUNTIME_ENV"
  [[ "${AWG_LISTEN_PORT:-}" =~ ^[0-9]{1,5}$ ]] || die "invalid AWG_LISTEN_PORT in $AWG_RUNTIME_ENV"
  [[ "${AWG_MTU:-}" =~ ^[0-9]{4}$ ]] || die "invalid AWG_MTU in $AWG_RUNTIME_ENV"
  [[ -n "${AWG_ADDRESS_V4:-}" && -n "${AWG_SUBNET_V4:-}" ]] || die "AWG_ADDRESS_V4/AWG_SUBNET_V4 missing"
  [[ -n "${AWG_SETCONF:-}" ]] || die "AWG_SETCONF missing"
}

go_arch() {
  case "$(uname -m)" in
    x86_64) echo amd64 ;;
    aarch64) echo arm64 ;;
    *) die "unsupported architecture $(uname -m)" ;;
  esac
}

verify_sha256() { # file expected
  local actual
  actual="$(sha256sum "$1" | awk '{print $1}')"
  [[ "$actual" == "$2" ]] || die "checksum mismatch for $(basename "$1"): expected $2, got $actual"
}

clone_pinned() { # url tag commit dest
  git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$2" "$1" "$4" \
    || die "cloning $1 at $2 failed"
  local head
  head="$(git -C "$4" rev-parse HEAD)"
  [[ "$head" == "$3" ]] || die "$1 tag $2 resolves to $head, expected pinned commit $3 (tag moved?); refusing to build"
}

build_all() { # workdir -> prints nothing, leaves binaries in workdir/out
  local work="$1" arch sha
  arch="$(go_arch)"
  sha="GO_TOOLCHAIN_SHA256_${arch^^}"
  mkdir -p "$work/out"
  log "fetching Go ${GO_TOOLCHAIN_VERSION} (${arch}) and verifying SHA-256"
  curl -fsSL --retry 3 -o "$work/go.tgz" "https://go.dev/dl/go${GO_TOOLCHAIN_VERSION}.linux-${arch}.tar.gz" \
    || die "downloading the Go toolchain failed"
  verify_sha256 "$work/go.tgz" "${!sha}"
  tar -C "$work" -xzf "$work/go.tgz"

  clone_pinned https://github.com/amnezia-vpn/amneziawg-go "$AMNEZIAWG_GO_VERSION" "$AMNEZIAWG_GO_COMMIT" "$work/amneziawg-go"
  clone_pinned https://github.com/amnezia-vpn/amneziawg-tools "$AMNEZIAWG_TOOLS_VERSION" "$AMNEZIAWG_TOOLS_COMMIT" "$work/amneziawg-tools"

  log "building amneziawg-go ${AMNEZIAWG_GO_VERSION} (go mod verify, isolated module cache)"
  (
    cd "$work/amneziawg-go"
    export GOROOT="$work/go" GOPATH="$work/gopath" GOMODCACHE="$work/gomodcache" \
           GOTOOLCHAIN=local GOFLAGS=-mod=readonly CGO_ENABLED=0 GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org
    "$work/go/bin/go" mod download
    "$work/go/bin/go" mod verify
    "$work/go/bin/go" build -trimpath -o "$work/out/amneziawg-go" .
  ) || die "building amneziawg-go failed"

  log "building awg from amneziawg-tools ${AMNEZIAWG_TOOLS_VERSION}"
  (
    cd "$work/amneziawg-tools/src"
    if command -v make >/dev/null 2>&1; then
      make --quiet >/dev/null
      cp wg "$work/out/awg"
    else
      gcc -O2 -I uapi/linux -std=gnu99 -D_GNU_SOURCE -DRUNSTATEDIR="\"/var/run\"" -o "$work/out/awg" ./*.c
    fi
  ) || die "building awg failed"
  "$work/out/awg" --version | grep -q "${AMNEZIAWG_TOOLS_VERSION}" \
    || die "built awg does not report ${AMNEZIAWG_TOOLS_VERSION}"
}

cmd_install() {
  [[ "$(id -u)" == 0 ]] || die "install must run as root"
  load_versions
  local rec="$AWG_LIB_STATE/amneziawg-binaries.sha256"
  if [[ -r "$rec" ]] && grep -q "^# ${AMNEZIAWG_GO_COMMIT} ${AMNEZIAWG_TOOLS_COMMIT}$" "$rec" \
     && (cd "$AWG_BIN_DIR" && sha256sum --quiet -c <(grep -v '^#' "$rec")) 2>/dev/null; then
    log "pinned binaries already installed and unchanged; nothing to build"
  else
    local work
    work="$(mktemp -d "${AWG_BUILD_ROOT:-${TMPDIR:-/tmp}}/awg-build.XXXXXX")"
    trap 'rm -rf "$work"' RETURN
    build_all "$work"
    install -m 0755 "$work/out/amneziawg-go" "$AWG_BIN_DIR/amneziawg-go"
    install -m 0755 "$work/out/awg" "$AWG_BIN_DIR/awg"
    mkdir -p "$AWG_LIB_STATE"
    {
      echo "# ${AMNEZIAWG_GO_COMMIT} ${AMNEZIAWG_TOOLS_COMMIT}"
      (cd "$AWG_BIN_DIR" && sha256sum amneziawg-go awg)
    } > "$rec.tmp"
    mv "$rec.tmp" "$rec"
    log "installed amneziawg-go and awg; hashes recorded in $rec"
  fi
  install -m 0644 "$AWG_UNIT_SRC" "$AWG_UNIT_DST"
  "$SYSTEMCTL" daemon-reload
  log "unit installed: enable with \`systemctl enable --now vpn-amneziawg\` after \`vpn-admin transport enable amneziawg\`"
}

cmd_verify() {
  local rec="$AWG_LIB_STATE/amneziawg-binaries.sha256"
  [[ -r "$rec" ]] || die "no recorded binary hashes at $rec"
  (cd "$AWG_BIN_DIR" && sha256sum --quiet -c <(grep -v '^#' "$rec")) || die "installed AmneziaWG binaries differ from the recorded build"
  log "installed binaries match $rec"
}

cmd_pre_start() {
  load_runtime_env
  [[ -f "$AWG_SETCONF" ]] || die "$AWG_SETCONF missing; run \`vpn-admin render-config\`"
  local mode
  mode="$(stat -c '%a' "$AWG_SETCONF")"
  [[ "$mode" == 600 ]] || die "$AWG_SETCONF has mode $mode; it holds the server private key and must be 0600"
  [[ -x "$AWG_BIN_DIR/amneziawg-go" && -x "$AWG_BIN_DIR/awg" ]] || die "amneziawg binaries missing; run \`amneziawg.sh install\`"
}

wait_for_socket() {
  local sock="$AWG_UAPI_DIR/$AWG_INTERFACE.sock" waited=0
  while [[ ! -S "$sock" ]]; do
    (( waited >= AWG_SOCKET_WAIT_SECONDS * 10 )) && die "UAPI socket $sock did not appear within ${AWG_SOCKET_WAIT_SECONDS}s"
    sleep 0.1
    waited=$((waited + 1))
  done
}

sysctl_capture_baseline_once() {
  local base="$AWG_LIB_STATE/amneziawg-sysctl-baseline.env"
  [[ -e "$base" ]] && return 0
  mkdir -p "$AWG_LIB_STATE"
  {
    echo "net.ipv4.ip_forward=$(sysctl -n net.ipv4.ip_forward 2>/dev/null || echo 0)"
    echo "net.ipv6.conf.all.forwarding=$(sysctl -n net.ipv6.conf.all.forwarding 2>/dev/null || echo 0)"
  } > "$base"
}

apply_forwarding() {
  sysctl_capture_baseline_once
  {
    echo "# Managed by singbox-vpn (AmneziaWG). Removed by amneziawg.sh uninstall."
    echo "net.ipv4.ip_forward=1"
    [[ -n "${AWG_SUBNET_V6:-}" ]] && echo "net.ipv6.conf.all.forwarding=1"
  } > "$AWG_SYSCTL_DROPIN"
  sysctl -q -p "$AWG_SYSCTL_DROPIN" >/dev/null
}

nat_add() {
  if command -v nft >/dev/null 2>&1; then
    nft delete table inet "$AWG_NFT_TABLE" 2>/dev/null || true
    local v6_rules=""
    if [[ -n "${AWG_SUBNET_V6:-}" ]]; then
      v6_rules="ip6 saddr ${AWG_SUBNET_V6} oifname != \"${AWG_INTERFACE}\" masquerade"
    fi
    nft -f - <<NFT
table inet ${AWG_NFT_TABLE} {
  chain forward {
    type filter hook forward priority filter; policy accept;
    iifname "${AWG_INTERFACE}" accept
    oifname "${AWG_INTERFACE}" ct state established,related accept
  }
  chain postrouting {
    type nat hook postrouting priority srcnat; policy accept;
    ip saddr ${AWG_SUBNET_V4} oifname != "${AWG_INTERFACE}" masquerade
    ${v6_rules}
  }
}
NFT
    return 0
  fi
  command -v iptables >/dev/null 2>&1 || die "neither nft nor iptables is available for NAT"
  local rule
  for rule in \
    "-t nat POSTROUTING -s ${AWG_SUBNET_V4} ! -o ${AWG_INTERFACE} -j MASQUERADE" \
    "-t filter FORWARD -i ${AWG_INTERFACE} -j ACCEPT" \
    "-t filter FORWARD -o ${AWG_INTERFACE} -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT"; do
    # shellcheck disable=SC2086
    set -- $rule
    local table="$2" chain="$3"; shift 3
    iptables -t "$table" -C "$chain" "$@" -m comment --comment "$AWG_RULE_COMMENT" 2>/dev/null \
      || iptables -t "$table" -A "$chain" "$@" -m comment --comment "$AWG_RULE_COMMENT"
  done
  if [[ -n "${AWG_SUBNET_V6:-}" ]] && command -v ip6tables >/dev/null 2>&1; then
    ip6tables -t nat -C POSTROUTING -s "$AWG_SUBNET_V6" ! -o "$AWG_INTERFACE" -j MASQUERADE -m comment --comment "$AWG_RULE_COMMENT" 2>/dev/null \
      || ip6tables -t nat -A POSTROUTING -s "$AWG_SUBNET_V6" ! -o "$AWG_INTERFACE" -j MASQUERADE -m comment --comment "$AWG_RULE_COMMENT"
  fi
}

nat_remove() {
  if command -v nft >/dev/null 2>&1; then
    nft delete table inet "$AWG_NFT_TABLE" 2>/dev/null || true
  fi
  local bin table
  for bin in iptables ip6tables; do
    command -v "$bin" >/dev/null 2>&1 || continue
    for table in nat filter; do
      # Delete only rules carrying our comment, one at a time, until none remain.
      local spec
      while spec="$("$bin" -t "$table" -S 2>/dev/null | grep -- "--comment \"\\?$AWG_RULE_COMMENT\"\\?" | head -1)" && [[ -n "$spec" ]]; do
        # shellcheck disable=SC2086
        "$bin" -t "$table" ${spec/-A /-D } || break
      done
    done
  done
}

cmd_post_start() {
  load_runtime_env
  wait_for_socket
  "$AWG_BIN_DIR/awg" setconf "$AWG_INTERFACE" "$AWG_SETCONF" || die "awg setconf rejected $AWG_SETCONF"
  ip address replace "$AWG_ADDRESS_V4" dev "$AWG_INTERFACE"
  if [[ -n "${AWG_ADDRESS_V6:-}" ]]; then
    ip -6 address replace "$AWG_ADDRESS_V6" dev "$AWG_INTERFACE"
  fi
  ip link set dev "$AWG_INTERFACE" mtu "$AWG_MTU" up
  apply_forwarding
  nat_add
  log "interface $AWG_INTERFACE up on UDP $AWG_LISTEN_PORT"
}

cmd_post_stop() {
  load_runtime_env
  nat_remove
  log "NAT/forward rules for $AWG_INTERFACE removed"
}

firewall_owner_file() { echo "$AWG_LIB_STATE/amneziawg-firewall-owned"; }

cmd_firewall_open() {
  load_runtime_env
  local owned
  owned="$(firewall_owner_file)"
  mkdir -p "$AWG_LIB_STATE"
  if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    if ! firewall-cmd --permanent --query-port="${AWG_LISTEN_PORT}/udp" >/dev/null 2>&1; then
      firewall-cmd --permanent --add-port="${AWG_LISTEN_PORT}/udp" >/dev/null
      firewall-cmd --add-port="${AWG_LISTEN_PORT}/udp" >/dev/null
      echo "firewalld ${AWG_LISTEN_PORT}/udp" > "$owned"
    fi
  elif command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q "Status: active"; then
    if ! ufw status 2>/dev/null | grep -q "^${AWG_LISTEN_PORT}/udp"; then
      ufw allow "${AWG_LISTEN_PORT}/udp" comment "$AWG_RULE_COMMENT" >/dev/null
      echo "ufw ${AWG_LISTEN_PORT}/udp" > "$owned"
    fi
  else
    warn "no active host firewall detected; make sure UDP ${AWG_LISTEN_PORT} is open in the provider firewall"
  fi
  log "UDP ${AWG_LISTEN_PORT} open (provider/cloud firewalls are not managed here)"
}

cmd_firewall_close() {
  local owned backend port
  owned="$(firewall_owner_file)"
  [[ -r "$owned" ]] || { log "no firewall rule owned by singbox-vpn; nothing to close"; return 0; }
  read -r backend port < "$owned"
  case "$backend" in
    firewalld)
      firewall-cmd --permanent --remove-port="$port" >/dev/null 2>&1 || true
      firewall-cmd --remove-port="$port" >/dev/null 2>&1 || true ;;
    ufw) ufw delete allow "$port" >/dev/null 2>&1 || true ;;
    *) warn "unknown firewall owner record: $backend" ;;
  esac
  rm -f "$owned"
  log "closed $port ($backend)"
}

cmd_uninstall() {
  [[ "$(id -u)" == 0 ]] || die "uninstall must run as root"
  "$SYSTEMCTL" disable --now vpn-amneziawg >/dev/null 2>&1 || true
  if [[ -r "$AWG_RUNTIME_ENV" ]]; then
    cmd_post_stop || true
    cmd_firewall_close || true
  fi
  rm -f "$AWG_UNIT_DST"
  "$SYSTEMCTL" daemon-reload || true
  local rec="$AWG_LIB_STATE/amneziawg-binaries.sha256"
  if [[ -r "$rec" ]]; then
    rm -f "$AWG_BIN_DIR/amneziawg-go" "$AWG_BIN_DIR/awg" "$rec"
  else
    warn "no build record; leaving $AWG_BIN_DIR/{amneziawg-go,awg} untouched (not installed by singbox-vpn)"
  fi
  if [[ -e "$AWG_SYSCTL_DROPIN" ]]; then
    rm -f "$AWG_SYSCTL_DROPIN"
    local base="$AWG_LIB_STATE/amneziawg-sysctl-baseline.env"
    if [[ -r "$base" ]]; then
      sysctl -q -p "$base" >/dev/null || warn "restoring forwarding sysctls from $base failed"
      rm -f "$base"
    fi
  fi
  log "AmneziaWG data plane removed; key material in $AWG_STATE_DIR is kept (vpn-admin backup covers it)"
}

main() {
  case "${1:-}" in
    install) cmd_install ;;
    verify) cmd_verify ;;
    pre-start) cmd_pre_start ;;
    post-start) cmd_post_start ;;
    post-stop) cmd_post_stop ;;
    firewall-open) cmd_firewall_open ;;
    firewall-close) cmd_firewall_close ;;
    uninstall) cmd_uninstall ;;
    *) sed -n '2,12p' "$0"; exit 2 ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
