#!/usr/bin/env python3
"""usage: stream.py (current|snap) SLOT PORT SECONDS -- one server-paced VLESS
stream through the real server; prints only the outcome, never a credential."""
import json, os, subprocess, sys, time, tempfile
kind, slot, port, secs = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
c = json.load(open(f"/root/b2/snap-{slot}.json")) if kind == "snap" else json.load(open("/root/b2/secrets.json"))[slot]
pub = open("/etc/vpn/compat/reality/public.key").read().strip(); sid = open("/etc/vpn/compat/reality/short_id.txt").read().strip()
cfg = {"log": {"level": "error"}, "inbounds": [{"type": "socks", "listen": "127.0.0.1", "listen_port": int(port)}],
       "outbounds": [{"type": "vless", "server": "62.238.46.190", "server_port": 443, "uuid": c["vless_uuid"], "flow": "xtls-rprx-vision",
       "tls": {"enabled": True, "server_name": "www.cloudflare.com", "utls": {"enabled": True, "fingerprint": "chrome"},
               "reality": {"enabled": True, "public_key": pub, "short_id": sid}}}]}
d = tempfile.mkdtemp(); os.chmod(d, 0o700); p = d + "/c.json"
fd = os.open(p, os.O_WRONLY | os.O_CREAT, 0o600); os.write(fd, json.dumps(cfg).encode()); os.close(fd)
pr = subprocess.Popen(["/usr/local/bin/sing-box", "run", "-c", p], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); time.sleep(1.5)
t0 = time.time()
r = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code} %{size_download}B", "--max-time", str(secs + 30),
                    "--socks5-hostname", "127.0.0.1:" + port, f"http://62.238.46.190:9555/drip?n={secs}"], capture_output=True, text=True)
pr.terminate(); os.remove(p); os.rmdir(d)
ok = r.returncode == 0 and r.stdout.endswith(f"{secs}B")
print(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} OPEN-CONNECTION {kind}:slot{slot} ({secs}B at 1B/s) "
      f"started {time.strftime('%H:%M:%S', time.gmtime(t0))} ended after {time.time()-t0:.1f}s: curl exit={r.returncode} got={r.stdout} "
      f"-> {'SURVIVED' if ok else 'CUT'}")
