#!/usr/bin/env bash
# Kernel network tuning for the sing-box data plane (VLESS+REALITY on
# TCP/443, Hysteria2/QUIC on UDP/443). Sourced by
# deploy/almalinux/install.sh and update.sh; expects log()/warn()/die()
# to already be defined by the caller.
#
# Scope is deliberately narrow — only parameters with direct upstream
# evidence for THIS workload (see docs/PERFORMANCE_OPTIMIZATION_PLAN.md
# P0/P1 items), not a generic "internet optimization" sysctl dump:
#
#   - net.core.rmem_max / net.core.wmem_max: raised so a UDP/QUIC socket
#     CAN request a larger buffer via setsockopt (they are ceilings, not
#     the buffer sing-box actually uses). 16 MiB matches the Hysteria2
#     upstream performance guide (https://v2.hysteria.network/docs/advanced/Performance/).
#   - net.ipv4.tcp_congestion_control=bbr + net.core.default_qdisc=fq (or
#     fq_codel): only applied if the running kernel actually supports
#     BBR/the chosen qdisc (verified, not assumed — see
#     perf_kernel_supports_bbr/perf_qdisc_available below) — never forced
#     on a kernel that doesn't support it.
#
# Idempotent: re-running install.sh/update.sh re-applies the same
# /etc/sysctl.d file and re-runs `sysctl --system`, which is a no-op if
# nothing changed.
#
# REVERSIBLE WITHOUT A REBOOT: on the very first apply on a given host,
# perf_capture_baseline() records this host's pre-singbox-vpn sysctl values to
# /var/lib/singbox-vpn/perf-tuning-baseline.env — exactly once, never
# overwritten by a later run (the whole point is "what this host had
# before singbox-vpn touched it", not "what singbox-vpn last set"). Simply deleting
# singbox-vpn's own /etc/sysctl.d drop-in and re-running `sysctl --system` is
# NOT sufficient to actually revert the live values: `sysctl --system`
# only applies settings explicitly listed in some file it processes — a
# parameter no remaining file mentions stays at whatever its current
# live value already is, it does not fall back to a kernel-compiled-in
# default. perf_tuning_rollback() below fixes this by writing an
# explicit rollback drop-in containing the captured baseline values and
# applying it immediately — see that function's doc comment for the
# exact mechanism and its own inherent limits.

# Overridable via environment (like PERF_RMEM_MAX/PERF_WMEM_MAX below)
# purely so deploy/lib/tests/test-perf-tuning.sh can point every write
# this file makes at a throwaway temp directory instead of real system
# paths — production callers (install.sh/update.sh) never set these,
# so they always get the real paths below.
: "${PERF_SYSCTL_DROPIN:=/etc/sysctl.d/99-singbox-vpn-dataplane.conf}"
: "${PERF_ROLLBACK_DROPIN:=/etc/sysctl.d/99-singbox-vpn-dataplane-rollback.conf}"
: "${PERF_STATE_DIR:=/var/lib/singbox-vpn}"
: "${PERF_BASELINE_FILE:=$PERF_STATE_DIR/perf-tuning-baseline.env}"

# UDP core buffer ceilings, bytes. 16 MiB — see file header.
PERF_RMEM_MAX="${PERF_RMEM_MAX:-16777216}"
PERF_WMEM_MAX="${PERF_WMEM_MAX:-16777216}"

perf_read_sysctl() {
  sysctl -n "$1" 2>/dev/null || true
}

# Thin wrapper around `sysctl --system`, split out purely so
# deploy/lib/tests/test-perf-tuning.sh can override it with a no-op —
# the real command re-reads and applies EVERY /etc/sysctl.d file on the
# system, not just singbox-vpn's, which is both unsafe to run for real in CI
# (no root there anyway) and unrelated to what these tests are actually
# checking (this file's own rendering/state logic, not the kernel's
# sysctl subsystem itself).
perf_apply_sysctl_system() {
  sysctl --system
}

