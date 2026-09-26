#!/usr/bin/env python3
"""B2 real-test client. usage: client.py snap SLOT | client.py test (vless|hy2) (current SLOT|snap SLOT)
Builds a sing-box client config from the mock control plane's 0600 secret
store, connects through the real server, fetches a URL via the tunnel and
prints ONLY the outcome (HTTP code / failure) -- never a credential."""
import json, os, subprocess, sys, time, tempfile
D = "/root/b2"; S = json.load(open(f"{D}/secrets.json"))
def cred(kind, slot):
    if kind == "snap": return json.load(open(f"{D}/snap-{slot}.json"))
    return S[str(slot)]
if sys.argv[1] == "snap":
    fd = os.open(f"{D}/snap-{sys.argv[2]}.json", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f: json.dump(S[sys.argv[2]], f)
    print(f"snapshot slot {sys.argv[2]} generation {S[sys.argv[2]]['generation']}"); sys.exit()
_, _, proto, kind, slot = sys.argv
c = cred(kind, slot)
pub = open("/etc/vpn/compat/reality/public.key").read().strip()
sid = open("/etc/vpn/compat/reality/short_id.txt").read().strip()
if proto == "vless":
    out = {"type": "vless", "tag": "proxy", "server": "62.238.46.190", "server_port": 443, "uuid": c["vless_uuid"],
           "flow": "xtls-rprx-vision", "tls": {"enabled": True, "server_name": "www.cloudflare.com",
           "utls": {"enabled": True, "fingerprint": "chrome"}, "reality": {"enabled": True, "public_key": pub, "short_id": sid}}}
else:
    out = {"type": "hysteria2", "tag": "proxy", "server": "62.238.46.190", "server_port": 443, "password": c["hysteria2_password"],
           "obfs": {"type": "salamander", "password": S["_obfs"]},
           # test-only self-signed server cert (production uses the node's real cert)
           "tls": {"enabled": True, "server_name": "b2-test.invalid", "insecure": True}}
cfg = {"log": {"level": "error"}, "inbounds": [{"type": "socks", "listen": "127.0.0.1", "listen_port": 10808}],
       "outbounds": [out]}
tmp = tempfile.mkdtemp(prefix="b2c-"); os.chmod(tmp, 0o700); p = f"{tmp}/c.json"
fd = os.open(p, os.O_WRONLY | os.O_CREAT, 0o600); os.write(fd, json.dumps(cfg).encode()); os.close(fd)
proc = subprocess.Popen(["/usr/local/bin/sing-box", "run", "-c", p], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1.5)
r = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code} %{size_download}B %{time_total}s", "--max-time", "10",
                    "--socks5-hostname", "127.0.0.1:10808", "https://www.gstatic.com/generate_204"], capture_output=True, text=True)
proc.terminate(); proc.wait(); os.remove(p); os.rmdir(tmp)
code = r.stdout.split()[0] if r.stdout else "000"
print(f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} proto={proto} cred={kind}:slot{slot} gen={c['generation']} "
      f"result={'PASS traffic' if code == '204' else 'REFUSED/no traffic'} curl={r.stdout.strip() or 'none'} exit={r.returncode}")
