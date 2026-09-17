#!/usr/bin/env bash
# Host isolation for test runs on a live server. Sourced, not executed.
#
# `update.sh --dev-rebuild` (and `--repair` on an install with no pinned
# release) runs the Rust test suite on the production host, as root, before
# it touches installed state. During the real two-VPS acceptance (defect D6)
# tests that reached the real `systemctl` restarted the live relay until
# systemd's start limit tripped — a ~77 s outage — while the updater still
# reported that installed state was not changed.
#
# `run_isolated_from_host GUARD_DIR COMMAND...` runs COMMAND so it cannot
# control this host, at three independent layers:
#
#   1. As root, COMMAND runs in a private mount namespace in which the
#      systemd manager socket and the D-Bus system bus socket are replaced
#      by /dev/null. Even an absolute /usr/bin/systemctl, firewall-cmd or
#      reboot cannot reach PID 1, logind or firewalld. If that namespace
#      cannot be set up, COMMAND is not run at all.
#   2. PATH starts with guards for host-control tools. A guard refuses,
#      records a mutating attempt in GUARD_DIR/refused-host-control.log and
#      exits 97.
#   3. SINGBOX_VPN_SYSTEMCTL names the systemctl guard; vpn-admin uses it
#      with no PATH fallback.
#
# `host_service_fingerprint` reports the live units' identity so a caller
# can state, from observation, whether they were restarted meanwhile.

HOST_CONTROL_TOOLS=(
  systemctl service reboot shutdown poweroff halt telinit
  firewall-cmd ufw iptables ip6tables nft
  useradd userdel usermod groupadd groupdel
  pkill killall
)

# Service-manager IPC endpoints masked inside the namespace.
HOST_CONTROL_SOCKETS=(
  /run/systemd/private
  /run/dbus/system_bus_socket
)

HOST_SERVICE_UNITS=(sing-box vpn-subscription)

isolation_write_guards() {
  local dir="$1" tool
  mkdir -p "$dir/bin" || return 1
  : > "$dir/refused-host-control.log" || return 1
  for tool in "${HOST_CONTROL_TOOLS[@]}"; do
    cat > "$dir/bin/$tool" <<EOF || return 1
#!/bin/sh
# Read-only queries are refused silently; anything else is recorded.
case "$tool:\${1:-}" in
  systemctl:--version|systemctl:show|systemctl:status|systemctl:is-active|systemctl:is-enabled|systemctl:is-failed|systemctl:cat|systemctl:list-units|systemctl:list-timers) ;;
  firewall-cmd:--state|firewall-cmd:--get-*|firewall-cmd:--list-*|firewall-cmd:--query-*|firewall-cmd:--version) ;;
  *) echo "$tool \$*" >> "$dir/refused-host-control.log" ;;
esac
echo "host-control guard: refused '$tool \$*' during an isolated test run" >&2
exit 97
EOF
    chmod 0755 "$dir/bin/$tool" || return 1
  done
}

run_isolated_from_host() {
  local guard_dir="$1"
  shift
  isolation_write_guards "$guard_dir" || {
    echo "test isolation: could not create host-control guards in $guard_dir" >&2
    return 125
  }
  local guarded=(env "PATH=$guard_dir/bin:$PATH" "SINGBOX_VPN_SYSTEMCTL=$guard_dir/bin/systemctl")
  if [ "$(id -u)" -ne 0 ]; then
    "${guarded[@]}" "$@"
    return
  fi
  if ! command -v unshare >/dev/null 2>&1; then
    echo "test isolation: unshare(1) is not available; refusing to run tests as root on this host" >&2
    return 125
  fi
  # shellcheck disable=SC2016 # expanded by the inner shell, on purpose
  unshare --mount --propagation private -- bash -c '
    set -e
    count="$1"; shift
    i=0
    while [ "$i" -lt "$count" ]; do
      socket="$1"; shift; i=$((i + 1))
      if [ -e "$socket" ]; then
        mount --bind /dev/null "$socket"
      fi
    done
    exec "$@"
  ' isolated-test-run "${#HOST_CONTROL_SOCKETS[@]}" "${HOST_CONTROL_SOCKETS[@]}" "${guarded[@]}" "$@"
}

# One line per unit: MainPID, restart counter and activation time. Identical
# output before and after a step means systemd did not restart the unit.
host_service_fingerprint() {
  local unit
  for unit in "${HOST_SERVICE_UNITS[@]}"; do
    printf '%s %s\n' "$unit" \
      "$(systemctl show -p MainPID -p NRestarts -p ActiveEnterTimestampMonotonic "$unit.service" 2>/dev/null | LC_ALL=C sort | tr '\n' ' ')"
  done
}
