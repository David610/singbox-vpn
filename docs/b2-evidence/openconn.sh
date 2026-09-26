set -u
cd /root/b2
ts(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
cat > /root/b2/drip.py <<'P'
import json, os, subprocess, sys, time, tempfile
slot, port = sys.argv[1], sys.argv[2]
S=json.load(open("/root/b2/secrets.json")); c=S[slot]
pub=open("/etc/vpn/compat/reality/public.key").read().strip(); sid=open("/etc/vpn/compat/reality/short_id.txt").read().strip()
cfg={"log":{"level":"error"},"inbounds":[{"type":"socks","listen":"127.0.0.1","listen_port":int(port)}],
 "outbounds":[{"type":"vless","server":"62.238.46.190","server_port":443,"uuid":c["vless_uuid"],"flow":"xtls-rprx-vision",
 "tls":{"enabled":True,"server_name":"www.cloudflare.com","utls":{"enabled":True,"fingerprint":"chrome"},"reality":{"enabled":True,"public_key":pub,"short_id":sid}}}]}
d=tempfile.mkdtemp(); os.chmod(d,0o700); p=d+"/c.json"; fd=os.open(p,os.O_WRONLY|os.O_CREAT,0o600); os.write(fd,json.dumps(cfg).encode()); os.close(fd)
pr=subprocess.Popen(["/usr/local/bin/sing-box","run","-c",p],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); time.sleep(1.5)
t0=time.time()
# server-paced stream: 40 bytes over 40s, cannot be pre-buffered
r=subprocess.run(["curl","-s","-o","/dev/null","-w","%{http_code} %{size_download}B","--max-time","60","--socks5-hostname","127.0.0.1:"+port,"https://httpbin.org/drip?duration=40&numbytes=40&code=200"],capture_output=True,text=True)
pr.terminate(); os.remove(p); os.rmdir(d)
print(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())} OPEN-CONNECTION slot{slot} (server-paced 40B/40s) started {time.strftime('%H:%M:%S',time.gmtime(t0))} ended after {time.time()-t0:.1f}s: curl exit={r.returncode} got={r.stdout}")
P
python3 /root/b2/drip.py 2 10810 &
A=$!
python3 /root/b2/drip.py 3 10811 &
B=$!
sleep 10
echo "$(ts) leasing+revoking slot 3 (slot 2 untouched) -> agent rotates slot 3 -> one sing-box restart"
python3 -c 'import json; json.dump({"lease":[3]}, open("/root/b2/control.json","w"))'; sleep 4
python3 -c 'import json; json.dump({"revoke":[3]}, open("/root/b2/control.json","w"))'
wait $A $B
echo "$(ts) sing-box restarts in window: $(journalctl -u sing-box --since '-80s' --no-pager -o cat | grep -c 'Started')"
