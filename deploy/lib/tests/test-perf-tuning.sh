#!/usr/bin/env bash
# Unit tests for deploy/lib/perf-tuning.sh's rendering, baseline-capture,
# and rollback logic. Runs entirely against a throwaway temp directory
# (PERF_SYSCTL_DROPIN/PERF_ROLLBACK_DROPIN/PERF_STATE_DIR/PERF_BASELINE_FILE
# overridden below) and stubs out `perf_read_sysctl`/
# `perf_apply_sysctl_system`/`perf_kernel_supports_bbr`/
# `perf_qdisc_available` — never touches real system sysctl state, never
# requires root. Safe to run in CI.
set -Eeuo pipefail

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

export PERF_SYSCTL_DROPIN="$TMPDIR_TEST/99-vpn1-dataplane.conf"
export PERF_ROLLBACK_DROPIN="$TMPDIR_TEST/99-vpn1-dataplane-rollback.conf"
export PERF_STATE_DIR="$TMPDIR_TEST/state"
export PERF_BASELINE_FILE="$PERF_STATE_DIR/perf-tuning-baseline.env"
export PERF_RMEM_MAX=16777216
export PERF_WMEM_MAX=16777216

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# log()/warn()/die() are normally provided by the sourcing caller
# (install.sh/update.sh); stub them here the same way.
log() { echo "[test-log] $*"; }
warn() { echo "[test-warn] $*" >&2; }
die() { echo "[test-die] $*" >&2; exit 1; }
# shellcheck source=deploy/lib/perf-tuning.sh
. "$LIB_DIR/perf-tuning.sh"

failures=0
assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $desc — expected [$expected], got [$actual]"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}
assert_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -qF "$needle"; then
    echo "ok: $desc"
  else
    echo "FAIL: $desc — expected to find [$needle]"
    failures=$((failures + 1))
  fi
}
assert_not_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -qF "$needle"; then
    echo "FAIL: $desc — did not expect to find [$needle]"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

echo "--- perf_tuning_render ---"

# Case A: kernel does not support BBR at all.
perf_kernel_supports_bbr() { return 1; }
perf_qdisc_available() { return 1; }
out="$(perf_tuning_render)"
assert_contains "no-BBR: rmem_max present" "$out" "net.core.rmem_max = 16777216"
assert_contains "no-BBR: wmem_max present" "$out" "net.core.wmem_max = 16777216"
assert_not_contains "no-BBR: no tcp_congestion_control line" "$out" "tcp_congestion_control"
assert_not_contains "no-BBR: no default_qdisc line" "$out" "default_qdisc"

# Case B: BBR + fq both available.
perf_kernel_supports_bbr() { return 0; }
perf_qdisc_available() { [ "$1" = "fq" ]; }
out="$(perf_tuning_render)"
assert_contains "BBR+fq: tcp_congestion_control = bbr" "$out" "net.ipv4.tcp_congestion_control = bbr"
assert_contains "BBR+fq: default_qdisc = fq" "$out" "net.core.default_qdisc = fq"

# Case C: BBR available, fq unavailable, fq_codel available -> falls back.
perf_kernel_supports_bbr() { return 0; }
perf_qdisc_available() { [ "$1" = "fq_codel" ]; }
out="$(perf_tuning_render)"
assert_contains "BBR+fq_codel fallback: tcp_congestion_control = bbr" "$out" "net.ipv4.tcp_congestion_control = bbr"
assert_contains "BBR+fq_codel fallback: default_qdisc = fq_codel" "$out" "net.core.default_qdisc = fq_codel"

# Case D: BBR available, NEITHER qdisc verified usable -> bbr alone, no
# blind default_qdisc guess.
perf_kernel_supports_bbr() { return 0; }
perf_qdisc_available() { return 1; }
out="$(perf_tuning_render)"
assert_contains "BBR, no verified qdisc: tcp_congestion_control = bbr" "$out" "net.ipv4.tcp_congestion_control = bbr"
assert_not_contains "BBR, no verified qdisc: no default_qdisc line" "$out" "default_qdisc"

