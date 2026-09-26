set -u
cd /root/b2
exec >> /root/b2/expiry.out 2>&1
ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
./client.py snap 1
VU=$(python3 -c 'import json,datetime;v=json.load(open("/root/b2/snap-1.json"))["valid_until"];print(int(datetime.datetime.fromisoformat(v.replace("Z","+00:00")).timestamp()))')
echo "$(ts) slot1 gen1 is LEASED; lease/slot valid_until=$(date -u -d @$VU +%Y-%m-%dT%H:%M:%SZ)"
while [ $(date +%s) -lt $((VU-40)) ]; do sleep 5; done
echo "--- before expiry"
./client.py test vless snap 1
./client.py test hy2 snap 1
while [ $(date +%s) -lt $((VU+8)) ]; do sleep 1; done
echo "--- after expiry"
journalctl -u b2-agent --since "@$((VU-60))" --no-pager -o cat | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'rotating|applied live'
./client.py test vless snap 1
./client.py test hy2 snap 1
./client.py test vless current 1
echo "$(ts) state: $(python3 -c 'import json;s=json.load(open("/root/b2/state.json"));print({k:(v["generation"],v["state"]) for k,v in s.items()})')"
echo DONE
