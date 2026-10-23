#!/usr/bin/env bash
# Synchronized fin1+ru1 header-only capture for ONE native YouTube playback attempt.
# usage: sync_capture.sh SECONDS [outdir]   (run on fin1 as root; needs the ru1 key)
# Captures only headers (snaplen 128). ru1 side: its own egress/ingress to servers only
# (UDP/443 and TCP/443 to/from Google), plus fin1<->ru1 hairpin TCP. No other users' client traffic.
set -u
SECS=${1:-150}; OUT=${2:-./sync-capture-$(date -u +%Y%m%dT%H%M%SZ)}
: "${RELAY_IP:?set RELAY_IP}" "${ENTRY_IP:?set ENTRY_IP}" "${RELAY_SSH_KEY:?set RELAY_SSH_KEY}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
K=$RELAY_SSH_KEY; R=root@$RELAY_IP; SSHO="-i $K -o IdentitiesOnly=yes -o BatchMode=yes"
RU=$RELAY_IP; FIN=$ENTRY_IP; PHONE_FILTER="tcp port 443 and not host $RU"
mkdir -p "$OUT"; chmod 700 "$OUT"
ssh $SSHO $R 'mkdir -p /root/capture' || exit 1
RUF="(udp and src host $RU and dst port 443) or (udp and dst host $RU and src port 443) or (tcp and src host $RU and dst port 443 and not dst host $FIN) or (tcp and dst host $RU and src port 443 and not src host $FIN) or (tcp and host $FIN and host $RU)"
ssh $SSHO $R "rm -f /root/capture/ru1_sync.pcap; (timeout $((SECS+5)) tcpdump -ni eth0 -s 128 -w /root/capture/ru1_sync.pcap '$RUF' >/dev/null 2>&1 &)"
(timeout $((SECS+5)) tcpdump -ni eth0 -s 128 -w "$OUT/fin1_hairpin.pcap" host $RU >/dev/null 2>&1 &)
(timeout $((SECS+5)) tcpdump -ni eth0 -s 128 -w "$OUT/fin1_phoneleg.pcap" "$PHONE_FILTER" >/dev/null 2>&1 &)
( for _ in $(seq 1 "$SECS"); do date -u +%T.%N; ss -tin state established sport = :443 2>/dev/null | grep -v $RU | sed -E 's/\s+/ /g'; echo; sleep 1; done > "$OUT/fin1_phone_ss.log" 2>&1 & )
sleep 2; date -u +%s.%N > "$OUT/GO_epoch"; echo "GO $(date -u +%T) UTC  (capture runs ${SECS}s -> $OUT)"
sleep $((SECS+6))
scp -q $SSHO $R:/root/capture/ru1_sync.pcap "$OUT/ru1_sync.pcap" && ssh $SSHO $R 'rm -f /root/capture/ru1_sync.pcap; rmdir /root/capture 2>/dev/null'
chmod 600 "$OUT"/*.pcap; (cd "$OUT" && sha256sum * > SHA256SUMS)
echo "== fin1 hairpin =="; python3 -I $HERE/pcap_analyze.py "$OUT/fin1_hairpin.pcap" | sed -n 1,9p
python3 -I $HERE/stuck_classify.py "$OUT/fin1_hairpin.pcap" | head -9
echo "== ru1 (egress to Google) =="; python3 -I $HERE/relay_summary.py "$OUT/ru1_sync.pcap"
echo "== phone leg (fin1 side) =="; tcpdump -nr "$OUT/fin1_phoneleg.pcap" 2>/dev/null | wc -l | sed 's/^/packets: /'
echo "DONE -> $OUT"
