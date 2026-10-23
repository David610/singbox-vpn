#!/usr/bin/env bash
# Qualify a candidate Google-egress node BEFORE installing any VPN stack on it.
# usage (on fin1, root): candidate_qualify.sh SSH_TARGET LABEL [SSH_KEY] [--sustained VIDEO_ID]
# Needs only python3 + ssh on the candidate. Nothing but test scripts (removed afterwards) is placed there.
# Stages: (1) TLS/SNI survey over 121 Google IPs x 7 SNIs, v4 + v6 reachability, QUIC version-neg probe
#         (2) browser media probe from fin1 THROUGH the candidate via plain ssh -D (no VPN protocol):
#             9 consecutive Shorts, then optional 900 s sustained long-form playback
# Verdict is machine-readable in $OUT/verdict.json. A candidate that passes handshakes but cannot play is REJECTED.
set -u
T=$1; L=$2; K=${3:-}; SUST=${5:-}
OUT=${EVIDENCE_DIR:-./egress-qualification}/$L-$(date -u +%Y%m%dT%H%M%SZ); mkdir -p "$OUT"; chmod 700 "$OUT"
SSHO="-o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10"; [ -n "$K" ] && SSHO="$SSHO -i $K -o IdentitiesOnly=yes"
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; PORT=18200
PROBE_HOME=${PROBE_HOME:-/home/ytprobe}
echo "== stage 1: network checks on $T"
ssh $SSHO $T 'echo reachable; python3 --version; curl -s -m6 -4 https://api.ipify.org; echo; curl -s -m6 -6 https://api64.ipify.org; echo' | tee "$OUT/host.txt"
python3 -I -c "import json;print(len(json.load(open('/tmp/ips_union.json'))),'IPs')" 2>/dev/null || echo "need /tmp/ips_union.json (collect_ips.py on fin1+candidate)"
scp -q $SSHO $A/tls_survey.py $A/quic_vn_probe.py $A/v6_check.py /tmp/ips_union.json $T:/tmp/
ssh $SSHO $T 'python3 -I /tmp/tls_survey.py /tmp/ips_union.json cand; echo; echo "[IPv6]"; python3 -I /tmp/v6_check.py; echo; echo "[QUIC version-neg]"; for h in $(python3 -I -c "import socket;print(*sorted({a[4][0] for a in socket.getaddrinfo(\"www.youtube.com\",443,socket.AF_INET)})[:3])"); do python3 -I /tmp/quic_vn_probe.py $h; done; cat /tmp/tls_survey_cand.json' > "$OUT/stage1.raw" 2>&1
head -n -1 "$OUT/stage1.raw" | tee "$OUT/stage1.txt"; tail -n 1 "$OUT/stage1.raw" > "$OUT/tls_survey.json"
ssh $SSHO $T 'rm -f /tmp/tls_survey.py /tmp/quic_vn_probe.py /tmp/v6_check.py /tmp/ips_union.json /tmp/tls_survey_cand.json'
echo "== stage 2: browser media probe through $T (plain ssh -D, TCP only)"
ssh $SSHO -D 127.0.0.1:$PORT -N $T &
SSH_PID=$!   # tracked PID, killed by exact PID below (see test-no-broad-process-kills.sh)
sleep 2
IDS=sAcElROnYIE,0HAETg7L15o,0uWzj4AiiZ8,0hkt2-EwMl4,5mU6SRS2Bxo,3II9AFi5U0M,147Jql-pxqk,1yfCBYdjLHU,19IX8zSEJXg
su - ytprobe -c "PLAYWRIGHT_BROWSERS_PATH=\$HOME/pw-browsers timeout 600 ~/venv/bin/python ~/shorts_media_probe.py --proxy socks5://127.0.0.1:$PORT --ids $IDS --dwell 25 --label cand_$L" | tee "$OUT/stage2_shorts.txt"
cp $PROBE_HOME/result_cand_$L.json "$OUT/" 2>/dev/null
if [ "${4:-}" = "--sustained" ] && [ -n "$SUST" ]; then
  su - ytprobe -c "PLAYWRIGHT_BROWSERS_PATH=\$HOME/pw-browsers timeout 1000 ~/venv/bin/python ~/shorts_media_probe.py --proxy socks5://127.0.0.1:$PORT --ids watch:$SUST --dwell 900 --label cand_${L}_sustained" | tee "$OUT/stage3_sustained.txt"
fi
kill "$SSH_PID" 2>/dev/null || true
python3 -I - "$OUT" <<'PY'
import json,sys,glob,collections
o=sys.argv[1];v={'dir':o}
try:
    r=json.load(open(o+'/tls_survey.json'));yt=collections.Counter(x[2] for x in r if x[1] not in('www.google.com','example.com',''));v['tls_youtube_sni']=dict(yt)
except Exception as e:v['tls_youtube_sni']='n/a'
try:
    pr=json.load(open(glob.glob(o+'/result_cand_*.json')[0]));c=collections.Counter(x['verdict'] for x in pr);v['shorts']=dict(c)
    v['verdict']='QUALIFIED-FOR-NEXT-STAGE' if c.get('PLAYED',0)>=8 and c.get('FROZE',0)==0 else 'REJECTED'
except Exception as e:v['shorts']='n/a';v['verdict']='REJECTED'
json.dump(v,open(o+'/verdict.json','w'));print('VERDICT',json.dumps(v))
PY