# Attempts to make BBR available before checking for it: on several
# supported distro kernels (notably stock Ubuntu/Debian) `tcp_bbr` is
# compiled as a loadable module, not built in, so a kernel that DOES
# support BBR can still be missing it from
# `tcp_available_congestion_control` purely because nothing has loaded
# the module yet. `modprobe tcp_bbr` only registers the congestion
# control algorithm with the kernel — it does not attach it to any
# socket or change any live connection's behavior by itself, so this is
# safe to attempt unconditionally. Never fails the caller: a modprobe
# failure (module doesn't exist, no modules support in this kernel/
# container) just means BBR genuinely isn't available here, which the
# subsequent availability check below correctly reports either way.
perf_kernel_supports_bbr() {
  local avail="/proc/sys/net/ipv4/tcp_available_congestion_control"
  [ -r "$avail" ] || return 1
  grep -qw bbr "$avail" && return 0
  modprobe -q tcp_bbr 2>/dev/null || true
  [ -r "$avail" ] || return 1
  grep -qw bbr "$avail"
}

# Checks whether a qdisc kind is available WITHOUT attaching it to any
# live interface (earlier versions of this function did
# `tc qdisc add dev lo root <kind>` purely to test availability, which
# briefly changes the loopback interface's live qdisc — unnecessary and
# avoidable). Three non-mutating signals, in order of reliability:
#   1. Already active (lsmod) — cheapest, most certain.
#   2. Listed in this kernel's modules.builtin (compiled directly in,
#      never appears in lsmod because it's not a loadable module at all).
#   3. Resolvable as a loadable module via `modprobe --dry-run`, which
#      only checks whether modprobe *could* load it — it does not
#      actually insert the module or touch any interface.
# Usage: perf_qdisc_available fq | perf_qdisc_available fq_codel
perf_qdisc_available() {
  local module="sch_${1}" builtin
  lsmod 2>/dev/null | grep -qw "$module" && return 0
  builtin="/lib/modules/$(uname -r)/modules.builtin"
  [ -r "$builtin" ] && grep -q "/${module}\.ko" "$builtin" && return 0
  command -v modprobe >/dev/null 2>&1 && modprobe --dry-run -q "$module" >/dev/null 2>&1 && return 0
  return 1
}

# Captures this host's pre-singbox-vpn sysctl values EXACTLY ONCE — a no-op on
# every call after the first. Must be called before the first
# perf_tuning_apply ever writes anything, or the "baseline" it records
# would just be singbox-vpn's own prior tuning instead of the host's true
# original state (the one known, inherent limitation of this scheme: it
# cannot recover a baseline retroactively on a host that already ran an
# older version of this script before this capture step existed — see
# the warning below for that case).
perf_capture_baseline() {
  if [ -f "$PERF_BASELINE_FILE" ]; then
    return 0
  fi
  if [ -f "$PERF_SYSCTL_DROPIN" ]; then
    warn "singbox-vpn's sysctl tuning ($PERF_SYSCTL_DROPIN) already exists but no baseline was ever recorded — this host likely ran an older singbox-vpn version before baseline capture existed. Recording CURRENT live values as the baseline; if they already include singbox-vpn's own prior tuning (rather than this host's true pre-singbox-vpn defaults), rollback will restore singbox-vpn's tuning, not the original distro defaults. If you know the true original values, set them by hand in $PERF_BASELINE_FILE before the next rollback."
  fi
  install -d -m 0755 "$PERF_STATE_DIR" 2>/dev/null || true
  local rmem wmem cc qdisc
  rmem="$(perf_read_sysctl net.core.rmem_max)"
  wmem="$(perf_read_sysctl net.core.wmem_max)"
  cc="$(perf_read_sysctl net.ipv4.tcp_congestion_control)"
  qdisc="$(perf_read_sysctl net.core.default_qdisc)"
  cat > "$PERF_BASELINE_FILE.tmp" <<EOF
# Captured once by deploy/lib/perf-tuning.sh, on the first apply on this
# host — the kernel network values this host had BEFORE singbox-vpn ever
# changed them. Used by perf_tuning_rollback() to restore them without
# requiring a reboot. Do NOT hand-edit unless you are deliberately
# correcting a bad baseline (see perf_capture_baseline's doc comment);
# do NOT delete this file unless you accept that a future rollback will
# fall back to whatever this host's OTHER sysctl.d files (or kernel
# compiled-in defaults) provide instead of these exact values.
BASELINE_RMEM_MAX="$rmem"
BASELINE_WMEM_MAX="$wmem"
BASELINE_TCP_CONGESTION_CONTROL="$cc"
BASELINE_DEFAULT_QDISC="$qdisc"
EOF
  mv -f "$PERF_BASELINE_FILE.tmp" "$PERF_BASELINE_FILE"
  chmod 0644 "$PERF_BASELINE_FILE"
  log "recorded pre-singbox-vpn kernel network baseline at $PERF_BASELINE_FILE"
}

