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
#      target — independent of anything VPN-specific. THIS is the layer
#      that characterizes a real client's network path — pass
#      --target-host pointed at (or run this whole script from) an
#      actual vantage point on that path for a meaningful reading.
#   3. Raw VPS throughput: a plain HTTPS download FROM this host to a
#      public CDN, no tunnel involved — isolates "this VPS's own
#      uplink/provider" from "the tunnel software".
#   4. VLESS+REALITY / Hysteria2 SERVER-SIDE PROTOCOL OVERHEAD: a REAL
#      sing-box client, running on THIS SAME VPS, dials THIS SAME VPS's
#      own public listener through a throwaway benchmark user created
#      and deleted by this script, while sing-box's own CPU usage is
#      sampled. This isolates "the tunnel protocol/crypto/QUIC stack's
#      own overhead" from "the raw uplink" (layer 3) — it does NOT
#      measure a real remote client's network path (see layer 2), because
#      the traffic here never leaves this host's own uplink/routing. A
#      real Russia-side measurement requires either layer 2 run from that
#      vantage point, or a sing-box client run on a real remote host on
#      that path against this VPS — this script cannot fake that from
#      the server side alone.
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
#
# --json emits a flat machine-readable object instead of the prose
# report (built via kv()/section()/sample_min_median_max() side
# effects, so JSON and text output can never structurally drift apart —
# whatever the text report measures is exactly what the JSON contains).
# --compare A.json B.json diffs two prior --json outputs (jq only, no
# new runtime dependency) and exits before taking any measurement.
set -Eeuo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/lib/vpn-benchmark-lib.sh
. "$LIB_DIR/vpn-benchmark-lib.sh"

CONFIG="/etc/vpn/deployment.toml"
RUNS=3
TARGET_HOST="1.1.1.1"
DOWNLOAD_URL="https://speed.cloudflare.com/__down?bytes=25000000"
VPN_BIN="$(command -v vpn || echo /usr/local/bin/vpn)"
SINGBOX_BIN="$(command -v sing-box || echo /usr/local/bin/sing-box)"
SKIP_TUNNEL=0
JSON_MODE=0
OUTPUT=""
QUICK=0
RUNS_EXPLICIT=0
DOWNLOAD_URL_EXPLICIT=0
COMPARE_A=""
COMPARE_B=""
PING_COUNT=20
MTR_CYCLES=10

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
  --quick                fast, bandwidth-light run: 1 sample per
                         measurement and a small (~2 MB) download unless
                         --runs/--download-url are also given explicitly
                         (those always win over --quick's defaults),
                         plus fewer ping/mtr probes
  --json                 emit a single machine-readable JSON object
                         instead of the prose report
  --output PATH          also write the report to PATH (in addition to
                         stdout)
  --compare A.json B.json
                         compare two prior --json outputs (numeric keys
                         get a delta) and exit — takes no measurements
  -h, --help             this text
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --config) CONFIG="$2"; shift 2 ;;
    --runs) RUNS="$2"; RUNS_EXPLICIT=1; shift 2 ;;
    --target-host) TARGET_HOST="$2"; shift 2 ;;
    --download-url) DOWNLOAD_URL="$2"; DOWNLOAD_URL_EXPLICIT=1; shift 2 ;;
    --skip-tunnel) SKIP_TUNNEL=1; shift ;;
    --quick) QUICK=1; shift ;;
    --json) JSON_MODE=1; shift ;;
    --output) OUTPUT="$2"; shift 2 ;;
    --compare) COMPARE_A="$2"; COMPARE_B="$3"; shift 3 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

have() { command -v "$1" >/dev/null 2>&1; }

if [ "$QUICK" -eq 1 ]; then
  [ "$RUNS_EXPLICIT" -eq 1 ] || RUNS=1
  [ "$DOWNLOAD_URL_EXPLICIT" -eq 1 ] || DOWNLOAD_URL="https://speed.cloudflare.com/__down?bytes=2000000"
  PING_COUNT=5
  MTR_CYCLES=3
