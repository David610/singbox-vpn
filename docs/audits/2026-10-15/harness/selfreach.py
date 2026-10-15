import json, os, subprocess, sys, tempfile, time
sp = sys.argv[1]
doc = json.load(open(sys.argv[2]))
r = [o for o in doc["outbounds"] if o["type"] == "vless"][0]
cfg = {"log": {"level": "error"}, "dns": {"servers": [{"tag": "local", "type": "udp", "server": "1.1.1.1"}], "final": "local"},
       "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": 17950}], "outbounds": [r, {"type": "direct", "tag": "direct"}],
       "route": {"final": r["tag"], "default_domain_resolver": {"server": "local"}}}
td = tempfile.mkdtemp(); cp = f"{td}/c.json"; json.dump(cfg, open(cp, "w"))
p = subprocess.Popen([f"{sp}/sbclient/sing-box-1.14.1-windows-amd64/sing-box.exe", "run", "-c", cp], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(4)
def c(url):
    q = subprocess.run(["curl.exe", "-sk", "--max-time", "8", "-x", "socks5h://127.0.0.1:17950", "-o", os.devnull, "-w", "%{http_code}", url], capture_output=True, text=True)
    return q.stdout.strip() + (" (rc %d)" % q.returncode)
print("tunnel -> node's own public nginx   https://91.244.71.165:8443/ :", c("https://91.244.71.165:8443/"))
print("tunnel -> sanity, public internet   https://www.gstatic.com/generate_204 :", c("https://www.gstatic.com/generate_204"))
p.terminate()
