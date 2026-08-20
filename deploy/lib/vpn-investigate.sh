#!/usr/bin/env bash
# Bounded, secret-free production investigation helpers. This script never
# changes services, firewall rules, credentials, routes, or Outline.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  vpn-investigate.sh target HOST [PORT]
  vpn-investigate.sh capture CLIENT_IP OUTPUT.pcap [SECONDS]
  vpn-investigate.sh udp-egress-capture CLIENT_IP OUTPUT.pcap [SECONDS]
  vpn-investigate.sh summarize INPUT.pcap
  vpn-investigate.sh udp-egress-verdict INPUT.pcap CLIENT_IP
  vpn-investigate.sh streaming [SECONDS]
  vpn-investigate.sh mtu HOST
  vpn-investigate.sh youtube
  vpn-investigate.sh tiktok
  vpn-investigate.sh client CLIENT_IP

target validates a REALITY handshake candidate from this host over IPv4 and
IPv6. capture records only CLIENT_IP and TCP/443 or UDP/443 for at most 300s.
summarize reports packet metadata only; it never prints payload or secrets.

udp-egress-capture records CLIENT_IP's TCP/443 tunnel traffic together with
ALL host-wide UDP/443 (the VPS's own application-QUIC egress, if any) for at
most 300s, so an operator can correlate "phone was using the tunnel" against
"VPS emitted/received UDP/443" by timestamp — see
docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md's Phase-1 experiment for why this is
a different question than `capture` answers.

udp-egress-verdict reads an INPUT.pcap produced by udp-egress-capture (or
capture) and CLIENT_IP, and prints a single labeled Phase-1 verdict —
Case A/D (no UDP/443 observed at all — cannot distinguish "client never
attempted QUIC" from "client attempted it but it was never relayed" from
server-side packet metadata alone), Case B (UDP/443 left this host but no
reply ever returned), or Case C (bidirectional UDP/443 present) — by
correlating the client's REALITY TCP/443 tunnel window against host-wide
UDP/443 packet direction, all timestamps in UTC. It does not itself decide
the fix; see docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §8's decision tree
for what each case means next. Reuse the exact iPhone reset procedure in
that document's §9.1/§9.7 before capturing — this command only reads an
existing pcap, it starts no capture and changes nothing on the phone.

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

tiktok checks DNS, TCP/443 TLS, and (where this host's curl build supports
it) QUIC/UDP/443 reachability to a fixed set of TikTok control-plane
(www.tiktok.com/tiktok.com) and CDN/media (tiktokcdn.com/tiktokv.com)
root domains, reported separately so a control-plane-reachable /
media-CDN-unreachable split is visible instead of one merged verdict. It
reports IPv4/IPv6 results separately. It never attempts to bypass any
restriction — it diagnoses network behavior only, and it explicitly does
NOT verify the TikTok app, Hiddify, or iOS/Android, and it does NOT
determine whether TikTok's own regional service policy (see
docs/TIKTOK_INVESTIGATION.md) applies independently of this result.

client gathers this server's own view of one reported client IP's connection
attempts: protocol self-test result, TCP/443 and UDP/443 listener presence,
recent sing-box journal entries mentioning that IP, host-wide counts of
REALITY invalid-connection/accepted/reset events, a suggested (not run)
bounded capture command, and read-only firewall state. Every line is labeled
FACT/INFERENCE/UNKNOWN; no secret is ever printed; nothing is mutated.
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

