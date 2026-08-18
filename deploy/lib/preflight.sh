#!/usr/bin/env bash
# Shared preflight checks. Sourced by deploy/almalinux/install.sh and the
# root bootstrap installer. Expects log()/warn()/die() to already be
# defined by the caller.

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
# packet, a momentary DNS hiccup, a mid-TLS-handshake stall) must not
# hard-abort the whole installer — but retries are still bounded, and
# failure after they're exhausted is still a hard failure (fail-closed).
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
    flags=(--connect-timeout 10 --max-time 60 --speed-limit 1024 --speed-time 30 --retry 3 --retry-delay 2)
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
  owner="$(ss -H "$ss_flag" 2>/dev/null | awk -v p=":$port\$" '$4 ~ p {print; exit}')"
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
    port="$(sshd -T 2>/dev/null | awk '/^port /{print $2; exit}')"
  fi
  if [ -z "$port" ] && [ -f "$sshd_config_file" ]; then
    port="$(awk 'tolower($1)=="port"{print $2; exit}' "$sshd_config_file" 2>/dev/null)"
  fi
  if [ -z "$port" ] && command -v ss >/dev/null 2>&1; then
    port="$(ss -H -lntp 2>/dev/null | grep -E 'sshd|"ssh"' | head -n1 | sed -E 's/.*:([0-9]+)[[:space:]].*/\1/')"
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
#   1. an explicit operator override, VPN1_SSH_PORT (set by install.sh's
#      --ssh-port/VPN1_SSH_PORT from a positively-known value, or by an
#      operator invoking firewall.sh/firewall-ufw.sh directly);
#   2. automatic detection (preflight_detect_ssh_port).
# Returns 1 with nothing printed if neither yields a valid port — the
# caller must fail closed (refuse to activate/modify the firewall)
# rather than default to 22.
preflight_resolve_ssh_port() {
  local override="${VPN1_SSH_PORT:-}"
  if [ -n "$override" ]; then
    preflight_validate_port "$override" "--ssh-port/VPN1_SSH_PORT" || return 1
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
    printf '%s\n' "$my_ips" | grep -qx "$r" && match=1
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

preflight_detect_public_ip() {
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
