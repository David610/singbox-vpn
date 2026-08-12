#!/usr/bin/env bash
# vpn1 performance benchmark: a repeatable, multi-layer measurement of
# where time/bytes actually go, so a performance decision can be made
# from evidence instead of guessing. See
# docs/PERFORMANCE_OPTIMIZATION_PLAN.md for how to read this output.
#
# Layers measured (each independently, so a slow number can be
# attributed to the right one):
#   1. Host: CPU/RAM/steal/load — see also `vpn doctor --performance`.
#   2. Network path: packet loss, RTT, jitter, MTU/PMTU to a chosen
#      target — independent of anything VPN-specific.
#   3. Raw VPS throughput: a plain HTTPS download FROM this host to a
#      public CDN, no tunnel involved — isolates "this VPS's own
#      uplink/provider" from "the tunnel software".
#   4. VLESS+REALITY / Hysteria2 throughput: a REAL sing-box client
#      dials this server's OWN public listener (over the real network,
#      not loopback-only) through a throwaway benchmark user created
#      and deleted by this script, while sing-box's own CPU usage is
#      sampled — isolates "the tunnel" from "the raw uplink" (layer 3).
#
# NOT authoritative: a single public CDN's speed is not a ceiling on
# what your ISP path can do, and one run is noise — this script always
# runs multiple samples (--runs, default 3) and reports the spread, not
# just an average. Never trust a single number from this or any speed
# test.
#
# Every step that needs a tool or a live server component this host
# doesn't have prints "SKIPPED: <reason>" and continues — it never
# fabricates a number and never aborts the whole report over one
# unavailable layer.
set -Eeuo pipefail

CONFIG="/etc/vpn/deployment.toml"
RUNS=3
TARGET_HOST="1.1.1.1"
DOWNLOAD_URL="https://speed.cloudflare.com/__down?bytes=25000000"
VPN_BIN="$(command -v vpn || echo /usr/local/bin/vpn)"
SINGBOX_BIN="$(command -v sing-box || echo /usr/local/bin/sing-box)"
SKIP_TUNNEL=0

usage() {
  cat <<EOF
Usage: $(basename "$0") [options]
  --config PATH        deployment.toml (default: $CONFIG)
  --runs N              samples per measurement (default: $RUNS)
  --target-host HOST    ping/mtr/MTU probe target (default: $TARGET_HOST) —
                         pass a host on your actual client-side ISP path
                         (e.g. a box you control in Russia) for a
                         meaningful loss/RTT/jitter reading; the default
                         only characterizes this VPS's own uplink.
  --download-url URL    raw-throughput test file (default: Cloudflare's
                         speed-test endpoint)
  --skip-tunnel          skip the VLESS/Hysteria2 layer (no throwaway
                         user is created; useful on a non-production host)
  -h, --help             this text
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --config) CONFIG="$2"; shift 2 ;;
    --runs) RUNS="$2"; shift 2 ;;
    --target-host) TARGET_HOST="$2"; shift 2 ;;
    --download-url) DOWNLOAD_URL="$2"; shift 2 ;;
    --skip-tunnel) SKIP_TUNNEL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