# Renders the sysctl drop-in content. Split out from perf_tuning_apply
# so tests can call it directly without root/sysctl access.
perf_tuning_render() {
  local congestion_lines=""
  if perf_kernel_supports_bbr; then
    local qdisc=""
    if perf_qdisc_available fq; then
      qdisc="fq"
    elif perf_qdisc_available fq_codel; then
      qdisc="fq_codel"
    fi
    if [ -n "$qdisc" ]; then
      congestion_lines="net.ipv4.tcp_congestion_control = bbr
net.core.default_qdisc = ${qdisc}"
    else
      # BBR itself is available, but neither of its usable qdisc
      # pairings could be verified on this kernel — set BBR alone
      # rather than guessing a qdisc that might not actually be usable.
      congestion_lines="net.ipv4.tcp_congestion_control = bbr"
    fi
  fi
  cat <<EOF
# Managed by singbox-vpn (deploy/lib/perf-tuning.sh) — do not hand-edit. To
# revert to this host's pre-singbox-vpn values without a reboot, run
# 'perf_tuning_rollback' (see this file), not just 'rm' + 'sysctl --system'
# — see the file-header comment for why the latter is not sufficient.
#
# UDP socket buffer ceilings for Hysteria2/QUIC (upstream Hysteria2
# performance guide: https://v2.hysteria.network/docs/advanced/Performance/).
# These raise the maximum a socket may request; they do not themselves
# force any socket to use more memory.
net.core.rmem_max = ${PERF_RMEM_MAX}
net.core.wmem_max = ${PERF_WMEM_MAX}
${congestion_lines}
EOF
}

