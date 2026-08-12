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
#   - net.ipv4.tcp_congestion_control=bbr + net.core.default_qdisc=fq:
#     only applied if the running kernel actually reports "bbr" in
#     tcp_available_congestion_control — never forced on a kernel that
#     doesn't support it, and the fq qdisc is BBR's documented pairing
#     (BBR paces writes itself; without fq/fq_codel a FIFO qdisc's
#     default drop behavior fights BBR's pacing).
#
# Idempotent: re-running install.sh/update.sh re-applies the same
# /etc/sysctl.d file and re-runs `sysctl --system`, which is a no-op if
# nothing changed. Reversible: delete the drop-in file and re-run
# `sysctl --system` (see docs/PERFORMANCE_OPTIMIZATION_PLAN.md rollback
# section) — nothing here edits /etc/sysctl.conf directly.

PERF_SYSCTL_DROPIN="/etc/sysctl.d/99-vpn1-dataplane.conf"

# UDP core buffer ceilings, bytes. 16 MiB — see file header.
PERF_RMEM_MAX="${PERF_RMEM_MAX:-16777216}"
PERF_WMEM_MAX="${PERF_WMEM_MAX:-16777216}"

perf_kernel_supports_bbr() {
  local avail="/proc/sys/net/ipv4/tcp_available_congestion_control"
  [ -r "$avail" ] || return 1
  grep -qw bbr "$avail"
}

perf_kernel_supports_fq() {
  # `tc qdisc add ... fq` requires CONFIG_NET_SCH_FQ; module may need
  # loading on kernels where it's built as a module rather than builtin.
  modprobe -q sch_fq 2>/dev/null || true
  tc qdisc add dev lo root fq 2>/dev/null && { tc qdisc del dev lo root 2>/dev/null || true; return 0; }
  return 1
}

# Renders the sysctl drop-in content. Split out from perf_tuning_apply
# so tests can call it directly without root/sysctl access.
perf_tuning_render() {
  local congestion_lines=""
  if perf_kernel_supports_bbr; then
    local qdisc="fq"
    perf_kernel_supports_fq || qdisc="fq_codel"
    congestion_lines="net.ipv4.tcp_congestion_control = bbr
net.core.default_qdisc = ${qdisc}"
  fi
  cat <<EOF
# Managed by vpn1 (deploy/lib/perf-tuning.sh) — safe to delete this file
# and run 'sysctl --system' to revert to distro defaults.
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
# here is a warning, never a `die`.
perf_tuning_apply() {
  if ! command -v sysctl >/dev/null 2>&1; then
    warn "sysctl not found; skipping kernel network tuning."
    return 0
  fi
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
  if sysctl --system >/tmp/vpn1-sysctl-system.out 2>&1; then
    log "applied kernel network tuning (sysctl --system)."
  else
    warn "sysctl --system reported errors — see /tmp/vpn1-sysctl-system.out. Continuing; this does not block installation."
  fi
  if perf_kernel_supports_bbr; then
    log "TCP BBR congestion control enabled for the REALITY/TCP data plane (kernel supports it)."
  else
    log "kernel does not report BBR support (tcp_available_congestion_control) — leaving tcp_congestion_control at its distro default. This is expected on older kernels; not an error."
  fi
}
