# Honest deferral and urgent revocation: a non-urgent revocation right
# after a batch rotation keeps working until the next boundary; an urgent
# one rotates within seconds and takes the pending one along.
set -u; . /root/b2/lib.sh
# force a batch rotation at the next boundary (non-urgent revoke of slot 3),
# so the rotation window is "used" and the next non-urgent revoke is deferred
ctl '{"lease":[3],"revoke":[3]}'
B=$(( ( $(date +%s) / 120 + 1) * 120 )); until_ts $((B+8))
agentlog_since $((B-2)) | grep rotating
T=$(date +%s)
echo "$(ts) boundary $(date -u -d @$B +%T) passed; $(state)"
ctl '{"lease":[0,1]}'; sleep 5
./client.py snap 0; ./client.py snap 1
ctl '{"revoke":[0]}'
echo "$(ts) slot 0 revoked (NON-urgent); next boundary $(date -u -d @$((B+120)) +%T)"
sleep 15
./client.py test vless snap 0   # expected: still PASS (deferred to the batch; bounded by expires_at)
echo "$(ts) sing-box restarts since revoke: $(restarts_since $T)"
U=$(date +%s)
ctl '{"revoke_urgent":[1]}'
echo "$(ts) slot 1 revoked URGENT"
for i in $(seq 1 30); do
  g=$(python3 -c 'import json;print(json.load(open("/root/b2/secrets.json"))["1"]["generation"])')
  [ "$g" != "$(python3 -c 'import json;print(json.load(open("/root/b2/snap-1.json"))["generation"])')" ] && break; sleep 1
done
agentlog_since $U
echo "$(ts) urgent: sing-box restarts since urgent revoke: $(restarts_since $U) (revoke->new generation reported: $(( $(date +%s) - U ))s)"
./client.py test vless snap 1
./client.py test hy2 snap 1
./client.py test vless snap 0   # the pending non-urgent revocation rode along
./client.py test vless current 1
echo "$(ts) $(state)"