fi

# --compare is a standalone mode: no measurement is taken, no output
# format flags matter, nothing else in this script runs.
if [ -n "$COMPARE_A" ]; then
  if ! have jq; then
    echo "FAILED: --compare requires jq (not installed)" >&2
    exit 1
  fi
  for f in "$COMPARE_A" "$COMPARE_B"; do
    [ -r "$f" ] || { echo "FAILED: cannot read $f" >&2; exit 1; }
  done
  echo "Comparing A=$COMPARE_A -> B=$COMPARE_B"
  printf '%-40s %18s %18s %18s\n' "KEY" "A" "B" "DELTA (B-A)"
  jq -r -n --slurpfile a "$COMPARE_A" --slurpfile b "$COMPARE_B" '
    ($a[0]) as $A | ($b[0]) as $B |
    (($A|keys) + ($B|keys) | unique | sort) as $allkeys |
    $allkeys[] as $k |
    (if $A[$k] == null then "-" else ($A[$k]|tostring) end) as $av |
    (if $B[$k] == null then "-" else ($B[$k]|tostring) end) as $bv |
    (if (($A[$k]|type)=="number") and (($B[$k]|type)=="number")
     then (($B[$k]-$A[$k])|tostring) else "-" end) as $delta |
    [$k, $av, $bv, $delta] | @tsv
  ' | awk -F'\t' '{printf "%-40s %18s %18s %18s\n", $1, $2, $3, $4}'
  exit 0
fi

if [ -n "$OUTPUT" ]; then
  exec > >(tee "$OUTPUT")
fi

# ---------------------------------------------------------------------
# JSON accumulation. section()/kv()/sample_min_median_max() are the
# only output primitives the rest of this script uses, so gating JSON
# emission inside them (instead of adding separate json_* calls at each
# call site) guarantees the JSON report can never drift from the prose
# report — they're built from literally the same calls.
#
# Backed by a file, not a bash array: sample_min_median_max() (and
# raw_dl_result/tunnel_dl_result) are captured via "$(...)" command
# substitution, which runs in a SUBSHELL — array mutations made by
# json_add() from inside that subshell would vanish the instant the
# subshell exits, silently dropping every key it tried to add. A file
# write is real process-global state and survives that.
# ---------------------------------------------------------------------
JSON_ACCUM_FILE=""
if [ "$JSON_MODE" -eq 1 ]; then
  JSON_ACCUM_FILE="$(mktemp)"
  trap 'rm -f "$JSON_ACCUM_FILE"' EXIT
fi
json_add() {
  # $1=key $2=value. A no-op outside --json so every call site can call
  # this unconditionally without checking JSON_MODE itself.
  [ "$JSON_MODE" -eq 1 ] || return 0
  [ -n "$1" ] || return 0
  jq -n --arg k "$1" --arg v "$2" '{($k): $v}' >> "$JSON_ACCUM_FILE"
}
json_emit() {
  local merged="{}"
  if [ -s "$JSON_ACCUM_FILE" ]; then
    merged="$(jq -s 'reduce .[] as $o ({}; . + $o)' "$JSON_ACCUM_FILE")"
  fi
  jq -n --argjson merged "$merged" \
    --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson quick "$([ "$QUICK" -eq 1 ] && echo true || echo false)" '
    {schema_version: 1, generated_at: $generated_at, quick_mode: $quick} +
    ( $merged | map_values(if (type=="string") and test("^-?[0-9]+(\\.[0-9]+)?$") then tonumber else . end) )
  '
}

