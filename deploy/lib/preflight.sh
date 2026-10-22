#!/usr/bin/env bash
# Shared preflight checks. Sourced by deploy/almalinux/install.sh and the
# root bootstrap installer. Expects log()/warn()/die() to already be
# defined by the caller.

# The AlmaLinux installer defines CURL_NET_FLAGS before sourcing this file.
# Plain curl --retry does not retry a refused TCP connection by default; the
# real lifecycle gate observed github.com intermittently returning ECONNREFUSED
# and a later retry succeeding. Add the explicit retry class once for every
# caller that already owns a shared curl-flags array. Keep this idempotent
# because some fixture tests source helpers more than once in one shell.
if declare -p CURL_NET_FLAGS >/dev/null 2>&1; then
  case " ${CURL_NET_FLAGS[*]} " in
    *" --retry-connrefused "*) ;;
    *) CURL_NET_FLAGS+=(--retry-connrefused) ;;
  esac
fi

preflight_require_root() {
  [ "$(id -u)" -eq 0 ] || die "must run as root (try: sudo bash install.sh)"
}

preflight_require_systemd() {
  command -v systemctl >/dev/null 2>&1 || die "systemd (systemctl) not found — singbox-vpn requires a systemd-based host."
  [ -d /run/systemd/system ] || die "systemd does not appear to be the running init system (/run/systemd/system missing)."
}

preflight_require_commands() {
  local missing=()
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || missing+=("$c")
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    die "required command(s) not found: ${missing[*]}"
  fi
}

# Disk space check: $1 = path, $2 = minimum free MiB.
preflight_check_disk_space() {
  local path="$1" min_mib="$2" avail_mib
  avail_mib="$(df -Pm "$path" | awk 'NR==2 {print $4}')"
  if [ -z "$avail_mib" ]; then
    warn "could not determine free disk space on $path; continuing."
    return 0
  fi
  if [ "$avail_mib" -lt "$min_mib" ]; then
    die "insufficient disk space on $path: ${avail_mib}MiB free, need at least ${min_mib}MiB."
  fi
  log "disk space OK: ${avail_mib}MiB free on $path"
}

# RAM check: warns (does not fail) below threshold, since building Rust
# from source is the only thing that actually needs headroom, and that
# path is a fallback, not the common case once releases are published.
preflight_check_memory() {
  local min_mib="$1" total_mib
  total_mib="$(awk '/MemTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo "")"
  if [ -z "$total_mib" ]; then
    warn "could not determine total memory; continuing."
    return 0
  fi
  if [ "$total_mib" -lt "$min_mib" ]; then
    warn "low memory: ${total_mib}MiB total (recommended: ${min_mib}MiB+). Building from source may be slow or OOM; prebuilt release binaries are preferred when available."
  else
    log "memory OK: ${total_mib}MiB total"
  fi
}

# Shared bounded-retry curl wrapper for every network-dependent preflight
# check below. A single transient blip against one URL (a dropped
# packet, a momentary DNS hiccup, a mid-TLS-handshake stall, or a
# transient connection-refused response) must not hard-abort the whole
# installer — but retries are still bounded, and failure after they're
# exhausted is still a hard failure (fail-closed).
# `$@` = the same args you'd pass straight to `curl` (including the
# URL), e.g. `preflight_curl_retry -fsS -o /dev/null "$url"`.
#
# Reuses the caller's own CURL_NET_FLAGS array (install.sh and the root
# bootstrap installer both already define one, sourced before this file)
# when present, so retry/stall-timeout behavior stays defined in exactly
# one place per script rather than drifting between two copies. Falls
# back to an equivalent inline default only when no such array is
# defined (e.g. a test or another caller sources preflight.sh standalone).
preflight_curl_retry() {
  local flags
  if declare -p CURL_NET_FLAGS >/dev/null 2>&1; then
    flags=("${CURL_NET_FLAGS[@]}")
  else
    flags=(--connect-timeout 10 --max-time 60 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2 --retry-connrefused)
  fi
  if curl "${flags[@]}" "$@"; then
    return 0
  fi
  # One further fallback attempt preferring IPv4, tried only AFTER a
  # normal attempt already failed — never the only mode used — so a
  # working IPv6-only or dual-stack host is never broken by forcing
  # IPv4. This covers the class of failure where DNS/routing picks a
  # broken AAAA path on a dual-stack host while IPv4 egress is fine.
  curl -4 "${flags[@]}" "$@"
}

