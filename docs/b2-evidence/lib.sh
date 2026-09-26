# shared helpers for the B2 evidence scripts
cd /root/b2
ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
ctl(){ python3 -c "import json,sys; json.dump(json.loads(sys.argv[1]), open('/root/b2/control.json','w'))" "$1"; }
state(){ python3 -c 'import json;s=json.load(open("/root/b2/state.json"));print({k:(v["generation"],v["state"],v["valid_until"][11:19]) for k,v in s.items()})'; }
until_ts(){ while [ "$(date +%s)" -lt "$1" ]; do sleep 1; done; }
at(){ date -u -d "$1" +%s; }
sbpid(){ systemctl show -p MainPID --value sing-box; }
restarts_since(){ journalctl -u sing-box --since "@$1" --no-pager -o cat | grep -c '^Started sing-box'; }
agentlog_since(){ journalctl -u b2-agent --since "@$1" --no-pager -o cat | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'rotating|applied live|renewals adopted|starting|tick failed' | sed -E 's/^([0-9T:-]+)\.([0-9]{2})[0-9]*Z +INFO +[a-z_:]+: /\1.\2 /'; }