echo
echo "--- perf_capture_baseline ---"

perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "212992" ;;
    net.core.wmem_max) echo "212992" ;;
    net.ipv4.tcp_congestion_control) echo "cubic" ;;
    net.core.default_qdisc) echo "pfifo_fast" ;;
  esac
}
perf_capture_baseline
assert_eq "baseline file created" "1" "$( [ -f "$PERF_BASELINE_FILE" ] && echo 1 || echo 0 )"
first_content="$(cat "$PERF_BASELINE_FILE")"
assert_contains "baseline captured original rmem_max" "$first_content" 'BASELINE_RMEM_MAX="212992"'
assert_contains "baseline captured original congestion control" "$first_content" 'BASELINE_TCP_CONGESTION_CONTROL="cubic"'

# Idempotency: even if the live sysctl values change (simulating vpn1
# having applied its OWN tuning since baseline capture), a second call
# must NOT overwrite the already-recorded baseline.
perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "16777216" ;;
    net.core.wmem_max) echo "16777216" ;;
    net.ipv4.tcp_congestion_control) echo "bbr" ;;
    net.core.default_qdisc) echo "fq" ;;
  esac
}
perf_capture_baseline
second_content="$(cat "$PERF_BASELINE_FILE")"
assert_eq "baseline is NOT overwritten on a second call" "$first_content" "$second_content"

echo
echo "--- perf_tuning_apply: effective-value verification / honest reporting ---"

# sysctl --system "succeeds" but the congestion-control value did NOT
# actually take effect (e.g. rejected by the kernel) — apply must NOT
# claim BBR is enabled in that case.
rm -rf "$TMPDIR_TEST/state2"
export PERF_STATE_DIR="$TMPDIR_TEST/state2"
export PERF_BASELINE_FILE="$PERF_STATE_DIR/perf-tuning-baseline.env"
export PERF_SYSCTL_DROPIN="$TMPDIR_TEST/apply-test.conf"
perf_kernel_supports_bbr() { return 0; }
perf_qdisc_available() { [ "$1" = "fq" ]; }
perf_apply_sysctl_system() { return 0; }
perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "16777216" ;;
    net.core.wmem_max) echo "16777216" ;;
    # Deliberately wrong: the file says bbr, but the effective value
    # stayed cubic (simulating a kernel that silently rejected it).
    net.ipv4.tcp_congestion_control) echo "cubic" ;;
    net.core.default_qdisc) echo "pfifo_fast" ;;
  esac
}
apply_out="$(perf_tuning_apply 2>&1)"
assert_not_contains "apply: does not falsely claim bbr enabled when effective value disagrees" \
  "$apply_out" "bbr congestion control enabled and confirmed"
assert_contains "apply: reports the congestion-control mismatch explicitly" \
  "$apply_out" "NOT applied"

# Now the effective value genuinely matches what was requested — apply
# SHOULD report it as confirmed.
perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "16777216" ;;
    net.core.wmem_max) echo "16777216" ;;
    net.ipv4.tcp_congestion_control) echo "bbr" ;;
    net.core.default_qdisc) echo "fq" ;;
  esac
}
apply_out="$(perf_tuning_apply 2>&1)"
assert_contains "apply: confirms bbr enabled when effective value genuinely matches" \
  "$apply_out" "bbr congestion control enabled and confirmed"

echo
echo "--- perf_tuning_rollback ---"

rm -rf "$TMPDIR_TEST/state3"
export PERF_STATE_DIR="$TMPDIR_TEST/state3"
export PERF_BASELINE_FILE="$PERF_STATE_DIR/perf-tuning-baseline.env"
export PERF_SYSCTL_DROPIN="$TMPDIR_TEST/rollback-active.conf"
export PERF_ROLLBACK_DROPIN="$TMPDIR_TEST/rollback-out.conf"