section() {
  [ "$JSON_MODE" -eq 1 ] && return 0
  echo; echo "$1"; printf '%s\n' "${1//?/-}"
}
kv() {
  # $1=label $2=value $3=optional machine-readable key for --json
  local label="$1" value="$2" key="${3:-}"
  json_add "$key" "$value"
  [ "$JSON_MODE" -eq 1 ] && return 0
  printf '%-28s %s\n' "$label:" "$value"
}
# Prints $* only outside --json — for narrative/detail lines that have
# no single machine-readable value (mtr tables, notes, etc).
note() { [ "$JSON_MODE" -eq 1 ] && return 0; printf '%s\n' "$*"; }

# Runs $1 a total of $RUNS times, printing "min / median / max" of the
# numeric values $2 extracts (a shell function name receiving nothing,
# printing one number to stdout, or nothing on failure). Never averages
# across a mix of successes and failures — a failed sample is dropped,
# not counted as zero. $2 (optional) is a machine-readable key prefix:
# when given, also records "<prefix>_min/_median/_max/_n" for --json.
sample_min_median_max() {
  local extractor="$1" key_prefix="${2:-}"
  local -a values=()
  local i
  for ((i = 0; i < RUNS; i++)); do
    local v
    v="$("$extractor" 2>/dev/null || true)"
    [ -n "$v" ] && values+=("$v")
  done
  if [ "${#values[@]}" -eq 0 ]; then
    json_add "${key_prefix:+${key_prefix}_n}" "0"
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
  if [ -n "$key_prefix" ]; then
    json_add "${key_prefix}_min" "$min"
    json_add "${key_prefix}_median" "$median"
    json_add "${key_prefix}_max" "$max"
    json_add "${key_prefix}_n" "$n"
  fi
  echo "min=$min median=$median max=$max (n=$n)"
}

# Reads one field ($1, e.g. RcvbufErrors) from /proc/net/snmp's "Udp:"
# block. That file has two "Udp:" lines: a header naming each column,
# then the values — this matches by column name, not position, so it
# can't silently misread if the kernel ever reorders/adds columns.
udp_snmp_field() {
  local field="$1"
  awk -v f="$field" '
    $1=="Udp:" {
      n++
      if (n==1) { for (i=2;i<=NF;i++) if ($i==f) idx=i }
      else if (idx) { print $idx; exit }
    }
  ' /proc/net/snmp 2>/dev/null
}

# ---------------------------------------------------------------------
# 1. Host
# ---------------------------------------------------------------------
section "Host"
if [ -r /proc/cpuinfo ]; then
  kv "CPU model" "$(awk -F: '/model name/{print $2; exit}' /proc/cpuinfo | sed 's/^ *//')" "cpu_model"
fi
kv "vCPUs" "$(nproc 2>/dev/null || echo unavailable)" "vcpus"
if [ -r /proc/loadavg ]; then
  kv "load average" "$(awk '{print $1, $2, $3}' /proc/loadavg)" "load_average"
fi
if [ -r /proc/meminfo ]; then
  kv "RAM total" "$(awk '/MemTotal/{print $2, $3}' /proc/meminfo)" "ram_total_kb"
  kv "swap total" "$(awk '/SwapTotal/{print $2, $3}' /proc/meminfo)" "swap_total_kb"
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
    kv "CPU steal (instantaneous)" "${steal_pct}" "cpu_steal_pct"
  fi
fi
NIC="$(ip -o route get "$TARGET_HOST" 2>/dev/null | awk '{for(i=1;i<=NF;i++) if ($i=="dev") print $(i+1)}' | head -1)" || NIC=""
if [ -n "${NIC:-}" ]; then
  kv "primary interface" "$NIC" "primary_interface"
else
  kv "primary interface" "unavailable (ip not found or unreachable)" "primary_interface"
fi

