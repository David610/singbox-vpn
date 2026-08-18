#!/usr/bin/env bash
# Regression coverage for P8: sing-box.service and vpn-subscription.service
# must carry conservative cgroup resource protection (so one runaway/
# compromised process cannot take the whole host down) without a CPUQuota
# tight enough to throttle real VPN traffic — see the file-header comments
# on each unit for the full rationale.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
SINGBOX_UNIT="$ROOT/deploy/almalinux/systemd/sing-box.service"
SUB_UNIT="$ROOT/deploy/almalinux/systemd/vpn-subscription.service"

fail() { echo "FAIL: $*" >&2; exit 1; }

for unit in "$SINGBOX_UNIT" "$SUB_UNIT"; do
  [ -f "$unit" ] || fail "missing unit file: $unit"
  grep -qE '^MemoryHigh=[0-9]+%$' "$unit" || fail "$unit: MemoryHigh must be a percentage-based soft throttle"
  grep -qE '^MemoryMax=[0-9]+%$' "$unit" || fail "$unit: MemoryMax must be a percentage-based hard ceiling"
  grep -qE '^TasksMax=[0-9]+$' "$unit" || fail "$unit: TasksMax must be set"
  # CPUQuota is deliberately never set on either unit — a tight CPU quota
  # is exactly the false-positive-prone protection this project chose not
  # to add (it would throttle legitimate Hysteria2/QUIC crypto bursts and
  # reproduce the silent-degradation class this whole effort targets).
  if grep -qE '^CPUQuota=' "$unit"; then
    fail "$unit: CPUQuota must not be set — it risks throttling real VPN traffic (see the unit's own comment)"
  fi
done

# MemoryHigh must always be strictly below MemoryMax on both units — a
# soft throttle at or above the hard kill ceiling would never actually
# engage before the kill, defeating its purpose.
for unit in "$SINGBOX_UNIT" "$SUB_UNIT"; do
  high=$(grep -oE '^MemoryHigh=[0-9]+' "$unit" | cut -d= -f2)
  max=$(grep -oE '^MemoryMax=[0-9]+' "$unit" | cut -d= -f2)
  [ "$high" -lt "$max" ] || fail "$unit: MemoryHigh ($high%) must be less than MemoryMax ($max%)"
done

# sing-box carries the internet-facing, higher-throughput data plane —
# its resource ceilings must never be TIGHTER than the loopback-only
# subscription service's, or a real VPN load could be constrained more
# aggressively than an idle API service.
singbox_max=$(grep -oE '^MemoryMax=[0-9]+' "$SINGBOX_UNIT" | cut -d= -f2)
sub_max=$(grep -oE '^MemoryMax=[0-9]+' "$SUB_UNIT" | cut -d= -f2)
[ "$singbox_max" -ge "$sub_max" ] || fail "sing-box.service's MemoryMax ($singbox_max%) must not be tighter than vpn-subscription.service's ($sub_max%)"

if command -v systemd-analyze >/dev/null 2>&1; then
  # `verify` still complains about the referenced binaries not existing
  # on a bare checkout (expected — they're installed by install.sh) but
  # must not report any unit-syntax/directive error for the new lines.
  out=$(systemd-analyze verify "$SINGBOX_UNIT" "$SUB_UNIT" 2>&1 || true)
  if echo "$out" | grep -iE 'MemoryHigh|MemoryMax|TasksMax' | grep -viE 'not executable'; then
    fail "systemd-analyze flagged the new resource-protection directives:\n$out"
  fi
else
  echo "systemd-analyze not available on this host — syntax-only checks above still ran"
fi

echo "systemd resource protection tests: PASS"
