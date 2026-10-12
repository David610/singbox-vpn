#!/usr/bin/env bash
# Bounded, read-only soak sampler for a singbox-vpn node.
#
# Runs ON the node for --duration seconds and appends one CSV row every
# --interval seconds. It reads only service state and /proc counters: no
# per-destination or per-user data, no credentials, no packet capture, and it
# changes nothing. Feed the CSV to deploy/lib/soak_evaluate.py, which applies
# criteria that are fixed BEFORE the run (see its header).
#
#   sudo bash deploy/lib/soak-sampler.sh --duration 5400 --interval 30 \
#        --output /root/soak.csv [--selftest-every 20]
#
# --selftest-every N runs `vpn-admin doctor --protocol --require-protocol`
# (a real REALITY handshake through the reserved probe principal) every N rows
# and records its pass/fail; use 0 to skip it. The probe is a normal, bounded
# dial; it does not restart services.
set -Eeuo pipefail

DURATION=5400
INTERVAL=30
OUTPUT=""
SELFTEST_EVERY=20
ADMIN="${SOAK_VPN_ADMIN:-/usr/local/bin/vpn-admin}"
SYSTEMCTL="${SOAK_SYSTEMCTL:-systemctl}"
PROC_ROOT="${SOAK_PROC_ROOT:-/proc}"

usage() { echo "usage: $0 --output FILE [--duration S] [--interval S] [--selftest-every N]" >&2; exit 2; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --duration) DURATION="$2"; shift 2 ;;
    --interval) INTERVAL="$2"; shift 2 ;;
    --output) OUTPUT="$2"; shift 2 ;;
    --selftest-every) SELFTEST_EVERY="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$OUTPUT" ] || usage
for n in "$DURATION" "$INTERVAL" "$SELFTEST_EVERY"; do
  case "$n" in ''|*[!0-9]*) echo "numeric arguments required" >&2; exit 2 ;; esac
done
[ "$INTERVAL" -ge 1 ] || { echo "--interval must be >= 1" >&2; exit 2; }
[ ! -e "$OUTPUT" ] || { echo "refusing to overwrite existing $OUTPUT" >&2; exit 2; }

umask 077
printf 'epoch,singbox_active,singbox_pid,singbox_restarts,singbox_rss_kb,singbox_fds,subscription_active,reconcile_result,reconcile_runs,watchdog_active,selftest\n' >"$OUTPUT"

prop() { "$SYSTEMCTL" show "$1" -p "$2" --value 2>/dev/null || true; }
active() { "$SYSTEMCTL" is-active --quiet "$1" 2>/dev/null && echo 1 || echo 0; }

# Count of completed reconcile runs: its InactiveEnterTimestamp changes once per run.
last_reconcile_stamp=""; reconcile_runs=0
end=$(( $(date +%s) + DURATION )); row=0
while :; do
  now="$(date +%s)"
  pid="$(prop sing-box.service MainPID)"; pid="${pid:-0}"
  rss=0; fds=0
  if [ "$pid" -gt 0 ] && [ -r "$PROC_ROOT/$pid/status" ]; then
    rss="$(awk '/^VmRSS:/ {print $2}' "$PROC_ROOT/$pid/status" 2>/dev/null || echo 0)"
    fds="$(find "$PROC_ROOT/$pid/fd" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l | tr -d ' ')"
  fi
  stamp="$(prop vpn-expiry-reconcile.service InactiveEnterTimestampMonotonic)"
  if [ -n "$stamp" ] && [ "$stamp" != "0" ] && [ "$stamp" != "$last_reconcile_stamp" ]; then
    last_reconcile_stamp="$stamp"; reconcile_runs=$((reconcile_runs + 1))
  fi
  selftest="na"
  if [ "$SELFTEST_EVERY" -gt 0 ] && [ $((row % SELFTEST_EVERY)) -eq 0 ]; then
    if "$ADMIN" doctor --protocol --require-protocol >/dev/null 2>&1; then selftest="pass"; else selftest="fail"; fi
  fi
  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$now" "$(active sing-box.service)" "$pid" "$(prop sing-box.service NRestarts)" "${rss:-0}" "${fds:-0}" \
    "$(active vpn-subscription.service)" "$(prop vpn-expiry-reconcile.service Result)" "$reconcile_runs" \
    "$(active vpn-service-watchdog.timer)" "$selftest" >>"$OUTPUT"
  row=$((row + 1))
  [ "$(date +%s)" -lt "$end" ] || break
  sleep "$INTERVAL"
done
echo "soak sample complete: $row rows in $OUTPUT"