section() { echo; echo "$1"; printf '%s\n' "${1//?/-}"; }
kv() { printf '%-28s %s\n' "$1:" "$2"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Runs $1 a total of $RUNS times, printing "min / median / max" of the
# numeric values $2 extracts (a shell function name receiving nothing,
# printing one number to stdout, or nothing on failure). Never averages
# across a mix of successes and failures — a failed sample is dropped,
# not counted as zero.
sample_min_median_max() {
  local extractor="$1"
  local -a values=()
  local i
  for ((i = 0; i < RUNS; i++)); do
    local v
    v="$("$extractor" 2>/dev/null || true)"
    [ -n "$v" ] && values+=("$v")
  done
  if [ "${#values[@]}" -eq 0 ]; then
    echo "unavailable"
    return
  fi
  local sorted
  sorted="$(printf '%s\n' "${values[@]}" | sort -n)"
  local n=${#values[@]}
  local min max median
  min="$(echo "$sorted" | head -1)"
  max="$(echo "$sorted" | tail -1)"
  median="$(echo "$sorted" | awk -v n="$n" 'NR==int((n+1)/2){print; exit}')"
  echo "min=$min median=$median max=$max (n=$n)"
}

# ---------------------------------------------------------------------
# 1. Host
# ---------------------------------------------------------------------
section "Host"
if [ -r /proc/cpuinfo ]; then
  kv "CPU model" "$(awk -F: '/model name/{print $2; exit}' /proc/cpuinfo | sed 's/^ *//')"
fi
kv "vCPUs" "$(nproc 2>/dev/null || echo unavailable)"
if [ -r /proc/loadavg ]; then
  kv "load average" "$(awk '{print $1, $2, $3}' /proc/loadavg)"
fi
if [ -r /proc/meminfo ]; then
  kv "RAM total" "$(awk '/MemTotal/{print $2, $3}' /proc/meminfo)"
  kv "swap total" "$(awk '/SwapTotal/{print $2, $3}' /proc/meminfo)"
fi
if [ -r /proc/stat ]; then
  read -r _ u1 n1 s1 i1 io1 irq1 sirq1 st1 _ < <(awk '/^cpu /{print; exit}' /proc/stat)
  sleep 0.3
  read -r _ u2 n2 s2 i2 io2 irq2 sirq2 st2 _ < <(awk '/^cpu /{print; exit}' /proc/stat)
  total1=$((u1+n1+s1+i1+io1+irq1+sirq1+st1))
  total2=$((u2+n2+s2+i2+io2+irq2+sirq2+st2))
  dt=$((total2-total1))
  if [ "$dt" -gt 0 ]; then
    steal_pct=$(( (st2-st1) * 100 / dt ))
    kv "CPU steal (instantaneous)" "${steal_pct}%"
  fi
fi
NIC="$(ip -o route get "$TARGET_HOST" 2>/dev/null | awk '{for(i=1;i<=NF;i++) if ($i=="dev") print $(i+1)}' | head -1)" || NIC=""
if [ -n "${NIC:-}" ]; then
  kv "primary interface" "$NIC"
else
  kv "primary interface" "unavailable (ip not found or unreachable)"
fi

# ---------------------------------------------------------------------
# 2. Network path (to --target-host)
# ---------------------------------------------------------------------
section "Network path (target: $TARGET_HOST)"
if have ping; then
  PING_OUT="$(timeout 15 ping -c 20 -i 0.2 -W 2 -q "$TARGET_HOST" 2>/dev/null || true)"
  if [ -n "$PING_OUT" ]; then
    LOSS="$(echo "$PING_OUT" | awk -F, '/packet loss/{for(i=1;i<=NF;i++) if ($i ~ /loss/) print $i}' | sed 's/^ *//')"
    RTT_LINE="$(echo "$PING_OUT" | awk -F= '/rtt|round-trip/{print $2}')"
    kv "packet loss" "${LOSS:-unavailable}"
    kv "RTT min/avg/max/mdev (ms)" "${RTT_LINE:-unavailable}"
  else
    kv "ping" "SKIPPED: target unreachable or ping blocked"
  fi
else
  kv "ping" "SKIPPED: ping not installed"
fi
if have mtr; then
  echo "  mtr summary (10 cycles):"
  timeout 30 mtr -r -c 10 -n "$TARGET_HOST" 2>/dev/null | sed 's/^/    /' || echo "    SKIPPED: mtr failed or timed out"
else
  kv "mtr" "SKIPPED: mtr not installed (optional; install for hop-by-hop loss)"
fi

# ---------------------------------------------------------------------
# MTU / PMTU
# ---------------------------------------------------------------------
section "MTU / PMTU (target: $TARGET_HOST)"
if have ping; then
  mtu_probe() {
    local size=$1
    ping -M "do" -c 1 -W 1 -s "$size" "$TARGET_HOST" >/dev/null 2>&1
  }
  # Binary search the largest non-fragmenting ICMP payload between 1000
  # and 1472 (1472 + 28 bytes of IP/ICMP header = 1500, standard
  # Ethernet MTU). A ceiling below ~1400 here on a path that should be
  # 1500 is a real PMTU-related symptom, not a guess.
  lo=1000; hi=1472; best=0
  while [ "$lo" -le "$hi" ]; do
    mid=$(((lo + hi) / 2))
    if mtu_probe "$mid"; then
      best=$mid
      lo=$((mid + 1))
    else
      hi=$((mid - 1))
    fi
  done
  if [ "$best" -gt 0 ]; then
    kv "largest non-fragmenting ICMP payload" "${best} bytes (path MTU ~ $((best + 28)) bytes)"
    [ "$((best + 28))" -lt 1450 ] && echo "  NOTE: path MTU is below the standard 1500 — this can cause QUIC/Hysteria2 fragmentation symptoms (stalls, poor throughput) independent of anything sing-box-side."
  else
    kv "MTU probe" "unavailable (target may block ICMP-with-DF; not conclusive)"
  fi
else
  kv "MTU probe" "SKIPPED: ping not installed"
fi

# ---------------------------------------------------------------------
# 3. Raw VPS throughput (no tunnel)
# ---------------------------------------------------------------------
section "Raw VPS throughput (this host -> Internet, no tunnel)"
if have curl; then
  raw_download_mbps() {
    local out
    out="$(curl -o /dev/null -s -w '%{speed_download}' --max-time 30 "$DOWNLOAD_URL" 2>/dev/null)" || return 1
    awk -v b="$out" 'BEGIN{printf "%.2f", b*8/1000000}'
  }
  echo "  download (Mbps), $RUNS run(s) against $DOWNLOAD_URL:"
  echo "    $(sample_min_median_max raw_download_mbps)"
  raw_latency_ms() {
    curl -o /dev/null -s -w '%{time_connect}' --max-time 10 "$DOWNLOAD_URL" 2>/dev/null \
      | awk '{printf "%.1f", $1*1000}'
  }
  echo "  TCP connect latency (ms), $RUNS run(s):"
  echo "    $(sample_min_median_max raw_latency_ms)"
  echo "  NOTE: no public upload-accepting endpoint is assumed to exist; run"
  echo "  'iperf3 -s' on a second host you control and 'iperf3 -c <host>' here"
  echo "  for a real symmetric throughput number if upload matters to you."
else
  kv "raw throughput" "SKIPPED: curl not installed"
fi

# ---------------------------------------------------------------------
# 4. VLESS+REALITY / Hysteria2 throughput (real tunnel, real network)
# ---------------------------------------------------------------------
tunnel_benchmark() {
  local proto_tag="$1" out_tag="$2" label="$3"
  section "$label throughput (through a real sing-box client, over the real network)"
  if [ "$SKIP_TUNNEL" -eq 1 ]; then
    kv "$label" "SKIPPED: --skip-tunnel"
    return
  fi
  if ! [ -x "$SINGBOX_BIN" ]; then
    kv "$label" "SKIPPED: sing-box binary not found at $SINGBOX_BIN"
    return
  fi
  if ! [ -x "$VPN_BIN" ]; then
    kv "$label" "SKIPPED: vpn-admin binary not found"
    return
  fi
  if ! have jq; then
    kv "$label" "SKIPPED: jq not installed"
    return
  fi

  local bench_name="vpn-benchmark-$$"
  local created=0
  local tmpdir
  tmpdir="$(mktemp -d)"
  chmod 0700 "$tmpdir"
  cleanup() {
    if [ "$created" -eq 1 ]; then
      "$VPN_BIN" --config "$CONFIG" user remove "$bench_user_id" >/dev/null 2>&1 || true
    fi
    if [ -n "${SINGBOX_CLIENT_PID:-}" ]; then
      kill "$SINGBOX_CLIENT_PID" >/dev/null 2>&1 || true
    fi
    rm -rf "$tmpdir"
  }
  trap cleanup RETURN

  local create_json
  if ! create_json="$("$VPN_BIN" --config "$CONFIG" user create --name "$bench_name" --json 2>/dev/null)"; then
    kv "$label" "SKIPPED: could not create a throwaway benchmark user (need root / valid deployment)"
    return
  fi
  created=1
  bench_user_id="$(echo "$create_json" | jq -r .id)"
  local sub_url token subscription_backend_port
  sub_url="$(echo "$create_json" | jq -r .subscription_url)"
  token="${sub_url##*/}"
  subscription_backend_port="$(awk -F'=' '/^\[subscription\]/{f=1;next} f && /listen_port/{gsub(/[^0-9]/,"",$2); print $2; exit}' "$CONFIG")"
  [ -n "$subscription_backend_port" ] || subscription_backend_port=9100

  local sub_json
  if ! sub_json="$(curl -fsS --max-time 10 "http://127.0.0.1:${subscription_backend_port}/sub/${token}?format=singbox" 2>/dev/null)"; then
    kv "$label" "SKIPPED: could not reach the local subscription backend"
    return
  fi

  local local_socks_port=18080
  local client_cfg="$tmpdir/client.json"
  # Reuse the exact outbound the real subscription serves (so this
  # measures what a real client actually gets, not a hand-tuned test
  # config), pointed at through a local-only SOCKS inbound, routed to
  # ONLY the requested transport's outbound tag.
  jq --arg tag "$out_tag" --argjson port "$local_socks_port" \
    '{ log: {level:"warn"}, inbounds: [{type:"mixed",tag:"in",listen:"127.0.0.1",listen_port:$port}], outbounds: .outbounds, route: {final: $tag} }' \
    <<<"$sub_json" > "$client_cfg" 2>/dev/null || {
      kv "$label" "SKIPPED: no $proto_tag outbound in this deployment's subscription"
      return
    }

  "$SINGBOX_BIN" run -c "$client_cfg" >"$tmpdir/client.log" 2>&1 &
  SINGBOX_CLIENT_PID=$!
  local waited=0
  while ! (exec 3<>"/dev/tcp/127.0.0.1/$local_socks_port") 2>/dev/null; do
    exec 3>&- 2>/dev/null || true
    sleep 0.2
    waited=$((waited + 1))
    if [ "$waited" -gt 25 ]; then
      kv "$label" "SKIPPED: sing-box client did not come up (see $tmpdir/client.log if it still exists)"
      return
    fi
  done
  exec 3>&- 2>/dev/null || true

  tunnel_download_mbps() {
    local out
    out="$(curl -o /dev/null -s -w '%{speed_download}' --max-time 30 \
      --socks5-hostname "127.0.0.1:$local_socks_port" "$DOWNLOAD_URL" 2>/dev/null)" || return 1
    awk -v b="$out" 'BEGIN{printf "%.2f", b*8/1000000}'
  }
  echo "  throughput (Mbps), $RUNS run(s):"
  echo "    $(sample_min_median_max tunnel_download_mbps)"

  local sb_pid
  sb_pid="$(pgrep -f "sing-box run -c $client_cfg" | head -1 || true)"
  if [ -n "$sb_pid" ] && [ -r "/proc/$sb_pid/stat" ]; then
    local t1 t2
    t1="$(awk '{print $14+$15}' "/proc/$sb_pid/stat")"
    tunnel_download_mbps >/dev/null 2>&1 || true
    t2="$(awk '{print $14+$15}' "/proc/$sb_pid/stat")"
    kv "sing-box client CPU ticks during one transfer" "$((t2 - t1))"
  fi
}

tunnel_benchmark "vless" "vless-reality-out" "VLESS+REALITY"
tunnel_benchmark "hysteria2" "hysteria2-out" "Hysteria2"

section "Assessment"
cat <<'EOF'
This script reports MEASUREMENTS only. It does not compute a verdict —
compare the numbers above against docs/PERFORMANCE_OPTIMIZATION_PLAN.md's
decision guide:
  - Raw VPS download far below your provider's advertised rate, with high
    CPU steal -> likely a noisy-neighbor/oversubscribed host, not
    something sysctl/sing-box tuning can fix.
  - Raw VPS throughput fine, but VLESS/Hysteria2 throughput much lower
    with low sing-box CPU -> likely network path (loss/PMTU/congestion
    control), not CPU-bound.
  - VLESS/Hysteria2 throughput much lower AND sing-box CPU pinned near
    100% of one core -> likely CPU-bound userspace crypto/QUIC processing
    -> consider more/better vCPUs (see docs/PERFORMANCE_OPTIMIZATION_PLAN.md).
  - High packet loss or RTT/jitter to --target-host, independent of the
    VPN entirely -> likely the network path itself (peering/routing),
    which no server-side tuning here can fix.
Re-run with --target-host set to a real vantage point on your users' ISP
path (not this script's default) for a meaningful network-path reading.
EOF