# Applies the tuning. Never fails the install/update run on a
# non-fatal tuning problem (e.g. sysctl binary missing in a container) —
# this is an optimization, not a correctness requirement, so a failure
# here is a warning, never a `die`. After writing the drop-in, VERIFIES
# the effective live values (via `sysctl -n`, i.e. actually reading
# /proc/sys) rather than assuming `sysctl --system` succeeded just
# because it exited 0 — a value can be rejected by the kernel (e.g. an
# unrecognized congestion-control name) while `sysctl --system` still
# reports overall success for the file. Every claim printed below is
# gated on that verification: "BBR enabled" is only ever printed when
# the effective `net.ipv4.tcp_congestion_control` actually reads back as
# `bbr`, never merely because this script wrote that line to a file.
perf_tuning_apply() {
  if ! command -v sysctl >/dev/null 2>&1; then
    warn "sysctl not found; skipping kernel network tuning."
    return 0
  fi
  perf_capture_baseline

  local rendered
  rendered="$(perf_tuning_render)"
  if [ -f "$PERF_SYSCTL_DROPIN" ] && [ "$(cat "$PERF_SYSCTL_DROPIN")" = "$rendered" ]; then
    log "kernel network tuning already up to date ($PERF_SYSCTL_DROPIN)."
  else
    printf '%s\n' "$rendered" > "$PERF_SYSCTL_DROPIN.tmp"
    mv -f "$PERF_SYSCTL_DROPIN.tmp" "$PERF_SYSCTL_DROPIN"
    chmod 0644 "$PERF_SYSCTL_DROPIN"
    log "wrote $PERF_SYSCTL_DROPIN"
  fi
  if ! perf_apply_sysctl_system >/tmp/singbox-vpn-sysctl-system.out 2>&1; then
    warn "sysctl --system reported errors — see /tmp/singbox-vpn-sysctl-system.out. Continuing; this does not block installation."
  fi

  local wanted_cc=""
  if printf '%s' "$rendered" | grep -q '^net.ipv4.tcp_congestion_control = '; then
    wanted_cc="$(printf '%s' "$rendered" | sed -n 's/^net.ipv4.tcp_congestion_control = //p')"
  fi
  local effective_rmem effective_wmem effective_cc
  effective_rmem="$(perf_read_sysctl net.core.rmem_max)"
  effective_wmem="$(perf_read_sysctl net.core.wmem_max)"
  effective_cc="$(perf_read_sysctl net.ipv4.tcp_congestion_control)"

  # Report exactly what did and did not apply — never a blanket
  # "success" that papers over one rejected value in an otherwise-fine
  # file.
  if [ "$effective_rmem" = "$PERF_RMEM_MAX" ]; then
    log "net.core.rmem_max effective value confirmed: $effective_rmem"
  else
    warn "net.core.rmem_max NOT applied as intended: wanted $PERF_RMEM_MAX, effective value is '$effective_rmem'"
  fi
  if [ "$effective_wmem" = "$PERF_WMEM_MAX" ]; then
    log "net.core.wmem_max effective value confirmed: $effective_wmem"
  else
    warn "net.core.wmem_max NOT applied as intended: wanted $PERF_WMEM_MAX, effective value is '$effective_wmem'"
  fi
  if [ -n "$wanted_cc" ]; then
    if [ "$effective_cc" = "$wanted_cc" ]; then
      log "TCP $wanted_cc congestion control enabled and confirmed active for the REALITY/TCP data plane."
    else
      warn "TCP $wanted_cc congestion control was requested but the effective congestion controller is '$effective_cc', not '$wanted_cc' — NOT applied. Leaving the host's existing congestion control in place."
    fi
  else
    log "kernel does not report BBR support (even after attempting 'modprobe tcp_bbr') — leaving tcp_congestion_control at its distro default. This is expected on older/minimal kernels; not an error."
  fi
}

