#!/usr/bin/env bash
# vpn-service-watchdog: a systemd-native safety net, nothing more. It
# exists for exactly one reason: sing-box.service/vpn-subscription.service
# now set an EXPLICIT, generous StartLimitIntervalSec/StartLimitBurst
# (see those unit files) so a burst of transient startup crashes survives
# instead of parking the unit `failed` after systemd's old implicit
# 10s/5-restart default. But "generous" still isn't "infinite" — a
# failure that takes longer than that budget's window to clear (a slow
# DHCP renewal, an ISP-side blip, a certificate renewal race) WOULD
# otherwise leave a fully recoverable service down forever, since once a
# unit hits `start-limit-hit`, systemd refuses even a later `systemctl
# start` until `systemctl reset-failed` runs first.
#
# Run periodically (low frequency — see vpn-service-watchdog.timer) by
# that timer. For each unit listed below, if it is CURRENTLY in a
# `failed` state, clear that state and ask systemd to start it again.
# That's the entire behavior:
#   - a unit that is running fine is untouched (not `failed`).
#   - a unit an operator deliberately stopped (`systemctl stop ...`) is
#     `inactive`, never `failed`, so `systemctl stop` continues to behave
#     normally — this script never fights a deliberate stop.
#   - a unit that is still genuinely broken just fails again immediately
#     and is picked up on the next tick — never a fast loop, since ticks
#     are minutes apart (see the timer), and never silent: every
#     recovery attempt is logged, and `vpn-admin doctor`/`vpn status`
#     read the exact same `systemctl` state this script does.
#
# Deliberately NOT a monitoring/control-plane service: no metrics, no
# persistent state of its own, no HTTP, no decisions beyond "is this
# unit failed right now — if so, give it one more chance." Every fact it
# acts on is asked of systemd itself, live, every time it runs.
set -uo pipefail

UNITS=(sing-box.service vpn-subscription.service)

status=0
for unit in "${UNITS[@]}"; do
  if systemctl is-failed --quiet "$unit" 2>/dev/null; then
    echo "vpn-service-watchdog: $unit is in a failed state — clearing and retrying"
    # `systemctl start` on a unit that hit its StartLimitBurst is
    # refused ("start of the unit process was attempted too often")
    # unless reset-failed runs first — this is systemd's own documented
    # remediation, not a workaround.
    systemctl reset-failed "$unit" 2>/dev/null || true
    if systemctl start "$unit"; then
      echo "vpn-service-watchdog: $unit restarted successfully"
    else
      echo "vpn-service-watchdog: $unit failed again immediately — see: journalctl -u $unit --no-pager -n 50" >&2
      status=1
    fi
  fi
done

exit "$status"