# ---------------------------------------------------------------------
# Kernel tuning (effective values) — a live read of the same knobs
# deploy/lib/perf-tuning.sh / `vpn doctor --performance` manage, so a
# benchmark result can be attributed to (or cleared of) kernel tuning
# without a separate command. Read-only: this never applies anything.
# ---------------------------------------------------------------------
section "Kernel tuning (effective)"
read_sysctl() {
  local key="$1"
  if have sysctl; then
    sysctl -n "$key" 2>/dev/null && return 0
  fi
  local path="/proc/sys/${key//./\/}"
  [ -r "$path" ] && cat "$path" 2>/dev/null
}
kv "net.core.rmem_max" "$(read_sysctl net.core.rmem_max || echo unavailable)" "rmem_max"
kv "net.core.wmem_max" "$(read_sysctl net.core.wmem_max || echo unavailable)" "wmem_max"
kv "net.ipv4.tcp_congestion_control" "$(read_sysctl net.ipv4.tcp_congestion_control || echo unavailable)" "tcp_congestion_control"
kv "net.core.default_qdisc" "$(read_sysctl net.core.default_qdisc || echo unavailable)" "default_qdisc"

# ---------------------------------------------------------------------
# 2. Network path (to --target-host)
# ---------------------------------------------------------------------
section "Network path (target: $TARGET_HOST)"
if have ping; then
  PING_OUT="$(timeout 15 ping -c "$PING_COUNT" -i 0.2 -W 2 -q "$TARGET_HOST" 2>/dev/null || true)"
  if [ -n "$PING_OUT" ]; then
    LOSS="$(echo "$PING_OUT" | awk -F, '/packet loss/{for(i=1;i<=NF;i++) if ($i ~ /loss/) print $i}' | sed 's/^ *//')"
    RTT_LINE="$(echo "$PING_OUT" | awk -F= '/rtt|round-trip/{print $2}')"
    kv "packet loss" "${LOSS:-unavailable}" "packet_loss"
    kv "RTT min/avg/max/mdev (ms)" "${RTT_LINE:-unavailable}" "rtt_min_avg_max_mdev_ms"
  else
    kv "ping" "SKIPPED: target unreachable or ping blocked" "ping_status"
  fi
else
  kv "ping" "SKIPPED: ping not installed" "ping_status"
fi
if have mtr; then
  note "  mtr summary ($MTR_CYCLES cycles):"
  if [ "$JSON_MODE" -eq 0 ]; then
    timeout 30 mtr -r -c "$MTR_CYCLES" -n "$TARGET_HOST" 2>/dev/null | sed 's/^/    /' || echo "    SKIPPED: mtr failed or timed out"
  fi
else
  kv "mtr" "SKIPPED: mtr not installed (optional; install for hop-by-hop loss)" "mtr_status"
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
    kv "largest non-fragmenting ICMP payload" "${best} bytes (path MTU ~ $((best + 28)) bytes)" "path_mtu_probe_bytes"
    [ "$((best + 28))" -lt 1450 ] && note "  NOTE: path MTU is below the standard 1500 — this can cause QUIC/Hysteria2 fragmentation symptoms (stalls, poor throughput) independent of anything sing-box-side."
  else
    kv "MTU probe" "unavailable (target may block ICMP-with-DF; not conclusive)" "path_mtu_probe_bytes"
  fi
else
  kv "MTU probe" "SKIPPED: ping not installed" "path_mtu_probe_bytes"
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
  note "  download (Mbps), $RUNS run(s) against $DOWNLOAD_URL:"
  raw_dl_result="$(sample_min_median_max raw_download_mbps raw_download_mbps)"
  note "    $raw_dl_result"
  raw_latency_ms() {
    curl -o /dev/null -s -w '%{time_connect}' --max-time 10 "$DOWNLOAD_URL" 2>/dev/null \
      | awk '{printf "%.1f", $1*1000}'
  }
  note "  TCP connect latency (ms), $RUNS run(s):"
  raw_latency_result="$(sample_min_median_max raw_latency_ms raw_tcp_connect_ms)"
  note "    $raw_latency_result"
  note "  NOTE: no public upload-accepting endpoint is assumed to exist; run"
  note "  'iperf3 -s' on a second host you control and 'iperf3 -c <host>' here"
  note "  for a real symmetric throughput number if upload matters to you."
