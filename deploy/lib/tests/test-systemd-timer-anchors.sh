#!/usr/bin/env bash
# Regression test: every repeating shipped systemd timer must have a trigger
# that fires WITHOUT a previous run of its service.
#
# `OnUnitActiveSec=` counts from the last time the service was activated and
# `OnBootSec=` only fires once, shortly after boot. A timer that has only those
# two is never scheduled on a host where it is started after boot and its
# service has not run yet, which is exactly the state right after install:
# `systemctl list-timers` shows NEXT "-" and the timer reports active (waiting)
# forever, until the next reboot. For vpn-expiry-reconcile.timer that means
# expired credentials are never removed from the live server (found on three
# real VPSs installed from v1.1.0-rc.9: an expired 30-minute lease still
# authenticated 11+ minutes later); for vpn-service-watchdog.timer it means a
# parked failed unit is never recovered.
#
# `OnActiveSec=` (relative to the timer's own activation), `OnStartupSec=` or
# `OnCalendar=` start the cycle.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UNIT_DIR="$REPO_ROOT/deploy/almalinux/systemd"

failures=0
checked=0

shopt -s nullglob
for timer in "$UNIT_DIR"/*.timer; do
  name="$(basename "$timer")"
  # Only the [Timer] section's directives, comments and CRs removed.
  directives="$(sed -n '/^\[Timer\]/,/^\[/p' "$timer" | tr -d '\r' | grep -v '^[[:space:]]*#' || true)"
  if ! grep -Eq '^(OnUnitActiveSec|OnUnitInactiveSec)=' <<<"$directives"; then
    echo "skip: $name has no relative-to-last-run trigger"
    continue
  fi
  checked=$((checked + 1))
  if grep -Eq '^(OnActiveSec|OnStartupSec|OnCalendar)=' <<<"$directives"; then
    echo "ok: $name has a trigger that starts the cycle without a previous service run"
  else
    echo "FAIL: $name repeats with OnUnitActiveSec/OnUnitInactiveSec but has no OnActiveSec=, OnStartupSec= or OnCalendar= to start the cycle; it is never scheduled after install until the next reboot"
    failures=$((failures + 1))
  fi
done

if [ "$checked" -eq 0 ]; then
  echo "FAIL: found no repeating timer units to check under $UNIT_DIR" >&2
  exit 1
fi
if [ "$failures" -ne 0 ]; then
  echo "$failures timer(s) FAILED"
  exit 1
fi
echo "PASS: all $checked repeating timers start their cycle without a previous service run"
