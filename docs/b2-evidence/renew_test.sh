# Renewal keeps an open connection alive across the ORIGINAL valid_until,
# with zero sing-box restarts. All 4 slots are leased so no other rotation
# (e.g. of an unleased slot) can occur in the window.
set -u; . /root/b2/lib.sh
T=$(date +%s); PID0=$(sbpid)
ctl '{"lease":[0,1,2,3]}'; sleep 5
echo "$(ts) all slots leased: $(state)"
./client.py snap 1
VU=$(python3 -c 'import json,datetime;v=json.load(open("/root/b2/snap-1.json"))["valid_until"];print(int(datetime.datetime.fromisoformat(v.replace("Z","+00:00")).timestamp()))')
echo "$(ts) slot1 original valid_until=$(date -u -d @$VU +%T)"
until_ts $((VU-360))
python3 stream.py snap 1 10820 600 &
S=$!
until_ts $((VU-180))
NEW=$(( ( ($(date +%s)+900) / 120) * 120 ))
echo "$(ts) client renews (same route): control plane sets extend_to=$(date -u -d @$NEW +%T) on all 4 leased slots"
ctl "{\"extend\":{\"0\":\"$(date -u -d @$NEW +%FT%TZ)\",\"1\":\"$(date -u -d @$NEW +%FT%TZ)\",\"2\":\"$(date -u -d @$NEW +%FT%TZ)\",\"3\":\"$(date -u -d @$NEW +%FT%TZ)\"}}"
sleep 10
agentlog_since $((VU-200))
echo "$(ts) state after adoption: $(state)"
until_ts $((VU+30))
echo "--- 30 s after the ORIGINAL valid_until, same credential, new connections:"
./client.py test vless snap 1
./client.py test hy2 snap 1
wait $S
echo "$(ts) sing-box restarts since test start: $(restarts_since $T); MainPID before=$PID0 after=$(sbpid)"
agentlog_since $T | grep -c rotating | sed 's/^/rotations logged: /'