preflight_check_connectivity() {
  local url="${1:-https://github.com}"
  if ! preflight_curl_retry -fsS -o /dev/null "$url"; then
    die "no outbound internet connectivity (failed to reach $url after retries). singbox-vpn needs to download sing-box/binaries during install."
  fi
  log "internet connectivity OK ($url reachable)"
}

preflight_check_dns() {
  local host="${1:-github.com}"
  if command -v getent >/dev/null 2>&1; then
    getent hosts "$host" >/dev/null 2>&1 || { warn "DNS resolution for $host failed."; return 1; }
  elif command -v host >/dev/null 2>&1; then
    host "$host" >/dev/null 2>&1 || { warn "DNS resolution for $host failed."; return 1; }
  else
    warn "no DNS lookup tool (getent/host) available; skipping DNS check."
    return 0
  fi
  log "DNS resolution OK ($host)"
}

# Detect whether a port is already bound by something other than singbox-vpn's
# own services. $1 = proto (tcp|udp), $2 = port.
preflight_check_port_free() {
  local proto="$1" port="$2" owner="" ss_flag
  command -v ss >/dev/null 2>&1 || return 0
  case "$proto" in
    tcp) ss_flag="-lntp" ;;
    udp) ss_flag="-lnup" ;;
    *) return 0 ;;
  esac
  owner="$(ss -H "$ss_flag" 2>/dev/null | awk -v p=":$port\$" '$4 ~ p {print; exit}' || true)"
  if [ -n "$owner" ]; then
    echo "Port ${proto^^}/$port is already used by:" >&2
    echo "  $owner" >&2
    return 1
  fi
  return 0
}

