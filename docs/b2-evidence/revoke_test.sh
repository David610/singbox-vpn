set -u
cd /root/b2
ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
./client.py snap 0
python3 - <<'P'
import json; json.dump({"lease":[0,1]}, open("/root/b2/control.json","w"))
P
sleep 5
echo "$(ts) mock control plane: slots 0,1 leased: $(python3 -c 'import json;s=json.load(open("/root/b2/state.json"));print({k:(v["generation"],v["state"]) for k,v in s.items()})')"
# long-lived connection through slot 1 (NOT revoked) to observe what a reload does to open connections
python3 - <<'P' &
import json, os, subprocess, time, tempfile
S=json.load(open("/root/b2/secrets.json")); c=S["1"]
pub=open("/etc/vpn/compat/reality/public.key").read().strip(); sid=open("/etc/vpn/compat/reality/short_id.txt").read().strip()
cfg={"log":{"level":"error"},"inbounds":[{"type":"socks","listen":"127.0.0.1","listen_port":10809}],
 "outbounds":[{"type":"vless","server":"62.238.46.190","server_port":443,"uuid":c["vless_uuid"],"flow":"xtls-rprx-vision",
 "tls":{"enabled":True,"server_name":"www.cloudflare.com","utls":{"enabled":True,"fingerprint":"chrome"},"reality":{"enabled":True,"public_key":pub,"short_id":sid}}}]}
d=tempfile.mkdtemp(); os.chmod(d,0o700); p=d+"/c.json"; fd=os.open(p,os.O_WRONLY|os.O_CREAT,0o600); os.write(fd,json.dumps(cfg).encode()); os.close(fd)
pr=subprocess.Popen(["/usr/local/bin/sing-box","run","-c",p],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); time.sleep(1.5)
t0=time.time()
r=subprocess.run(["curl","-s","-o","/dev/null","-w","%{http_code} %{size_download}B","--limit-rate","200k","--max-time","40","--socks5-hostname","127.0.0.1:10809","https://speed.cloudflare.com/__down?bytes=6000000"],capture_output=True,text=True)
pr.terminate(); os.remove(p); os.rmdir(d)
print(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())} OPEN-CONNECTION (slot1 vless, not revoked) started {time.strftime('%H:%M:%S',time.gmtime(t0))}: curl exit={r.returncode} result={r.stdout} (exit 0=survived, 18/56=cut)")
P
BG=$!
sleep 6
echo "$(ts) revoking slot 0 at control plane"
python3 -c 'import json; json.dump({"revoke":[0]}, open("/root/b2/control.json","w"))'
for i in $(seq 1 20); do
  g=$(python3 -c 'import json;print(json.load(open("/root/b2/secrets.json"))["0"]["generation"])')
  [ "$g" != "1" ] && break; sleep 1
done
echo "$(ts) agent rotated slot 0 -> generation $g; sing-box restarts: $(journalctl -u sing-box --since '-60s' --no-pager -o cat | grep -c 'Started sing-box')"
journalctl -u b2-agent --since '-60s' --no-pager -o cat | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'rotating|applied live' | tail -2
./client.py test vless snap 0
./client.py test hy2 snap 0
./client.py test vless current 0
./client.py test hy2 current 0
./client.py test vless current 1
wait $BG
echo "$(ts) state: $(python3 -c 'import json;s=json.load(open("/root/b2/state.json"));print({k:(v["generation"],v["state"]) for k,v in s.items()})')"