# Restores this host's pre-singbox-vpn kernel network values, without a reboot.
# Requires perf_capture_baseline to have run at least once (i.e.
# perf_tuning_apply must have run before); if no baseline was ever
# recorded, there is nothing to roll back TO and this is a no-op, not a
# guess.
#
# Mechanism: writes an explicit rollback drop-in
# ($PERF_ROLLBACK_DROPIN) containing the captured baseline values and
# applies it immediately via `sysctl --system`. This is deliberately NOT
# just "delete singbox-vpn's drop-in and run sysctl --system": `sysctl --system`
# only applies values some file explicitly lists — a parameter no
# remaining file mentions is left at whatever its CURRENT live value
# already is, it is never reset to a kernel-compiled-in default by
# omission. Without an explicit rollback file re-asserting the original
# numbers, deleting singbox-vpn's drop-in alone would leave the live kernel
# still running singbox-vpn's values indefinitely (until the next reboot, if
# even then). $PERF_ROLLBACK_DROPIN is left in place afterward (not a
# transient file) so a later unrelated `sysctl --system` run continues
# to reassert the restored baseline rather than silently drifting back
# toward whatever singbox-vpn's now-deleted file used to say.
perf_tuning_rollback() {
  if ! [ -f "$PERF_BASELINE_FILE" ]; then
    warn "no perf-tuning baseline recorded at $PERF_BASELINE_FILE — either singbox-vpn's kernel tuning was never applied on this host, or the baseline file was deleted. Nothing to roll back."
    return 1
  fi
  # shellcheck disable=SC1090
  . "$PERF_BASELINE_FILE"

  rm -f "$PERF_SYSCTL_DROPIN"

  {
    echo "# Written by deploy/lib/perf-tuning.sh's perf_tuning_rollback —"
    echo "# restores this host's captured pre-singbox-vpn kernel network values"
    echo "# (from $PERF_BASELINE_FILE). Safe to delete once you've confirmed"
    echo "# the values you want are in effect; deleting it does not itself"
    echo "# change anything further, it just stops re-asserting these numbers"
    echo "# on a future 'sysctl --system' run."
    [ -n "${BASELINE_RMEM_MAX:-}" ] && echo "net.core.rmem_max = $BASELINE_RMEM_MAX"
    [ -n "${BASELINE_WMEM_MAX:-}" ] && echo "net.core.wmem_max = $BASELINE_WMEM_MAX"
    [ -n "${BASELINE_TCP_CONGESTION_CONTROL:-}" ] && echo "net.ipv4.tcp_congestion_control = $BASELINE_TCP_CONGESTION_CONTROL"
    [ -n "${BASELINE_DEFAULT_QDISC:-}" ] && echo "net.core.default_qdisc = $BASELINE_DEFAULT_QDISC"
  } > "$PERF_ROLLBACK_DROPIN.tmp"
  mv -f "$PERF_ROLLBACK_DROPIN.tmp" "$PERF_ROLLBACK_DROPIN"
  chmod 0644 "$PERF_ROLLBACK_DROPIN"

  if ! perf_apply_sysctl_system >/tmp/singbox-vpn-sysctl-rollback.out 2>&1; then
    warn "sysctl --system reported errors during rollback — see /tmp/singbox-vpn-sysctl-rollback.out"
  fi

  local failed=0
  if [ -n "${BASELINE_RMEM_MAX:-}" ]; then
    [ "$(perf_read_sysctl net.core.rmem_max)" = "$BASELINE_RMEM_MAX" ] \
      && log "net.core.rmem_max restored to $BASELINE_RMEM_MAX" \
      || { warn "net.core.rmem_max did not restore to $BASELINE_RMEM_MAX"; failed=1; }
  fi
  if [ -n "${BASELINE_WMEM_MAX:-}" ]; then
    [ "$(perf_read_sysctl net.core.wmem_max)" = "$BASELINE_WMEM_MAX" ] \
      && log "net.core.wmem_max restored to $BASELINE_WMEM_MAX" \
      || { warn "net.core.wmem_max did not restore to $BASELINE_WMEM_MAX"; failed=1; }
  fi
  if [ -n "${BASELINE_TCP_CONGESTION_CONTROL:-}" ]; then
    [ "$(perf_read_sysctl net.ipv4.tcp_congestion_control)" = "$BASELINE_TCP_CONGESTION_CONTROL" ] \
      && log "net.ipv4.tcp_congestion_control restored to $BASELINE_TCP_CONGESTION_CONTROL" \
      || { warn "net.ipv4.tcp_congestion_control did not restore to $BASELINE_TCP_CONGESTION_CONTROL"; failed=1; }
  fi
  if [ -n "${BASELINE_DEFAULT_QDISC:-}" ]; then
    [ "$(perf_read_sysctl net.core.default_qdisc)" = "$BASELINE_DEFAULT_QDISC" ] \
      && log "net.core.default_qdisc restored to $BASELINE_DEFAULT_QDISC" \
      || { warn "net.core.default_qdisc did not restore to $BASELINE_DEFAULT_QDISC"; failed=1; }
  fi
  return "$failed"
}

# Direct-execution entry point: `sudo deploy/lib/perf-tuning.sh rollback`
# (or `... apply`) on the actual VPS, so rollback is a real command an
# operator can run, not just documentation to hand-translate into a
# bash one-liner. Skipped when this file is `source`d (the normal
# install.sh/update.sh path) — `${BASH_SOURCE[0]}" = "$0"` is only true
# when this file itself is the script being executed.
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  log() { echo "[perf-tuning] $*"; }
  warn() { echo "[perf-tuning] WARNING: $*" >&2; }
  die() { echo "[perf-tuning] ERROR: $*" >&2; exit 1; }
  case "${1:-}" in
    apply)
      perf_tuning_apply
      ;;
    rollback)
      perf_tuning_rollback
      ;;
    *)
      echo "Usage: $0 {apply|rollback}" >&2
      echo "  apply    — (re)apply singbox-vpn's kernel network tuning (same as install.sh/update.sh)." >&2
      echo "  rollback — restore this host's pre-singbox-vpn kernel network values, without a reboot." >&2
      exit 1
      ;;
  esac
fi