# Detect the port sshd actually listens on, so the firewall scripts can
# guarantee the operator's real SSH session before enabling a
# default-deny firewall (docs/FINAL_PRODUCTION_AUDIT.md P0-10). Does NOT
# assume 22 — checks, in order: sshd's own effective config (`sshd -T`),
# static sshd_config `Port` directives, then a live listener owned by
# sshd.
#
# FAIL-CLOSED (checkpoint-1 requirement): if every detection method is
# inconclusive, this prints NOTHING and returns 1 — it never guesses 22.
# A caller about to activate/enable/reload a host firewall MUST treat a
# non-zero return here as fatal (die before touching the firewall), not
# as "assume 22 and continue". Use preflight_resolve_ssh_port() below
# instead of calling this directly — it also honours an explicit
# operator override for the case where detection is genuinely
# impossible.
preflight_detect_ssh_port() {
  local sshd_config_file="${SSHD_CONFIG_FILE:-/etc/ssh/sshd_config}"
  local port=""
  if command -v sshd >/dev/null 2>&1; then
    port="$(sshd -T 2>/dev/null | awk '/^port /{print $2; exit}' || true)"
  fi
  if [ -z "$port" ] && [ -f "$sshd_config_file" ]; then
    port="$(awk 'tolower($1)=="port"{print $2; exit}' "$sshd_config_file" 2>/dev/null)"
  fi
  if [ -z "$port" ] && command -v ss >/dev/null 2>&1; then
    port="$(ss -H -lntp 2>/dev/null | grep -E 'sshd|"ssh"' | head -n1 | sed -E 's/.*:([0-9]+)[[:space:]].*/\1/' || true)"
  fi
  if [ -z "$port" ]; then
    return 1
  fi
  if ! [[ "$port" =~ ^[0-9]+$ ]] || [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
    return 1
  fi
  echo "$port"
  return 0
}

# Canonical single entry point for resolving the SSH port that must
# survive any firewall activation/reload — every caller (install.sh,
# firewall.sh, firewall-ufw.sh) uses this, never its own copy of the
# detect-or-fallback logic. Precedence:
#   1. an explicit operator override, SINGBOX_VPN_SSH_PORT (set by install.sh's
#      --ssh-port/SINGBOX_VPN_SSH_PORT from a positively-known value, or by an
#      operator invoking firewall.sh/firewall-ufw.sh directly);
#   2. automatic detection (preflight_detect_ssh_port).
# Returns 1 with nothing printed if neither yields a valid port — the
# caller must fail closed (refuse to activate/modify the firewall)
# rather than default to 22.
preflight_resolve_ssh_port() {
  local override="${SINGBOX_VPN_SSH_PORT:-}"
  if [ -n "$override" ]; then
    preflight_validate_port "$override" "--ssh-port/SINGBOX_VPN_SSH_PORT" || return 1
    echo "$override"
    return 0
  fi
  preflight_detect_ssh_port
}

# Validate a value before it is ever interpolated into a TOML file,
# nginx config, or shell command (docs/FINAL_PRODUCTION_AUDIT.md P1
# "validate all operator input"). Deliberately conservative: a real
# hostname/IP never contains whitespace, quotes, shell metacharacters, or
# newlines, so anything containing them is rejected outright rather than
# guessed at.
preflight_validate_hostname() {
  local value="$1" label="${2:-hostname}"
  [ -n "$value" ] || { echo "$label must not be empty" >&2; return 1; }
  case "$value" in
    *[\ \'\"\`\$\\\;\&\|\<\>\(\)\{\}$'\n']*)
      echo "$label '$value' contains characters that are never valid in a hostname/IP — refusing to use it." >&2
      return 1
      ;;
  esac
  if ! [[ "$value" =~ ^[A-Za-z0-9]([A-Za-z0-9-]{0,62})?(\.[A-Za-z0-9]([A-Za-z0-9-]{0,62})?)*$ ]]; then
    echo "$label '$value' is not a syntactically valid hostname/IP — refusing to use it." >&2
    return 1
  fi
  return 0
}

preflight_validate_port() {
  local value="$1" label="${2:-port}"
  if ! [[ "$value" =~ ^[0-9]+$ ]] || [ "$value" -lt 1 ] || [ "$value" -gt 65535 ]; then
    echo "$label '$value' is not a valid TCP/UDP port (1-65535) — refusing to use it." >&2
    return 1
  fi
  return 0
}

# Verifies $1 (a hostname the operator wants to use for PUBLIC_HOST)
# actually resolves to one of THIS host's own addresses, before any
# certificate is requested for it. Best-effort: DNS/NAT/multi-homed
# hosts make a perfect check impossible from inside the host itself, so
# this compares DNS answers against every locally-configured address
# (`hostname -I`) plus the outbound-detected public IP
# (preflight_detect_public_ip) — the same signal a real client and
# Let's Encrypt's own HTTP-01 validator would ultimately use. Returns 1
# (and prints the mismatch) if DNS resolves to something that is
# provably NOT this host (e.g. it still points at a previous server, or
# is fronted by a proxy/CDN whose edge IP is not this VPS) — callers
# should treat that as a hard stop before touching the firewall/certbot,
# not merely a warning, since certbot's HTTP-01 challenge is guaranteed
# to fail (or worse, silently validate against the WRONG server) in that
# case. Returns 2 (skip / could not verify) when no DNS tool is
# available or DNS returns nothing — callers should warn, not hard-fail,
# since that is inconclusive rather than a confirmed mismatch.
preflight_check_hostname_resolves_here() {
  local host="$1" resolved my_ips r match=0
  if command -v getent >/dev/null 2>&1; then
    resolved="$(getent ahosts "$host" 2>/dev/null | awk '{print $1}' | sort -u)"
  elif command -v host >/dev/null 2>&1; then
    resolved="$(host "$host" 2>/dev/null | awk '/has (address|IPv6 address)/{print $NF}' | sort -u)"
  else
    warn "no DNS lookup tool (getent/host) available; cannot verify $host resolves to this server."
    return 2
  fi
  if [ -z "$resolved" ]; then
    echo "DNS for '$host' did not resolve to any address at all." >&2
    return 2
  fi
  my_ips="$( { command -v hostname >/dev/null 2>&1 && hostname -I 2>/dev/null; preflight_detect_public_ip 2>/dev/null; } | tr ' ' '\n' | sed '/^$/d' | sort -u)"
  while IFS= read -r r; do
    [ -z "$r" ] && continue
    grep -qx "$r" <<<"$my_ips" && match=1
  done <<EOF
$resolved
EOF
  if [ "$match" -eq 1 ]; then
    log "DNS for $host resolves to this server ($(printf '%s' "$resolved" | tr '\n' ' '))."
    return 0
  fi
  echo "DNS for '$host' resolves to: $(printf '%s' "$resolved" | tr '\n' ' ')" >&2
  echo "  ...none of which match this server's own addresses: $(printf '%s' "$my_ips" | tr '\n' ' ')" >&2
  return 1
}

# Own-address detection (I-15). The renderer can only deny a tunnel user
# access to the node's own public address if deployment.toml records it
# (public_ipv4/public_ipv6); these helpers establish that address from
# evidence the node itself controls. DNS is deliberately NOT one of them:
# a record proves what the domain's owner configured, not which addresses
# belong to this machine.

# 0 when $1 is a syntactically valid global-unicast address that no private,
# loopback, link-local, CGNAT, multicast or reserved range covers (IPv6:
# 2000::/3 only, which excludes ULA fc00::/7 and link-local fe80::/10).
preflight_is_public_ip() {
  local ip="${1:-}" a b first
  if [[ "$ip" =~ ^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})$ ]]; then
    local c d
    a=$((10#${BASH_REMATCH[1]})) b=$((10#${BASH_REMATCH[2]}))
    c=$((10#${BASH_REMATCH[3]})) d=$((10#${BASH_REMATCH[4]}))
    (( a <= 255 && b <= 255 && c <= 255 && d <= 255 )) || return 1
    (( a == 0 || a == 10 || a == 127 || a >= 224 )) && return 1
    (( a == 100 && b >= 64 && b <= 127 )) && return 1
    (( a == 169 && b == 254 )) && return 1
    (( a == 172 && b >= 16 && b <= 31 )) && return 1
    (( a == 192 && b == 168 )) && return 1
    # 192.88.99.0/24: deprecated 6to4 relay anycast (RFC 7526); also used by this
    # product's own arcana-2hop0 interface, never a host's public address (I-57).
    (( a == 192 && b == 88 && c == 99 )) && return 1
    (( a == 198 && (b == 18 || b == 19) )) && return 1
    return 0
  fi
  if [[ "$ip" =~ ^[0-9a-fA-F]{0,4}(:[0-9a-fA-F]{0,4}){2,7}$ ]] && [[ "$ip" != *:::* ]]; then
    [ "$(grep -o '::' <<<"$ip" | wc -l)" -le 1 ] || return 1
    first="${ip%%:*}"
    [ -n "$first" ] || return 1
    a=$((16#$first))
    (( a >= 0x2000 && a <= 0x3fff )) && return 0
  fi
  return 1
}

# Prints 4 or 6 for an address literal, nothing (and returns 1) otherwise.
preflight_ip_family() {
  case "${1:-}" in
    *[!0-9a-fA-F:.]*|"") return 1 ;;
    *:*) echo 6 ;;
    *.*) echo 4 ;;
    *) return 1 ;;
  esac
}

# Public global-scope addresses currently configured on this host's
# interfaces, one per line, for family $1 (4|6). Skips temporary/deprecated/
# tentative IPv6 privacy addresses. Empty output (return 0) when there are none.
preflight_local_public_addrs() {
  local fam="${1:?family}" line addr
  command -v ip >/dev/null 2>&1 || return 0
  ip -o "-$fam" addr show scope global 2>/dev/null | while IFS= read -r line; do
    case " $line " in *" temporary "*|*" deprecated "*|*" tentative "*|*" dadfailed "*) continue ;; esac
    addr="$(printf '%s\n' "$line" | awk '{for(i=1;i<=NF;i++) if($i=="inet"||$i=="inet6"){print $(i+1); exit}}')"
    addr="${addr%%/*}"
    if preflight_is_public_ip "$addr"; then echo "$addr"; fi
  done | sort -u || true
  return 0
}

# The address the internet sees this host as, for family $1, accepted only
# when two independent services return the same public address (one service
# alone could be wrong or hostile). Used for hosts whose public address is not
# configured on an interface (NAT / floating IP).
preflight_detect_egress_ip_confirmed() {
  local fam="${1:?family}" url ip first=""
  echo "[install] confirming this server's public IPv$fam address with two of api.ipify.org / ifconfig.me / icanhazip.com (set PUBLIC_IPV$fam to skip this)..." >&2
  for url in "https://api.ipify.org" "https://ifconfig.me/ip" "https://icanhazip.com"; do
    ip="$(curl -fsS "-$fam" --connect-timeout 5 --max-time 10 "$url" 2>/dev/null | tr -d '[:space:]')" || ip=""
    [ -n "$ip" ] || continue
    [ "$(preflight_ip_family "$ip" 2>/dev/null)" = "$fam" ] || continue
    preflight_is_public_ip "$ip" || continue
    if [ -z "$first" ]; then
      first="$ip"
    elif [ "$first" = "$ip" ]; then
      echo "$ip"
      return 0
    else
      first="$ip"
    fi
  done
  return 1
}

# Addresses $1 resolves to (A and AAAA), one per line. Only ever used to pick
# between addresses already proven to be this host's own.
preflight_resolve_host_addrs() {
  local host="${1:?host}"
  if command -v getent >/dev/null 2>&1; then
    getent ahosts "$host" 2>/dev/null | awk '{print $1}' | sort -u
  fi
  return 0
}

preflight_detect_public_ip() {
  # Disclosed, not silent: with no PUBLIC_HOST/--domain given, this asks
  # one of these three third-party services "what IP did this request
  # come from" so the installer can pick a default sslip.io hostname.
  # Each one necessarily learns this VPS's address to answer that — same
  # as any outbound connection would reveal it — but say so up front
  # rather than let it happen as an unannounced side effect of a
  # preflight check. Skip entirely by setting PUBLIC_HOST/--domain.
  # Deliberately >&2, not `log` (which writes to stdout in install.sh):
  # this function's own stdout is the detected IP, captured via command
  # substitution by its caller — anything this function writes to stdout
  # would corrupt that captured value.
  echo "[install] auto-detecting this server's public IP via api.ipify.org / ifconfig.me / icanhazip.com (set PUBLIC_HOST or --domain to skip this)..." >&2
  local ip=""
  for url in "https://api.ipify.org" "https://ifconfig.me/ip" "https://icanhazip.com"; do
    # `|| true`: under `set -e`/pipefail a failed lookup (timeout,
    # non-200, DNS block) would otherwise abort the whole installer on
    # the first candidate instead of trying the next fallback URL.
    ip="$(preflight_curl_retry -fsS "$url" 2>/dev/null | tr -d '[:space:]')" || true
    if [ -n "$ip" ] && [[ "$ip" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]]; then
      echo "$ip"
      return 0
    fi
  done
  return 1
}

# --- Build disk budget (I-81; reconstructed 2026-10-22, original 3d7c005 MISSING) ---
# The previous fin1 filled its disk because each qualification run copied the
# source tree and built into its own multi-GB Rust target dir; ~7 stale copies
# accumulated and the host had to be rebuilt. Rules:
#   * every local build uses ONE shared target dir under a managed cache root;
#   * a build starts only if free space >= operational reserve + build estimate,
#     and inodes are not nearly exhausted (fail closed if either is unknown);
#   * abandoned output is removed only from direct children of the managed cache
#     that carry a disposable marker, never through a symlink, never a git repo,
#     and only with --apply (dry run is the default).
# Defaults (override per host): reserve 10 GiB keeps the VPN, logs, backups and
# an emergency margin alive on the 38 GB fin1 disk (~25%); the estimate of
# 8 GiB covers one workspace debug+test target. Neither was measured on the
# rebuilt host yet: qualify-local.sh records the real target size after each
# run (target_dir_mib in ENVIRONMENT.txt) so the estimate can be replaced by data.
SINGBOX_VPN_BUILD_CACHE="${SINGBOX_VPN_BUILD_CACHE:-/var/cache/singbox-vpn-build}"
SINGBOX_VPN_BUILD_RESERVE_MIB="${SINGBOX_VPN_BUILD_RESERVE_MIB:-10240}"
SINGBOX_VPN_BUILD_ESTIMATE_MIB="${SINGBOX_VPN_BUILD_ESTIMATE_MIB:-8192}"
SINGBOX_VPN_BUILD_MIN_INODES="${SINGBOX_VPN_BUILD_MIN_INODES:-200000}"

# $1 = path, $2 = reserve MiB, $3 = build estimate MiB, $4 = min free inodes.
preflight_check_build_budget() {
  local path="$1" reserve="${2:-$SINGBOX_VPN_BUILD_RESERVE_MIB}" est="${3:-$SINGBOX_VPN_BUILD_ESTIMATE_MIB}"
  local min_inodes="${4:-$SINGBOX_VPN_BUILD_MIN_INODES}" out avail ifree need
  out="$(df -Pm "$path" 2>/dev/null || true)"
  avail="$(awk 'NR==2 {print $4}' <<<"$out")"
  out="$(df -Pi "$path" 2>/dev/null || true)"
  ifree="$(awk 'NR==2 {print $4}' <<<"$out")"
  need=$((reserve + est))
  if ! [[ "$avail" =~ ^[0-9]+$ ]] || ! [[ "$ifree" =~ ^[0-9]+$ ]]; then
    echo "build budget: cannot determine free space/inodes on $path; refusing to build." >&2
    return 1
  fi
  if [ "$avail" -lt "$need" ]; then
    echo "build budget: ${avail}MiB free on $path, need ${need}MiB (reserve ${reserve} + build ${est}); refusing to build. Dry run: preflight_prune_build_cache $SINGBOX_VPN_BUILD_CACHE" >&2
    return 1
  fi
  if [ "$ifree" -lt "$min_inodes" ]; then
    echo "build budget: ${ifree} free inodes on $path, need ${min_inodes}; refusing to build." >&2
    return 1
  fi
  log "build budget OK: ${avail}MiB / ${ifree} inodes free on $path (need ${need}MiB)"
}

# Creates the managed cache and exports CARGO_TARGET_DIR (shared) and
# CARGO_BUILD_JOBS (<= nproc, ~1.5 GiB RAM per rustc job, at least 1).
preflight_cargo_build_env() {
  local root="$SINGBOX_VPN_BUILD_CACHE" mem_mib jobs cpus
  [ -L "$root" ] && { echo "build cache $root is a symlink; refusing." >&2; return 1; }
  mkdir -p "$root/target" || return 1
  : >>"$root/.singbox-vpn-build-cache"
  cpus="$(nproc 2>/dev/null || echo 1)"
  mem_mib="$(awk '/MemTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 0)"
  jobs=$((mem_mib / 1536)); [ "$jobs" -gt "$cpus" ] && jobs="$cpus"; [ "$jobs" -lt 1 ] && jobs=1
  export CARGO_TARGET_DIR="$root/target" CARGO_BUILD_JOBS="$jobs"
  log "cargo: shared target $CARGO_TARGET_DIR, $jobs job(s)"
}

# Installer/updater source builds (fallback when no release binary is used): they build inside their own per-run scratch
# workspace, which the installer removes afterwards (I-33), so there is no accumulation. They still need the I-81 budget and
# bounded parallelism. $1 = the directory the build writes into. Exports CARGO_BUILD_JOBS; does not move the target dir.
preflight_source_build_guard() {
  local dir="${1:?build dir}" cpus mem_mib jobs
  preflight_check_build_budget "$dir" || return 1
  cpus="$(nproc 2>/dev/null || echo 1)"
  mem_mib="$(awk '/MemTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 0)"
  jobs=$((mem_mib / 1536)); [ "$jobs" -gt "$cpus" ] && jobs="$cpus"; [ "$jobs" -lt 1 ] && jobs=1
  export CARGO_BUILD_JOBS="$jobs"
}

# $1 = managed cache root; options: --apply (default is a dry run),
# --include-shared (also the shared target/, only when no build holds $root/.lock).
preflight_prune_build_cache() {
  local root="${1:?cache root}" apply=0 shared=0 real entry name
  shift
  for a in "$@"; do case "$a" in --apply) apply=1 ;; --include-shared) shared=1 ;; *) echo "unknown option $a" >&2; return 2 ;; esac; done
  [ -d "$root" ] && [ ! -L "$root" ] || { echo "prune: $root is not a real directory" >&2; return 1; }
  real="$(cd "$root" && pwd -P)"
  [ "$real" != "/" ] && [ -f "$real/.singbox-vpn-build-cache" ] || { echo "prune: $root is not a managed build cache" >&2; return 1; }
  exec {lockfd}>>"$real/.lock"
  if ! flock -n "$lockfd"; then
    exec {lockfd}>&-
    if [ "$shared" -eq 1 ]; then echo "prune: a build holds $real/.lock; refusing." >&2; return 1; fi
    echo "prune: a build is active; only marked abandoned output is eligible." >&2
  fi
  for entry in "$real"/*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    name="${entry##*/}"
    [ -L "$entry" ] && continue                      # never follow out of scope
    [ -d "$entry" ] || continue
    [ -e "$entry/.git" ] && continue                 # never a repository
    if [ "$name" = target ]; then
      [ "$shared" -eq 1 ] || continue
    else
      [ -f "$entry/.singbox-vpn-disposable" ] || continue
    fi
    if [ "$apply" -eq 1 ]; then rm -rf --one-file-system -- "$entry" && echo "removed $entry"
    else echo "would remove $entry ($(du -sm "$entry" 2>/dev/null | cut -f1)MiB)"; fi
  done
  [ -n "${lockfd:-}" ] && { exec {lockfd}>&- || true; } 2>/dev/null
  return 0
}
