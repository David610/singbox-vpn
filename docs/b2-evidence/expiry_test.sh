# Expiry with the control plane DOWN and after an agent restart: the expired
# credential must end REFUSED. Leases a fresh slot, stops the mock control
# plane, restarts the agent, waits past valid_until.
set -u; . /root/b2/lib.sh
exec >> /root/b2/expiry.out 2>&1
ctl '{"lease":[2]}'; sleep 5
./client.py snap 2
VU=$(python3 -c 'import json,datetime;v=json.load(open("/root/b2/snap-2.json"))["valid_until"];print(int(datetime.datetime.fromisoformat(v.replace("Z","+00:00")).timestamp()))')
echo "$(ts) slot2 is LEASED; valid_until (= expires_at)=$(date -u -d @$VU +%T)"
T=$(date +%s); PID0=$(sbpid)
systemctl stop b2-mockcp; echo "$(ts) mock control plane STOPPED"
systemctl restart b2-agent; echo "$(ts) agent RESTARTED"
sleep 10
agentlog_since $T | head -4
echo "$(ts) sing-box restarts caused by the agent restart: $(restarts_since $T) (MainPID $PID0 -> $(sbpid))"
until_ts $((VU-40))
echo "--- before expiry"
./client.py test vless snap 2
./client.py test hy2 snap 2
until_ts $((VU+8))
echo "--- after expiry (control plane still down)"
agentlog_since $((VU-5))
./client.py test vless snap 2
./client.py test hy2 snap 2
./client.py test vless current 2
echo "$(ts) sing-box restarts since agent restart: $(restarts_since $T)"
echo DONE
