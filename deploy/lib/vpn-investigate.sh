#!/usr/bin/env bash
# Bounded, secret-free production investigation helpers. This script never
# changes services, firewall rules, credentials, routes, or Outline.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  vpn-investigate.sh target HOST [PORT]
  vpn-investigate.sh capture CLIENT_IP OUTPUT.pcap [SECONDS]
  vpn-investigate.sh summarize INPUT.pcap
  vpn-investigate.sh streaming [SECONDS]
  vpn-investigate.sh mtu HOST
  vpn-investigate.sh youtube

target validates a REALITY handshake candidate from this host over IPv4 and
IPv6. capture records only CLIENT_IP and TCP/443 or UDP/443 for at most 300s.
summarize reports packet metadata only; it never prints payload or secrets.

streaming runs a sustained (default 20s) outbound TCP transfer and a
sustained repeated-UDP probe from THIS HOST, plus local listener/conntrack/
UDP-memory readouts, to catch the class of failure a one-shot health check
misses (throughput collapse, loss, or resets that only appear under a real,
minutes-not-milliseconds flow). It never proves a real client's own path
behaves the same way — see the disclaimer it always prints.

mtu probes HOST with a DF-bit ping sweep across common packet sizes to
report the largest size that gets through without fragmentation, over
whichever of IPv4/IPv6 the host resolves to. It never modifies any MTU
setting — diagnosis only.

youtube checks DNS, TCP/443 TLS, and (where this host's curl build supports
it) QUIC/UDP/443 reachability to a fixed set of YouTube/Google domains, and
reports IPv4/IPv6 results separately. It never attempts to bypass any
restriction — it diagnoses network behavior only, and it explicitly does
NOT verify the YouTube app, Hiddify, or iOS.
EOF
}