else
  kv "raw throughput" "SKIPPED: curl not installed" "raw_download_mbps_status"
fi

# ---------------------------------------------------------------------
# 4. VLESS+REALITY / Hysteria2 SERVER-SIDE PROTOCOL OVERHEAD
#    (NOT a Russia -> VPS network-path measurement — see label below)
# ---------------------------------------------------------------------
# IMPORTANT measurement-methodology note: this section runs a real
# sing-box CLIENT on this same VPS, dialing this same VPS's public IP.
# That is a genuinely real handshake and a genuinely real transfer
# through the real protocol/crypto/QUIC stack, which is useful for
# isolating protocol/CPU overhead (layer 3's raw-throughput number vs.
# this layer's tunneled number, at matched RTT ~0). It is explicitly
# NOT a measurement of the actual client-side network path (e.g. a
# Russian ISP's route to this VPS) — traffic here never leaves the
# host's own uplink/loopback-adjacent routing, so path-specific loss,
# jitter, censorship middleboxes, or peering problems on a real client's
# route are invisible to this test by construction. Do not read this
# section's numbers as "what a real user in Russia would see" — for
# that, run this same script's --target-host network-path section
# (layer 2) from an actual vantage point on that path, or run a sing-box
# client on a real remote host on that path against this VPS.
tunnel_benchmark() {
  local transport="$1" label="$2"
  section "$label protocol/server-side overhead (sing-box client on THIS VPS -> THIS VPS's public IP; NOT a remote-client network-path measurement)"
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

  # Discover the outbound tag dynamically — see
  # deploy/lib/vpn-benchmark-lib.sh's doc comment for why this never
  # hardcodes a tag string. 0 matches is a legitimate "this deployment
  # doesn't offer that transport" (SKIP); >1 matches is ambiguous and
  # must never be guessed at (hard failure, not a silent pick).
  #
  # Deliberately NOT `out_tag="$(...)"; rc=$?` — under `set -e`, a failed
  # command substitution inside a plain assignment terminates the whole
  # script right there, before `rc=$?` (or anything else) ever runs. An
  # `if`/`else` conditional is exempt from errexit by POSIX/bash
  # definition, so this is the one form that actually lets a nonzero
  # `vpn_benchmark_discover_outbound_tag` exit status reach the handling
  # below instead of silently killing the benchmark run.
  local out_tag rc
  if out_tag="$(vpn_benchmark_discover_outbound_tag "$sub_json" "$transport")"; then
    rc=0
  else
    rc=$?
  fi
  if [ "$rc" -eq 2 ]; then
    kv "$label" "SKIPPED: no $transport outbound in this deployment's subscription"
    return
  elif [ "$rc" -ne 0 ]; then
    kv "$label" "FAILED: could not identify the $transport outbound (see stderr above) — refusing to guess"
    return
  fi

  local local_socks_port=18080
  local client_cfg="$tmpdir/client.json"
  # Reuse the exact outbound the real subscription serves (so this
  # measures what a real client actually gets, not a hand-tuned test
  # config), pointed at through a local-only SOCKS inbound, routed to
  # ONLY the dynamically discovered transport's outbound tag.
  jq --arg tag "$out_tag" --argjson port "$local_socks_port" \
    '{ log: {level:"warn"}, inbounds: [{type:"mixed",tag:"in",listen:"127.0.0.1",listen_port:$port}], outbounds: .outbounds, route: {final: $tag} }' \
    <<<"$sub_json" > "$client_cfg" 2>/dev/null || {
      kv "$label" "FAILED: could not build the client config from the discovered outbound"
      return
    }

  if ! "$SINGBOX_BIN" check -c "$client_cfg" >"$tmpdir/check.log" 2>&1; then
    kv "$label" "FAILED: sing-box check rejected the generated client config (see $tmpdir/check.log if it still exists)"
    sed 's/^/    /' "$tmpdir/check.log" 2>/dev/null || true
    return
  fi

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
  local tunnel_dl_result
  tunnel_dl_result="$(sample_min_median_max tunnel_download_mbps "${transport}_mbps")"
  kv "throughput (Mbps), $RUNS run(s)" "$tunnel_dl_result"

  # CLIENT-side (this throwaway benchmark process) CPU cost of one
  # transfer.
  local sb_pid
  sb_pid="$(pgrep -f "sing-box run -c $client_cfg" | head -1 || true)"

  # SERVER-side (the real production sing-box, distinct from the
  # throwaway client above) CPU/RSS cost of the same transfer — this is
  # what tells an operator "does this transport cost the VPS itself
  # noticeably more", as opposed to the client-side number above, which
  # is this script's own disposable test process and says nothing about
  # production server load. MainPID via systemd is authoritative when
  # available; the pgrep fallback matches the fixed config path every
  # AlmaLinux deploy/health-check/acceptance-test script agrees on.
  local server_pid=""
  if have systemctl; then
    server_pid="$(systemctl show -p MainPID --value sing-box 2>/dev/null || true)"
    [ "$server_pid" = "0" ] && server_pid=""
  fi
  if [ -z "$server_pid" ]; then
    server_pid="$(pgrep -f 'sing-box run -c /etc/vpn/compat/sing-box/config.json' | head -1 || true)"
  fi

  local ct1="" ct2="" st1="" st2=""
  [ -n "$sb_pid" ] && [ -r "/proc/$sb_pid/stat" ] && ct1="$(awk '{print $14+$15}' "/proc/$sb_pid/stat")"
  [ -n "$server_pid" ] && [ -r "/proc/$server_pid/stat" ] && st1="$(awk '{print $14+$15}' "/proc/$server_pid/stat")"

  tunnel_download_mbps >/dev/null 2>&1 || true

  if [ -n "$sb_pid" ] && [ -n "$ct1" ] && [ -r "/proc/$sb_pid/stat" ]; then
    ct2="$(awk '{print $14+$15}' "/proc/$sb_pid/stat")"
    kv "sing-box client CPU ticks during one transfer" "$((ct2 - ct1))" "${transport}_client_cpu_ticks"
  fi
  if [ -n "$server_pid" ] && [ -n "$st1" ] && [ -r "/proc/$server_pid/stat" ]; then
    st2="$(awk '{print $14+$15}' "/proc/$server_pid/stat")"
    kv "sing-box SERVER process CPU ticks during one transfer" "$((st2 - st1))" "${transport}_server_cpu_ticks"
    if [ -r "/proc/$server_pid/status" ]; then
      kv "sing-box SERVER process RSS" "$(awk '/VmRSS/{print $2}' "/proc/$server_pid/status")" "${transport}_server_rss_kb"
    fi
  elif [ -z "$server_pid" ]; then
    kv "sing-box SERVER process CPU/RSS" "unavailable (production sing-box PID not found — not running under systemd as unit 'sing-box', or config not at /etc/vpn/compat/sing-box/config.json)" "${transport}_server_cpu_ticks_status"
  fi
}