# ---------------------------------------------------------------------------
# udp-egress-capture — the Phase-1 "does application UDP/443 actually leave
# the VPS" experiment from docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md. A plain
# `capture CLIENT_IP` (above) only sees traffic to/from CLIENT_IP — under
# VLESS+REALITY that is TCP/443 tunnel traffic, never the VPS's OWN outbound
# UDP/443 toward Google/YouTube (that traffic's IP layer is CLIENT_IP <->
# VPS on the tunnel side, and VPS <-> Google on the egress side — two
# different conversations that never share a src/dst pair). This captures
# BOTH legs in one bounded window so an operator can correlate them by
# timestamp: the client's REALITY TCP/443 tunnel traffic (proves when the
# phone was actively using the VPN) and ALL UDP/443 host-wide (proves
# whether the VPS emitted/received any application QUIC during that same
# window, from ANY relayed session, not just this one client — this host
# only has one client's tunnel active per capture in practice, so a
# host-wide UDP/443 filter is the closest bounded proxy for "traffic this
# client's relayed session generated" without payload inspection or
# per-flow NAT-table correlation, which this tool deliberately does not
# attempt). Same safety envelope as `capture`: metadata only via
# `summarize`, bounded duration, no service/firewall/route mutation.
# ---------------------------------------------------------------------------
udp_egress_capture() {
  local ip=${1:-} output=${2:-} seconds=${3:-60}
  valid_ip "$ip" || { echo "invalid client IP" >&2; return 2; }
  [[ "$output" == *.pcap ]] || { echo "output must end in .pcap" >&2; return 2; }
  [[ "$seconds" =~ ^[0-9]+$ ]] && ((seconds >= 1 && seconds <= 300)) || { echo "seconds must be 1..300" >&2; return 2; }
  command -v tcpdump >/dev/null || { echo "tcpdump is required" >&2; return 3; }
  umask 077
  echo "Capturing (host $ip and tcp port 443) or (udp port 443), host-wide, for ${seconds}s -> $output"
  echo "This is two DIFFERENT conversations in one file: the client's REALITY tunnel (TCP/443"
  echo "to/from $ip) and ANY host-wide UDP/443 (the VPS's own application-QUIC egress, if any)."
  echo "Correlate by timestamp with 'summarize', not by src/dst pair — they will not match."
  timeout --signal=INT "${seconds}s" tcpdump -i any -nn -s 160 -w "$output" \
    "(host $ip and tcp port 443) or udp port 443" || [[ $? -eq 124 ]]
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
# udp-egress-verdict — Phase-1 A/B/C/D verdict from
# docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §9.1/§8. `summarize` above prints
# raw packet metadata and leaves the A/B/C/D read to the operator; this
# function does that correlation itself so a single command answers the
# Phase-1 question instead of requiring a human to eyeball a timeline.
#
# Correlation method (deliberately simple, no per-flow NAT-table
# reconstruction, same restraint as udp-egress-capture's own comment):
#   - TCP/443 packets to/from CLIENT_IP prove the REALITY tunnel was in use
#     during the window (same signal `client CLIENT_IP` already reports).
#   - Because the capture is host-wide for UDP/443 (see udp-egress-capture),
#     THIS HOST is always one endpoint of every captured UDP/443 packet —
#     there is no third party being captured. So udp.dstport==443 packets
#     are this host acting as the SENDER (egress toward some external
#     UDP/443 service — outbound) and udp.srcport==443 packets are this
#     host acting as the RECEIVER (a reply arriving FROM some external
#     UDP/443 service — inbound). This needs no local-IP enumeration.
#   - "host-wide" UDP/443 is a proxy for "traffic this client's relayed
#     session generated", not a proven per-flow attribution — restated
#     explicitly in the verdict output, not just this comment.
#
# What this can and cannot distinguish (the documented gap this function
# exists to close): it can tell outbound-only (Case B) from bidirectional
# (Case C) from neither (Case A/D) with the tunnel active. It CANNOT tell
# Case A (client attempted QUIC, Hiddify/sing-box never relayed it) from
# Case D (client never attempted QUIC at all) — both look identical from
# this host's own NIC, because both mean zero UDP/443 packets appear here.
# That is stated as UNKNOWN in the output, never guessed at.
# ---------------------------------------------------------------------------
udp_egress_verdict() {
  local input=${1:-} client_ip=${2:-}
  valid_ip "$client_ip" || { echo "invalid client IP" >&2; return 2; }
  [[ -f "$input" ]] || { echo "pcap not found" >&2; return 2; }
  command -v tshark >/dev/null || { echo "tshark is required" >&2; return 3; }

  echo "Phase-1 A/B/C/D verdict for $input, client $client_ip"
  echo "======================================================================"
  echo "All timestamps below are UTC, to correlate with 'journalctl -u sing-box' output"
  echo "captured separately (see docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §9.1)."
  echo "Every line is labeled FACT, INFERENCE, or UNKNOWN. No secret is ever printed."
  echo

  local tcp_count
  tcp_count=$(tshark -n -r "$input" -Y "ip.addr==${client_ip} and tcp.port==443" -T fields -e frame.number 2>/dev/null | wc -l)
  if [[ "$tcp_count" -eq 0 ]]; then
    echo "FACT: 0 TCP/443 packets to/from ${client_ip} (the REALITY tunnel) observed in this capture."
    echo "INFERENCE: this is a PROCEDURAL FAILURE, not a network finding — the experiment did not"
    echo "  actually run (wrong CLIENT_IP, phone not connected during the window, or the reset"
    echo "  procedure in docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §9.1/§9.7 was skipped or not"
    echo "  immediately followed by opening the app). Per §9.1's FAIL interpretation: redo the"
    echo "  capture, do not record this as evidence."
    echo
    echo "VERDICT: INCONCLUSIVE — no tunnel activity observed; UDP/443 egress cannot be evaluated."
    return 0
  fi

  local tcp_first tcp_last
  tcp_first=$(tshark -n -r "$input" -Y "ip.addr==${client_ip} and tcp.port==443" -T fields -e frame.time_epoch 2>/dev/null | head -1)
  tcp_last=$(tshark -n -r "$input" -Y "ip.addr==${client_ip} and tcp.port==443" -T fields -e frame.time_epoch 2>/dev/null | tail -1)
  echo "FACT: ${tcp_count} TCP/443 packets to/from ${client_ip} observed — the REALITY tunnel was"
  echo "  active during this capture."
  echo "FACT: tunnel window (UTC): $(date -u -d "@${tcp_first%.*}" '+%Y-%m-%dT%H:%M:%SZ') .. $(date -u -d "@${tcp_last%.*}" '+%Y-%m-%dT%H:%M:%SZ')"
  echo

  local udp_out udp_in
  udp_out=$(tshark -n -r "$input" -Y 'udp.dstport==443' -T fields -e frame.number 2>/dev/null | wc -l)
  udp_in=$(tshark -n -r "$input" -Y 'udp.srcport==443' -T fields -e frame.number 2>/dev/null | wc -l)
  echo "FACT: host-wide UDP/443 packets with THIS HOST as sender (dst port 443 — egress toward"
  echo "  some external UDP/443 service): ${udp_out}"
  echo "FACT: host-wide UDP/443 packets with THIS HOST as receiver (src port 443 — a reply"
  echo "  arriving from some external UDP/443 service): ${udp_in}"
  echo "INFERENCE: 'host-wide' means ANY relayed session on this VPS during the window, not"
  echo "  provably traffic relayed from ${client_ip} specifically — see udp-egress-capture's"
  echo "  own comment for why this proxy is used instead of per-flow NAT-table correlation."
  if [[ "$udp_out" -gt 0 ]]; then
    local udp_out_first
    udp_out_first=$(tshark -n -r "$input" -Y 'udp.dstport==443' -T fields -e frame.time_epoch 2>/dev/null | head -1)
    echo "FACT: first outbound UDP/443 packet (UTC): $(date -u -d "@${udp_out_first%.*}" '+%Y-%m-%dT%H:%M:%SZ')"
  fi
  if [[ "$udp_in" -gt 0 ]]; then
    local udp_in_first
    udp_in_first=$(tshark -n -r "$input" -Y 'udp.srcport==443' -T fields -e frame.time_epoch 2>/dev/null | head -1)
    echo "FACT: first inbound UDP/443 reply (UTC): $(date -u -d "@${udp_in_first%.*}" '+%Y-%m-%dT%H:%M:%SZ')"
  fi
  echo

  echo "======================================================================"
  if [[ "$udp_out" -eq 0 && "$udp_in" -eq 0 ]]; then
    echo "VERDICT — Case A/D: no UDP/443 observed (cannot distinguish client-not-attempted from"
    echo "  client-not-relayed from server side alone)."
    echo "INFERENCE: the REALITY tunnel was active throughout the window (see the FACT above), but"
    echo "  genuinely zero UDP/443 packets crossed this host's egress interface in either direction."
    echo "UNKNOWN: whether the YouTube app never attempted QUIC/UDP at all (Case D) or attempted it"
    echo "  but Hiddify/sing-box never relayed it (Case A) — that distinction requires client-side"
    echo "  (Hiddify/iOS) instrumentation this server cannot observe. Proceed to §6.5's A/B/C"
    echo "  reject-rule test per docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §8's decision tree."
  elif [[ "$udp_out" -gt 0 && "$udp_in" -eq 0 ]]; then
    echo "VERDICT — Case B: UDP/443 outbound only, no replies."
    echo "INFERENCE: application UDP/443 left this VPS but no reply was ever observed returning —"
    echo "  consistent with a VPS/provider/upstream egress problem (§3.3/§7 of that document), not"
    echo "  a client or relay problem. Does not by itself rule out a reply arriving after the"
    echo "  capture window closed; re-run with a longer window if this result is unexpected."
  else
    echo "VERDICT — Case C: bidirectional UDP/443 present."
    echo "INFERENCE: application UDP/443 both left and returned to this VPS during the window. If"
    echo "  the app still failed during this same window, that points downstream — relay/TUN/"
    echo "  session/MTU behavior (§3.2/§3.4/§3.11 of that document) — not VPS egress reachability."
  fi
  echo
  echo "UNKNOWN: this verdict characterizes packets crossing THIS HOST's own interfaces only. It"
  echo "  cannot observe the phone's TUN state, whether the YouTube app actually attempted QUIC,"
  echo "  or Hiddify/sing-box's internal relay decision — see"
  echo "  docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §1's client/server observability boundary."
}

# ---------------------------------------------------------------------------
# client — remote client investigation. Gathers everything this host can
# see about ONE reported client IP's connection attempts, without ever
# printing secrets (REALITY private/public key, VLESS UUID, Hysteria2
# password) or running any capture/mutation itself — it only suggests the
# `capture` command above, it never invokes it. Every line is labeled
# FACT (directly observed on this host right now), INFERENCE (a
# conclusion drawn from those facts, clearly hedged), or UNKNOWN (this
# host cannot determine it) — see docs/RUSSIA_PRODUCTION_INVESTIGATION.md
# and docs/INCIDENT_2026-08-11_REALITY_HANDSHAKE_INCONCLUSIVE.md for why
# `processed invalid connection` alone must never be silently upgraded to
# a DPI/censorship/bad-credential verdict.
# ---------------------------------------------------------------------------
client() {
  local ip=${1:-}
  valid_ip "$ip" || { echo "invalid client IP" >&2; return 2; }
  echo "Client investigation for $ip"
  echo "======================================================================"
  echo "Every line below is labeled FACT, INFERENCE, or UNKNOWN. No secret"
  echo "(REALITY key material, VLESS UUID, Hysteria2 password) is ever printed."
  echo

  echo "--- Protocol self-test ---"
  if command -v vpn-admin >/dev/null; then
    if OUT="$(vpn-admin doctor --protocol 2>&1)"; then
      echo "FACT: vpn-admin doctor --protocol exited 0."
    else
      echo "FACT: vpn-admin doctor --protocol exited non-zero."
    fi
    echo "$OUT" | grep -Ei 'reality|hysteria|PASS|FAIL|INCONCLUSIVE' | sed 's/^/FACT (doctor output): /' || true
    echo "INFERENCE: this proves the server's own REALITY/Hysteria2 stack is (or is not)"
    echo "  self-consistent from THIS host, dialing itself. It does NOT prove any"
    echo "  specific remote client (including $ip) can complete the same handshake —"
    echo "  see docs/INCIDENT_2026-08-11_REALITY_HANDSHAKE_INCONCLUSIVE.md."
  else
    echo "UNKNOWN: vpn-admin not found on PATH — protocol self-test not run."
  fi
  echo

  echo "--- Listener presence (TCP/443, UDP/443) ---"
  if command -v ss >/dev/null; then
    local tcp_listen udp_listen
    tcp_listen="$(ss -H -tln 'sport = :443' 2>/dev/null || true)"
    udp_listen="$(ss -H -uln 'sport = :443' 2>/dev/null || true)"
    if [[ -n "$tcp_listen" ]]; then
      echo "FACT: a TCP listener is bound on :443 on this host."
      echo "$tcp_listen" | sed 's/^/FACT (ss): /'
    else
      echo "FACT: no TCP listener observed bound on :443 on this host."
    fi
    if [[ -n "$udp_listen" ]]; then
      echo "FACT: a UDP listener is bound on :443 on this host."
      echo "$udp_listen" | sed 's/^/FACT (ss): /'
    else
      echo "FACT: no UDP listener observed bound on :443 on this host."
    fi
  else
    echo "UNKNOWN: ss not found — listener presence not checked."
  fi
  echo

  echo "--- Recent sing-box log entries mentioning $ip ---"
  if command -v journalctl >/dev/null; then
    local matches
    matches="$(journalctl -u sing-box --since '2 hours ago' --no-pager 2>/dev/null | grep -F "$ip" | tail -50 || true)"
    if [[ -n "$matches" ]]; then
      echo "FACT: the following journal lines (last 2h, most recent 50) mention $ip:"
      echo "$matches" | sed 's/^/FACT (journal): /'
    else
      echo "FACT: no journal lines in the last 2h mention $ip (does not prove no attempt"
      echo "  occurred — sing-box logging may not include the client IP for every event"
      echo "  type, or the attempt may be older than 2h)."
    fi
  else
    echo "UNKNOWN: journalctl not found — recent log entries not checked."
  fi
  echo

  echo "--- REALITY/Hysteria2 event counts (last 2h, host-wide, not scoped to $ip) ---"
  if command -v journalctl >/dev/null; then
    local invalid accepted reset
    invalid="$(journalctl -u sing-box --since '2 hours ago' --no-pager 2>/dev/null | grep -Fc 'processed invalid connection' || true)"
    accepted="$(journalctl -u sing-box --since '2 hours ago' --no-pager 2>/dev/null | grep -Eic 'inbound/(vless|hysteria2).*(accepted|connection from)' || true)"
    reset="$(journalctl -u sing-box --since '2 hours ago' --no-pager 2>/dev/null | grep -Fic 'connection reset' || true)"
    echo "FACT: REALITY 'processed invalid connection' occurrences: ${invalid:-0}"
    echo "FACT: inbound accepted/connection-from occurrences: ${accepted:-0}"
    echo "FACT: 'connection reset' occurrences: ${reset:-0}"
    echo "INFERENCE: 'processed invalid connection' means the SERVER's REALITY layer"
    echo "  could not complete the hijack for that connection and transparently"
    echo "  proxied it to the decoy handshake target instead — it does NOT by itself"
    echo "  distinguish an unauthenticated probe, a stale/mismatched client credential,"
    echo "  DPI interference, or a client-side TLS/REALITY implementation difference."
    echo "  See docs/INCIDENT_2026-08-10_REALITY_HANDSHAKE_TIMEOUT.md and"
    echo "  docs/INCIDENT_2026-08-11_REALITY_HANDSHAKE_INCONCLUSIVE.md."
  else
    echo "UNKNOWN: journalctl not found — event counts not computed."
  fi
  echo

  echo "--- Suggested bounded capture (NOT run automatically) ---"
  echo "INFERENCE: to capture only this client's TCP/443 and UDP/443 packets"
  echo "  (metadata via 'summarize', never payload beyond 160 bytes), an operator"
  echo "  can run, for up to 300s:"
  echo "    sudo $0 capture $ip /root/${ip//[:.]/_}.pcap 120"
  echo "    sudo $0 summarize /root/${ip//[:.]/_}.pcap"
  echo

  echo "--- Firewall state (read-only) ---"
  if command -v firewall-cmd >/dev/null && firewall-cmd --state >/dev/null 2>&1; then
    echo "FACT: firewalld is active."
    firewall-cmd --list-ports 2>/dev/null | sed 's/^/FACT (firewalld ports): /' || true
  elif command -v nft >/dev/null; then
    if nft list ruleset >/dev/null 2>&1; then
      echo "FACT: nftables ruleset is present (not printed in full here to avoid noise —"
      echo "  run 'nft list ruleset' directly on this host for the full table)."
    else
      echo "UNKNOWN: nftables present but ruleset could not be read (permissions?)."
    fi
  elif command -v iptables >/dev/null; then
    echo "FACT: iptables is present."
    iptables -S INPUT 2>/dev/null | grep -E '443' | sed 's/^/FACT (iptables INPUT rule): /' || \
      echo "FACT: no iptables INPUT rule explicitly mentions port 443 (may still be allowed by a default policy or a rule matching by other criteria)."
  else
    echo "UNKNOWN: no recognized firewall tool (firewalld/nft/iptables) found."
  fi
  echo

  echo "======================================================================"
  echo "This investigation characterizes THIS SERVER's own state and logs only."
  echo "It cannot observe the client device's TUN state, DNS, routing, or"
  echo "application behavior — see docs/RUSSIA_PRODUCTION_INVESTIGATION.md's"
  echo "evidence-boundary conventions before drawing a conclusion from it alone."
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
  echo "singbox-vpn sustained-flow diagnostics (~${seconds}s TCP window, ~${seconds}s UDP window)"
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
  local family payload

  if [[ -n "$resolved_v4" ]]; then
    echo "IPv4 ($resolved_v4):"
    for payload in "${sizes[@]}"; do
      # "-M do" is ping's own flag value ("Do not fragment"), not a shell `do` keyword.
      # shellcheck disable=SC1010
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
      # "-M do" is ping's own flag value, not a shell `do` keyword.
      # shellcheck disable=SC1010
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

# ---------------------------------------------------------------------------
# tiktok — TikTok reachability diagnosis, structured the same way as
# `youtube` above but split into control-plane (www.tiktok.com,
# tiktok.com) vs. CDN/media (tiktokcdn.com, tiktokv.com) root domains —
# see docs/TIKTOK_INVESTIGATION.md hypothesis E ("TikTok API works but
# video CDN fails is a different problem from TikTok cannot establish any
# connection"). Root domains only, deliberately: TikTok's real app/web
# traffic uses many rotating regional subdomains (v16-webapp-prime,
# p16-sign-va, api16-normal-c-useast1a, and similar) that cannot be
# enumerated reliably without a live capture from an actual TikTok
# client — see docs/TIKTOK_INVESTIGATION.md's "why not hard-code more
# domains" note. A PASS here only proves this host's own network path to
# TikTok's root infrastructure is not blocked at the DNS/TCP/QUIC level;
# it does NOT verify the TikTok app, Hiddify, or any client-side
# behavior, and it does NOT determine whether TikTok's own Russia
# service policy (independent of network reachability — see
# docs/TIKTOK_INVESTIGATION.md hypothesis G) applies.
# ---------------------------------------------------------------------------

TIKTOK_CONTROL_DOMAINS=(www.tiktok.com tiktok.com)
TIKTOK_MEDIA_DOMAINS=(tiktokcdn.com tiktokv.com)

tiktok_dns() {
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

tiktok_tcp() {
  local host=$1 family=$2
  command -v curl >/dev/null || return
  # Transport-level probe only, same convention as youtube_tcp: TikTok's
  # bare-domain HTTP response code is not evaluated, only whether the TCP
  # connection and TLS handshake completed at all.
  if curl -s"${family}" --output /dev/null --connect-timeout 5 --max-time 10 \
      -w '' "https://${host}/" >/dev/null 2>&1; then
    echo "[OK]   $host IPv${family}/TCP/443+TLS: PASS (connection + TLS handshake completed; HTTP status is not evaluated)"
  else
    local rc=$?
    echo "[WARN] $host IPv${family}/TCP/443+TLS: FAIL (curl exit $rc — connection or TLS handshake did not complete)"
  fi
}

tiktok_quic() {
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

tiktok() {
  echo "TikTok endpoint investigation (server-side network diagnosis only)"
  echo "This does NOT attempt to bypass any restriction, and does NOT verify the TikTok app,"
  echo "Hiddify, or iOS/Android — it only characterizes this host's own network path to a small"
  echo "set of TikTok root domains, dialed directly (not through this deployment's own tunnel)."
  echo "See docs/TIKTOK_INVESTIGATION.md for how to interpret this alongside a real-device test."
  echo
  echo "--- Control-plane domains (web/API) ---"
  local host
  for host in "${TIKTOK_CONTROL_DOMAINS[@]}"; do
    tiktok_dns "$host"
  done
  echo
  for host in "${TIKTOK_CONTROL_DOMAINS[@]}"; do
    tiktok_tcp "$host" 4
    tiktok_tcp "$host" 6
  done
  echo
  echo "--- CDN/media domains (image/video delivery) ---"
  for host in "${TIKTOK_MEDIA_DOMAINS[@]}"; do
    tiktok_dns "$host"
  done
  echo
  for host in "${TIKTOK_MEDIA_DOMAINS[@]}"; do
    tiktok_tcp "$host" 4
    tiktok_tcp "$host" 6
  done
  echo
  for host in "${TIKTOK_MEDIA_DOMAINS[@]}"; do
    tiktok_quic "$host"
  done
  echo
  echo "Server-side connectivity is characterized above, split control-plane vs. CDN/media. This"
  echo "does NOT verify the TikTok app, Hiddify's TUN/routing behavior, or anything on the client"
  echo "device — only this host's own network path to these root domains. It also does NOT rule"
  echo "out TikTok's own Russia service policy applying independently of network reachability —"
  echo "see docs/TIKTOK_INVESTIGATION.md hypothesis G."
}

case ${1:-} in
  target) shift; target "$@" ;;
  capture) shift; capture "$@" ;;
  udp-egress-capture) shift; udp_egress_capture "$@" ;;
  summarize) shift; summarize "$@" ;;
  udp-egress-verdict) shift; udp_egress_verdict "$@" ;;
  streaming) shift; streaming "$@" ;;
  mtu) shift; mtu "$@" ;;
  youtube) shift; youtube "$@" ;;
  tiktok) shift; tiktok "$@" ;;
  client) shift; client "$@" ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
