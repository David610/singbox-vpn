#!/usr/bin/env bash
# Shared preflight checks. Sourced by deploy/almalinux/install.sh and the
# root bootstrap installer. Expects log()/warn()/die() to already be
# defined by the caller.

preflight_require_root() {
  [ "$(id -u)" -eq 0 ] || die "must run as root (try: sudo bash install.sh)"
}

preflight_require_systemd() {
  command -v systemctl >/dev/null 2>&1 || die "systemd (systemctl) not found — vpn1 requires a systemd-based host."
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

preflight_check_connectivity() {
  local url="${1:-https://github.com}"
  if ! curl -fsS --max-time 8 -o /dev/null "$url"; then
    die "no outbound internet connectivity (failed to reach $url). vpn1 needs to download sing-box/binaries during install."
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

# Detect whether a port is already bound by something other than vpn1's
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
# sshd. Falls back to 22 with a loud warning only if every detection
# method is inconclusive (e.g. sshd not installed as a systemd unit
# named `sshd`/`ssh`).
preflight_detect_ssh_port() {
  local port=""
  if command -v sshd >/dev/null 2>&1; then
    port="$(sshd -T 2>/dev/null | awk '/^port /{print $2; exit}')"
  fi
  if [ -z "$port" ] && [ -f /etc/ssh/sshd_config ]; then
    port="$(awk 'tolower($1)=="port"{print $2; exit}' /etc/ssh/sshd_config 2>/dev/null)"
  fi
  if [ -z "$port" ] && command -v ss >/dev/null 2>&1; then
    port="$(ss -H -lntp 2>/dev/null | grep -E 'sshd|"ssh"' | head -n1 | sed -E 's/.*:([0-9]+)[[:space:]].*/\1/')"
  fi
  if [ -z "$port" ]; then
    echo "22"
    return 1
  fi
  if ! [[ "$port" =~ ^[0-9]+$ ]] || [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
    echo "22"
    return 1
  fi
  echo "$port"
  return 0
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

preflight_detect_public_ip() {
  local ip=""
  for url in "https://api.ipify.org" "https://ifconfig.me/ip" "https://icanhazip.com"; do
    ip="$(curl -fsS --max-time 6 "$url" 2>/dev/null | tr -d '[:space:]')"
    if [ -n "$ip" ] && [[ "$ip" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]]; then
      echo "$ip"
      return 0
    fi
  done
  return 1
}