# Host-wide UDP socket receive/send buffer errors, sampled immediately
# before and after the tunnel layer's real QUIC/TCP transfers. This is
# a system-wide kernel counter (not scoped to sing-box specifically —
# any UDP socket on the box counts), so it can only ever suggest, never
# prove, that Hysteria2/QUIC hit a UDP buffer limit during this run;
# treat a nonzero delta as a lead to check `vpn doctor --performance`
# and the effective net.core.rmem_max/wmem_max above, not a verdict on
# its own.
UDP_RCVBUF_ERR_BEFORE="$(udp_snmp_field RcvbufErrors)"
UDP_SNDBUF_ERR_BEFORE="$(udp_snmp_field SndbufErrors)"

tunnel_benchmark "vless-reality" "VLESS+REALITY"
tunnel_benchmark "hysteria2" "Hysteria2"

UDP_RCVBUF_ERR_AFTER="$(udp_snmp_field RcvbufErrors)"
UDP_SNDBUF_ERR_AFTER="$(udp_snmp_field SndbufErrors)"

section "UDP socket buffer errors (host-wide, before/after protocol tests)"
if [ -n "$UDP_RCVBUF_ERR_BEFORE" ] && [ -n "$UDP_RCVBUF_ERR_AFTER" ]; then
  kv "RcvbufErrors before -> after" "$UDP_RCVBUF_ERR_BEFORE -> $UDP_RCVBUF_ERR_AFTER (delta +$((UDP_RCVBUF_ERR_AFTER - UDP_RCVBUF_ERR_BEFORE)))" "udp_rcvbuf_errors_delta"
  kv "SndbufErrors before -> after" "$UDP_SNDBUF_ERR_BEFORE -> $UDP_SNDBUF_ERR_AFTER (delta +$((UDP_SNDBUF_ERR_AFTER - UDP_SNDBUF_ERR_BEFORE)))" "udp_sndbuf_errors_delta"
  note "  NOTE: host-wide counters, not sing-box-specific — a nonzero delta during this run is a lead, not proof."