valid_host() { [[ "$1" =~ ^([A-Za-z0-9-]+\.)*[A-Za-z0-9-]+$ ]] && [[ ${#1} -le 253 ]]; }
valid_ip() { python3 - "$1" <<'PY'
import ipaddress, sys
try: ipaddress.ip_address(sys.argv[1])
except ValueError: raise SystemExit(1)
PY
}

target() {
  local host=${1:-} port=${2:-443} family
  valid_host "$host" || { echo "invalid hostname" >&2; return 2; }
  [[ "$port" =~ ^[0-9]+$ ]] && ((port > 0 && port < 65536)) || { echo "invalid port" >&2; return 2; }
  command -v curl >/dev/null && command -v openssl >/dev/null || { echo "curl and openssl are required" >&2; return 3; }
  echo "DNS (addresses only):"
  getent ahosts "$host" | awk '{print $1}' | sort -u || true
  for family in 4 6; do
    echo "IPv${family}:"
    if curl -"$family" --silent --show-error --output /dev/null --connect-timeout 5 --max-time 15 \
      --write-out 'http=%{http_code} remote=%{remote_ip} connect=%{time_connect}s tls=%{time_appconnect}s total=%{time_total}s redirect=%{redirect_url}\n' \
      "https://${host}:${port}/"; then
      echo "IPv${family} HTTP/TLS: PASS (local target suitability only)"
    else
      echo "IPv${family} HTTP/TLS: FAIL"
    fi
  done
  if timeout 15 openssl s_client -connect "${host}:${port}" -servername "$host" -verify_hostname "$host" -tls1_3 </dev/null 2>&1 \
      | awk '/Protocol *:|Cipher *:|Verification:|Verify return code:/{print}'; then
    echo "certificate/SNI check completed"
  else
    echo "certificate/SNI check failed"
  fi
  if command -v ping >/dev/null; then ping -c 5 -W 2 "$host" | tail -2 || echo "ICMP unavailable or filtered"; fi
  echo "RESULT: this does not test REALITY authentication or Russian-network compatibility."
}

capture() {
  local ip=${1:-} output=${2:-} seconds=${3:-120}
  valid_ip "$ip" || { echo "invalid client IP" >&2; return 2; }
  [[ "$output" == *.pcap ]] || { echo "output must end in .pcap" >&2; return 2; }
  [[ "$seconds" =~ ^[0-9]+$ ]] && ((seconds >= 1 && seconds <= 300)) || { echo "seconds must be 1..300" >&2; return 2; }
  command -v tcpdump >/dev/null || { echo "tcpdump is required" >&2; return 3; }
  umask 077
  echo "Capturing only host $ip and TCP/443 or UDP/443 for ${seconds}s -> $output"
  timeout --signal=INT "${seconds}s" tcpdump -i any -nn -s 160 -w "$output" \
    "host $ip and (tcp port 443 or udp port 443)" || [[ $? -eq 124 ]]
  chmod 600 "$output"
}

summarize() {
  local input=${1:-}
  [[ -f "$input" ]] || { echo "pcap not found" >&2; return 2; }
  command -v tshark >/dev/null || { echo "tshark is required" >&2; return 3; }
  echo "Packets/bytes by conversation (metadata only):"
  tshark -n -r "$input" -q -z conv,tcp -z conv,udp
  echo "TCP events:"
  printf 'syn='; tshark -n -r "$input" -Y 'tcp.flags.syn==1 && tcp.flags.ack==0' -T fields -e frame.number | wc -l
  printf 'rst='; tshark -n -r "$input" -Y 'tcp.flags.reset==1' -T fields -e frame.number | wc -l
  printf 'fin='; tshark -n -r "$input" -Y 'tcp.flags.fin==1' -T fields -e frame.number | wc -l
  printf 'retransmissions='; tshark -n -r "$input" -Y 'tcp.analysis.retransmission' -T fields -e frame.number | wc -l
  echo "Timeline (epoch, protocol, frame length, TCP flags; no payload):"
  tshark -n -r "$input" -T fields -e frame.time_epoch -e _ws.col.Protocol -e frame.len -e tcp.flags
  echo "Authentication/application state requires synchronized sing-box logs; packet metadata alone cannot label DPI or credential failure."
}

# ---------------------------------------------------------------------------
# streaming — P2: sustained-flow diagnostics. A one-shot health check (a
# bound socket, a single small request) cannot see conntrack eviction,
# throughput collapse, or loss that only shows up under a real, sustained
# flow — exactly the gap between "vpn doctor passes" and "video/voice
# actually works". Everything here is read-only / outbound-only: no
# service, firewall, or sysctl is touched.
# ---------------------------------------------------------------------------

# Conservative, documented thresholds — same values as
# `apps/admin/src/main.rs`'s `CONNTRACK_WARN_THRESHOLD_PCT` /
# `UDP_MEM_WARN_THRESHOLD_PCT`, so a WARN from this script and a WARN from
# `vpn doctor` mean the same thing. Keep these two in sync if either changes.
STREAMING_CONNTRACK_WARN_PCT=80
STREAMING_UDPMEM_WARN_PCT=80
# Default public HTTP(S) endpoint for the sustained-transfer test: a
# well-known CDN speed-test object, not tied to any single provider's
# reputation and already used elsewhere in this project's benchmark
# tooling (deploy/lib/vpn-benchmark-lib.sh) for the same reason.
STREAMING_DOWNLOAD_URL="${STREAMING_DOWNLOAD_URL:-https://speed.cloudflare.com/__down?bytes=104857600}"
# A small, fixed set of public resolvers for the sustained UDP probe —
# deliberately not the deployment's own Hysteria2 listener (that would
# require live credentials and mutate nothing, but this script is meant to
# run with zero dependency on any per-deployment secret).
STREAMING_UDP_TARGETS=(1.1.1.1 8.8.8.8 9.9.9.9)

streaming_conntrack() {
  local count max
  count=$(cat /proc/sys/net/netfilter/nf_conntrack_count 2>/dev/null || true)
  max=$(cat /proc/sys/net/netfilter/nf_conntrack_max 2>/dev/null || true)
  if [[ -z "$count" || -z "$max" || "$max" -eq 0 ]]; then
    echo "Conntrack utilization: unavailable (nf_conntrack module not loaded, or unreadable)"
    return
  fi
  local pct=$(( count * 1000 / max ))
  local whole=$(( pct / 10 )) frac=$(( pct % 10 ))
  if (( whole >= STREAMING_CONNTRACK_WARN_PCT )); then
    echo "[WARN] Conntrack utilization: ${count} / ${max} (${whole}.${frac}%) — at or above the ${STREAMING_CONNTRACK_WARN_PCT}% warning threshold"
  else
    echo "[OK]   Conntrack utilization: ${count} / ${max} (${whole}.${frac}%)"
  fi
}

streaming_udp_memory() {
  local udp_mem in_use
  udp_mem=$(cat /proc/sys/net/ipv4/udp_mem 2>/dev/null || true)
  if [[ -z "$udp_mem" ]]; then
    echo "UDP memory pressure: unavailable (net.ipv4.udp_mem unreadable)"
    return
  fi
  local pressure_pages
  pressure_pages=$(awk '{print $2}' <<<"$udp_mem")
  in_use=$(awk '/^UDP:/{for(i=1;i<=NF;i++) if ($i=="mem") print $(i+1)}' /proc/net/sockstat 2>/dev/null || true)
  if [[ -z "$in_use" || -z "$pressure_pages" || "$pressure_pages" -eq 0 ]]; then
    echo "UDP memory pressure: unavailable (/proc/net/sockstat unreadable)"
    return
  fi
  local pct=$(( in_use * 1000 / pressure_pages ))
  local whole=$(( pct / 10 )) frac=$(( pct % 10 ))
  if (( whole >= STREAMING_UDPMEM_WARN_PCT )); then
    echo "[WARN] UDP memory pressure: ${in_use} / ${pressure_pages} pages (${whole}.${frac}% of pressure threshold)"
  else
    echo "[OK]   UDP memory pressure: normal (${in_use} / ${pressure_pages} pages, ${whole}.${frac}% of pressure threshold)"
  fi
}

streaming_listeners() {
  command -v ss >/dev/null || { echo "[WARN] listener check: \`ss\` unavailable"; return; }
  if ss -tlnp 2>/dev/null | grep -q ':443 '; then
    echo "[OK]   TCP/443 listener present"
  else
    echo "[WARN] TCP/443 listener not observed (expected if REALITY is not installed on this host)"
  fi
  if ss -ulnp 2>/dev/null | grep -q ':443 '; then
    echo "[OK]   UDP/443 listener present"
  else
    echo "[WARN] UDP/443 listener not observed (expected if Hysteria2 is not installed on this host)"
  fi
}

# Sustained TCP transfer: downloads for up to SECONDS, sampling
# instantaneous speed every ~2s via curl's own progress reporting, so a
# collapse partway through (not just a low final average) is visible.
streaming_tcp_transfer() {
  local seconds=$1
  command -v curl >/dev/null || { echo "[WARN] TCP sustained flow: curl unavailable"; return; }
  echo "Sustained TCP transfer (up to ${seconds}s, ${STREAMING_DOWNLOAD_URL}):"
  local out
  out=$(timeout "$((seconds + 10))" curl --silent --show-error --output /dev/null \
    --max-time "$seconds" \
    --write-out 'http=%{http_code} bytes=%{size_download} avg_speed_bytes_per_sec=%{speed_download} total=%{time_total}s\n' \
    "$STREAMING_DOWNLOAD_URL" 2>&1) || true
  echo "  $out"
  local speed
  speed=$(grep -oE 'avg_speed_bytes_per_sec=[0-9.]+' <<<"$out" | cut -d= -f2 | cut -d. -f1 || true)
  local bytes
  bytes=$(grep -oE 'bytes=[0-9]+' <<<"$out" | cut -d= -f2 || true)
  if [[ -z "$bytes" || "$bytes" -eq 0 ]]; then
    echo "[WARN] TCP sustained flow: no bytes transferred — the endpoint may be unreachable or the transfer failed before starting"
  elif [[ -n "$speed" && "$speed" -lt 51200 ]]; then
    # Below 50 KB/s sustained is a real, conservative floor: even a
    # heavily loaded low-tier VPS should clear this for a plain HTTPS
    # download with nothing else competing for the link during this test.
    echo "[WARN] TCP sustained flow: average throughput ${speed} B/s is unusually low — possible congestion, loss, or a slow/rate-limited path"
  else
    echo "[OK]   TCP sustained flow: PASS"
  fi
}

# Sustained UDP probe: repeated independent DNS-over-UDP queries spread
# over SECONDS (not one query — a burst can hide loss that only shows up
# once conntrack/NAT state has been open for a while, which is exactly the
# failure class a one-shot UDP-bound-socket check cannot see). This is a
# real, standards-compliant UDP round trip (RFC 1035 over UDP/53), not a
# synthetic ping — the closest meaningful sustained-UDP test achievable
# without a live per-deployment Hysteria2 credential.
streaming_udp_transfer() {
  local seconds=$1
  command -v python3 >/dev/null || { echo "[WARN] UDP sustained flow: python3 unavailable"; return; }
  echo "Sustained UDP probe (repeated DNS-over-UDP queries over ~${seconds}s):"
  python3 - "$seconds" "${STREAMING_UDP_TARGETS[@]}" <<'PY'
import socket, struct, sys, time, random

duration = float(sys.argv[1])
targets = sys.argv[2:]

def make_query():
    txid = random.randint(0, 0xFFFF)
    header = struct.pack(">HHHHHH", txid, 0x0100, 1, 0, 0, 0)
    qname = b"".join(bytes([len(p)]) + p.encode() for p in "example.com".split(".")) + b"\x00"
    question = qname + struct.pack(">HH", 1, 1)  # A, IN
    return txid, header + question

sent = 0
recv = 0
rtts = []
deadline = time.monotonic() + duration
interval = 0.5
while time.monotonic() < deadline:
    for host in targets:
        txid, query = make_query()
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.settimeout(2.0)
        t0 = time.monotonic()
        try:
            sock.sendto(query, (host, 53))
            sent += 1
            data, _ = sock.recvfrom(512)
            if len(data) >= 2 and struct.unpack(">H", data[0:2])[0] == txid:
                recv += 1
                rtts.append(time.monotonic() - t0)
        except OSError:
            pass
        finally:
            sock.close()
    time.sleep(interval)

loss_pct = 0.0 if sent == 0 else (1 - recv / sent) * 100
avg_rtt_ms = (sum(rtts) / len(rtts) * 1000) if rtts else float("nan")
print(f"  sent={sent} received={recv} loss={loss_pct:.1f}% avg_rtt={avg_rtt_ms:.1f}ms" if rtts
      else f"  sent={sent} received={recv} loss={loss_pct:.1f}%")

if sent == 0:
    print("[WARN] UDP sustained flow: no probes were sent")
elif loss_pct >= 20:
    print(f"[WARN] UDP sustained flow: {loss_pct:.1f}% loss over the test window — sustained UDP (Hysteria2/QUIC-relevant) may be unreliable on this path")
elif loss_pct > 0:
    print(f"[OK]   UDP sustained flow: PASS (some loss observed, {loss_pct:.1f}%, within normal variance)")
else:
    print("[OK]   UDP sustained flow: PASS")
PY
}

streaming_ip_family() {
  command -v curl >/dev/null || { echo "[WARN] IPv4/IPv6 reachability: curl unavailable"; return; }
  local family label
  for family in 4 6; do
    label="IPv${family}"
    if curl -s"${family}" --output /dev/null --connect-timeout 5 --max-time 10 https://www.gstatic.com/generate_204 2>/dev/null; then
      echo "[OK]   ${label}: PASS (reachability only)"
    else
      echo "[INFO] ${label}: not reachable from this host (no ${label} connectivity, or the path is blocked)"
    fi
  done
}

streaming() {
  local seconds=${1:-20}
  [[ "$seconds" =~ ^[0-9]+$ ]] && ((seconds >= 5 && seconds <= 120)) || { echo "seconds must be 5..120" >&2; return 2; }
  echo "vpn1 sustained-flow diagnostics (~${seconds}s TCP window, ~${seconds}s UDP window)"
  echo "Purpose: catch failures that only appear under a real, minutes-not-milliseconds flow —"
  echo "a healthy \`vpn doctor\` or a bound-socket check can coexist with exactly this class of break."
  echo
  streaming_listeners
  streaming_conntrack
  streaming_udp_memory
  echo
  streaming_tcp_transfer "$seconds"
  echo
  streaming_udp_transfer "$seconds"
  echo
  streaming_ip_family
  echo
  echo "This test does not verify Hiddify/iOS/YouTube behavior, and it does not run through this"
  echo "deployment's own REALITY/Hysteria2 tunnel — it measures THIS HOST's own outbound network"
  echo "path, which every tunneled client's traffic ultimately depends on. A PASS here narrows the"
  echo "search; it does not prove a specific remote client/app works."
}

# ---------------------------------------------------------------------------
# mtu — P5: PMTU/fragmentation diagnosis, never a global override. Reports
# evidence; changes nothing.
# ---------------------------------------------------------------------------

mtu() {
  local host=${1:-}
  valid_host "$host" || { echo "invalid hostname" >&2; return 2; }
  command -v ping >/dev/null || { echo "ping is required" >&2; return 3; }
  echo "PMTU probe against $host (DF-bit ping sweep — diagnosis only, no MTU is changed)"

  local resolved_v4 resolved_v6
  resolved_v4=$(getent ahostsv4 "$host" 2>/dev/null | awk '{print $1; exit}' || true)
  resolved_v6=$(getent ahostsv6 "$host" 2>/dev/null | awk '{print $1; exit}' || true)

  # ICMP payload sizes chosen to bracket common real-world PMTU failure
  # points: 1472 = max unfragmented ICMP payload on a plain 1500-byte-MTU
  # IPv4 path (1500 - 20 IP - 8 ICMP); 1400/1372/1300/1200 step down
  # through the range where PPPoE, GRE/IPsec, WireGuard, and various VPN
  # encapsulation overheads commonly land; 1200 is IPv6's own guaranteed
  # minimum PMTU (RFC 8200 §5) — anything failing at 1200 indicates a
  # problem well beyond ordinary encapsulation overhead.
  local sizes=(1472 1400 1372 1300 1200)
  local largest_ok=0
  local family payload flag cmd

  if [[ -n "$resolved_v4" ]]; then
    echo "IPv4 ($resolved_v4):"
    for payload in "${sizes[@]}"; do
      if ping -4 -M do -c 2 -W 2 -s "$payload" "$resolved_v4" >/dev/null 2>&1; then
        echo "  payload ${payload}B (~$((payload + 28))B on the wire): OK"
        (( payload > largest_ok )) && largest_ok=$payload
      else
        echo "  payload ${payload}B (~$((payload + 28))B on the wire): FAIL or fragmentation-needed"
      fi
    done
  else
    echo "IPv4: no A record for $host"
  fi

  local largest_ok_v6=0
  if [[ -n "$resolved_v6" ]]; then
    echo "IPv6 ($resolved_v6):"
    # IPv6 has no DF bit (fragmentation is source-only by design, RFC
    # 8200) — the closest ping-based equivalent is -M do, which most
    # ping6 implementations map to disabling fragmentation and surfacing
    # a Packet Too Big response instead.
    for payload in "${sizes[@]}"; do
      if ping -6 -M do -c 2 -W 2 -s "$payload" "$resolved_v6" >/dev/null 2>&1; then
        echo "  payload ${payload}B (~$((payload + 48))B on the wire): OK"
        (( payload > largest_ok_v6 )) && largest_ok_v6=$payload
      else
        echo "  payload ${payload}B (~$((payload + 48))B on the wire): FAIL or Packet-Too-Big"
      fi
    done
  else
    echo "IPv6: no AAAA record for $host"
  fi

  echo
  if [[ -n "$resolved_v4" ]]; then
    if [[ "$largest_ok" -eq 0 ]]; then
      echo "[WARN] IPv4: even the smallest tested payload (${sizes[-1]}B) failed — this may indicate ICMP is filtered rather than a real PMTU problem; not conclusive on its own."
    elif [[ "$largest_ok" -lt 1400 ]]; then
      echo "[WARN] Path appears unable to sustain IPv4 packets above ~${largest_ok}B payload (~$((largest_ok + 28))B on the wire). Client-side MTU testing is recommended."
    else
      echo "[OK]   IPv4 path sustains at least ${largest_ok}B payloads without fragmentation."
    fi
  fi
  if [[ -n "$resolved_v6" ]]; then
    if [[ "$largest_ok_v6" -lt 1400 && "$largest_ok_v6" -gt 0 ]]; then
      echo "[WARN] Path appears unable to sustain IPv6 packets above ~${largest_ok_v6}B payload. Client-side MTU testing is recommended."
    elif [[ "$largest_ok_v6" -ge 1400 ]]; then
      echo "[OK]   IPv6 path sustains at least ${largest_ok_v6}B payloads without fragmentation."
    fi
  fi
  echo
  echo "This is a diagnostic signal, not proof — some networks/hosts filter the ICMP messages this"
  echo "test relies on, which looks identical to a real PMTU failure. No MTU setting was changed by"
  echo "this command, on this host or any client."
}

# ---------------------------------------------------------------------------
# youtube — P6: distinguishes "VPN/host network broken" from "TCP fine, UDP
# not" from "a specific Google endpoint problem" from "client/Hiddify/iOS
# problem" (which this script cannot observe at all). Never attempts to
# bypass anything; reachability/DNS/TLS diagnosis only.
# ---------------------------------------------------------------------------

YOUTUBE_DOMAINS=(youtube.com googlevideo.com youtubei.googleapis.com ytimg.com)

youtube_dns() {
  local host=$1
  local a aaaa
  a=$(getent ahostsv4 "$host" 2>/dev/null | awk '{print $1}' | sort -u | paste -sd, - || true)
  aaaa=$(getent ahostsv6 "$host" 2>/dev/null | awk '{print $1}' | sort -u | paste -sd, - || true)
  if [[ -n "$a" ]]; then
    echo "[OK]   $host DNS (A): $a"
  else
    echo "[WARN] $host DNS (A): no result"
  fi
  if [[ -n "$aaaa" ]]; then
    echo "[OK]   $host DNS (AAAA): $aaaa"
  else
    echo "[INFO] $host DNS (AAAA): no result (host may simply not publish one)"
  fi
}

youtube_tcp() {
  local host=$1 family=$2
  command -v curl >/dev/null || return
  # A bare connectivity probe, not a claim about any particular HTTP
  # status: googlevideo.com/youtubei.googleapis.com legitimately answer
  # most unauthenticated bare requests with 4xx — that is a normal,
  # correctly-routed response from the real service, not a failure. Only
  # curl's own transport-level exit status (TLS handshake completed, some
  # HTTP response was received at all) is treated as PASS/FAIL here.
  if curl -s"${family}" --output /dev/null --connect-timeout 5 --max-time 10 \
      -w '' "https://${host}/" >/dev/null 2>&1; then
    echo "[OK]   $host IPv${family}/TCP/443+TLS: PASS (connection + TLS handshake completed; HTTP status is not evaluated)"
  else
    local rc=$?
    echo "[WARN] $host IPv${family}/TCP/443+TLS: FAIL (curl exit $rc — connection or TLS handshake did not complete)"
  fi
}

youtube_quic() {
  local host=$1
  command -v curl >/dev/null || { echo "[INFO] $host QUIC/UDP/443: curl unavailable"; return; }
  if ! curl --version 2>/dev/null | head -1 | grep -qi 'HTTP3'; then
    echo "[INFO] $host QUIC/UDP/443: not tested — this host's curl build has no HTTP/3 support, so there is no standards-compliant QUIC client available here. This is a tooling gap, not a network result."
    return
  fi
  if curl --http3-only -s --output /dev/null --connect-timeout 5 --max-time 10 "https://${host}/" 2>/dev/null; then
    echo "[OK]   $host QUIC/UDP/443: PASS (HTTP/3 request completed over QUIC)"
  else
    echo "[WARN] $host QUIC/UDP/443: FAIL (HTTP/3-only request did not complete — does not by itself distinguish UDP blocking from a server declining HTTP/3 for this request)"
  fi
}

youtube() {
  echo "YouTube/Google endpoint investigation (server-side network diagnosis only)"
  echo "This does NOT attempt to bypass any restriction, and does NOT verify the YouTube app,"
  echo "Hiddify, or iOS — it only characterizes this host's own network path to these domains."
  echo
  local host
  for host in "${YOUTUBE_DOMAINS[@]}"; do
    youtube_dns "$host"
  done
  echo
  for host in "${YOUTUBE_DOMAINS[@]}"; do
    youtube_tcp "$host" 4
    youtube_tcp "$host" 6
  done
  echo
  # googlevideo.com and youtubei.googleapis.com are where actual video
  # delivery and playback API calls happen — the two most relevant to the
  # reported "playback fails" symptom specifically.
  youtube_quic googlevideo.com
  youtube_quic youtubei.googleapis.com
  echo
  echo "Server-side connectivity is characterized above. This does NOT verify the YouTube iOS app,"
  echo "Hiddify's TUN/routing behavior, or anything on the client device — only this host's own"
  echo "network path to these domains, dialed directly (not through this deployment's own tunnel)."
}

case ${1:-} in
  target) shift; target "$@" ;;
  capture) shift; capture "$@" ;;
  summarize) shift; summarize "$@" ;;
  streaming) shift; streaming "$@" ;;
  mtu) shift; mtu "$@" ;;
  youtube) shift; youtube "$@" ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