# No baseline recorded yet -> rollback must be a clean no-op failure,
# never a crash or a fabricated "success".
set +e
rollback_out="$(perf_tuning_rollback 2>&1)"
rollback_rc=$?
set -e
assert_eq "rollback with no baseline: non-zero exit" "1" "$rollback_rc"
assert_contains "rollback with no baseline: explains why" "$rollback_out" "Nothing to roll back"

# Record a baseline, then verify rollback writes a drop-in with those
# exact values and (with a stubbed perf_read_sysctl reporting the
# baseline as now-effective) reports success.
perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "212992" ;;
    net.core.wmem_max) echo "212992" ;;
    net.ipv4.tcp_congestion_control) echo "cubic" ;;
    net.core.default_qdisc) echo "pfifo_fast" ;;
  esac
}
perf_capture_baseline
perf_apply_sysctl_system() { return 0; }

rollback_out="$(perf_tuning_rollback 2>&1)"
rollback_rc=$?
assert_eq "rollback with recorded baseline: exit 0" "0" "$rollback_rc"
assert_contains "rollback: rollback drop-in contains baseline rmem_max" \
  "$(cat "$PERF_ROLLBACK_DROPIN")" "net.core.rmem_max = 212992"
assert_contains "rollback: reports rmem_max restored" "$rollback_out" "restored to 212992"
assert_contains "rollback: rollback drop-in contains baseline default_qdisc" \
  "$(cat "$PERF_ROLLBACK_DROPIN")" "net.core.default_qdisc = pfifo_fast"
assert_contains "rollback: reports default_qdisc restored" "$rollback_out" "net.core.default_qdisc restored to pfifo_fast"
assert_eq "rollback: vpn1's active drop-in was removed" "0" "$( [ -f "$PERF_SYSCTL_DROPIN" ] && echo 1 || echo 0 )"

# Restoration verification: if the effective value does NOT actually
# match the baseline after rollback (kernel rejected it, or something
# else overrode it), rollback must report failure, not silently claim
# success.
perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "999999" ;; # wrong on purpose
    net.core.wmem_max) echo "212992" ;;
    net.ipv4.tcp_congestion_control) echo "cubic" ;;
    net.core.default_qdisc) echo "pfifo_fast" ;;
  esac
}
set +e
rollback_out="$(perf_tuning_rollback 2>&1)"
rollback_rc=$?
set -e
assert_eq "rollback: reports failure when effective value does not match baseline" "1" "$rollback_rc"
assert_contains "rollback: explains the mismatch" "$rollback_out" "did not restore"

# qdisc-specific mismatch: rmem/wmem/congestion-control all restore
# correctly, but default_qdisc does not — rollback must still report
# failure, isolated to the qdisc value (not masked by the other three
# happening to match).
perf_read_sysctl() {
  case "$1" in
    net.core.rmem_max) echo "212992" ;;
    net.core.wmem_max) echo "212992" ;;
    net.ipv4.tcp_congestion_control) echo "cubic" ;;
    net.core.default_qdisc) echo "fq_codel" ;; # wrong on purpose
  esac
}
set +e
rollback_out="$(perf_tuning_rollback 2>&1)"
rollback_rc=$?
set -e
assert_eq "rollback: reports failure when only default_qdisc fails to restore" "1" "$rollback_rc"
assert_contains "rollback: explains the qdisc-specific mismatch" "$rollback_out" "net.core.default_qdisc did not restore to pfifo_fast"
assert_not_contains "rollback: does not misreport rmem_max as failing in the qdisc-only mismatch case" \
  "$rollback_out" "net.core.rmem_max did not restore"

echo
if [ "$failures" -eq 0 ]; then
  echo "all perf-tuning tests passed"
  exit 0
else
  echo "$failures test(s) FAILED"
  exit 1
fi