else
  kv "UDP buffer errors" "unavailable (/proc/net/snmp has no Udp: RcvbufErrors/SndbufErrors fields)" "udp_buf_errors_status"
fi

if [ "$JSON_MODE" -eq 1 ]; then
  json_emit
  exit 0
fi

section "Assessment"
cat <<'EOF'
This script reports MEASUREMENTS only. It does not compute a verdict —
compare the numbers above against docs/PERFORMANCE_OPTIMIZATION_PLAN.md's
decision guide. IMPORTANT: layer 4 (VLESS+REALITY / Hysteria2) above is a
same-host hairpin test (this VPS's own sing-box client dialing this same
VPS) — it measures protocol/CPU overhead, NOT a real remote client's
network path. Read it accordingly:
  - Raw VPS download (layer 3) far below your provider's advertised rate,
    with high CPU steal -> likely a noisy-neighbor/oversubscribed host,
    not something sysctl/sing-box tuning can fix.
  - Raw VPS throughput fine, but the layer-4 hairpin throughput is much
    lower with low sing-box CPU during the transfer -> likely
    protocol/congestion-control overhead intrinsic to the tunnel at this
    RTT, not something this test can attribute to CPU or to the real
    network path (which it cannot see).
  - Layer-4 hairpin throughput much lower AND sing-box CPU (client OR the
    real SERVER process) pinned near 100% of one core -> likely CPU-bound
    userspace crypto/QUIC processing -> consider more/better vCPUs (see
    docs/PERFORMANCE_OPTIMIZATION_PLAN.md).
  - Nonzero UDP RcvbufErrors/SndbufErrors delta during the Hysteria2 run
    -> a lead (not proof) that a UDP socket hit a buffer limit; check the
    effective net.core.rmem_max/wmem_max reported above and
    `vpn doctor --performance`.
  - High packet loss or RTT/jitter in layer 2 (network path) to
    --target-host -> that IS a real network-path measurement (unlike
    layer 4) -> if --target-host is a real vantage point on your users'
    ISP path, this is evidence of a peering/routing problem no
    server-side tuning here can fix. The default --target-host (a public
    resolver near this VPS) only characterizes this VPS's own uplink,
    not your users' path.
Re-run with --target-host set to a real vantage point on your users' ISP
path (not this script's default) for a meaningful network-path reading —
this is the only layer above that can actually see that path. Layer 4
cannot, no matter how it's re-run from this VPS alone.

Re-run with --json --output <path> before/after a change and compare with
--compare <before.json> <after.json> to see exactly what moved.
EOF
