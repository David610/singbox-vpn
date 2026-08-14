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

target validates a REALITY handshake candidate from this host over IPv4 and
IPv6. capture records only CLIENT_IP and TCP/443 or UDP/443 for at most 300s.
summarize reports packet metadata only; it never prints payload or secrets.
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

case ${1:-} in
  target) shift; target "$@" ;;
  capture) shift; capture "$@" ;;
  summarize) shift; summarize "$@" ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
