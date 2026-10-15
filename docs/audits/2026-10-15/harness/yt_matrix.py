"""Run the project's Shorts probe: direct control, then through the REALITY tunnel in default / IPv4-literal / IPv6-literal modes."""
import json
import os
import subprocess
import sys
import tempfile
import time

SP = os.path.dirname(os.path.abspath(__file__))
SINGBOX = os.path.join(SP, "sbclient", "sing-box-1.14.1-windows-amd64", "sing-box.exe")
PROBE = sys.argv[2]
PORT = 17940
COUNT = sys.argv[3] if len(sys.argv) > 3 else "6"


def run_probe(label: str, extra: list[str]) -> dict:
    t0 = time.time()
    p = subprocess.run([sys.executable, "-X", "utf8", PROBE, "--count", COUNT, *extra], capture_output=True, text=True, timeout=900)
    lines = [ln for ln in (p.stdout + p.stderr).splitlines() if ln.strip()]
    return {"label": label, "exit": p.returncode, "seconds": round(time.time() - t0), "tail": lines[-9:]}


doc = json.load(open(sys.argv[1]))
r = [o for o in doc["outbounds"] if o.get("type") == "vless"][0]
cfg = {"log": {"level": "error"}, "dns": {"servers": [{"tag": "local", "type": "udp", "server": "1.1.1.1"}, {"tag": "remote", "type": "udp", "server": "1.1.1.1", "detour": r["tag"]}], "final": "remote"},
       "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": PORT}], "outbounds": [r, {"type": "direct", "tag": "direct"}],
       "route": {"final": r["tag"], "default_domain_resolver": {"server": "local"}}}
td = tempfile.mkdtemp()
cp = os.path.join(td, "c.json")
json.dump(cfg, open(cp, "w"))
results = []
client = subprocess.Popen([SINGBOX, "run", "-c", cp], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    time.sleep(4)
    px = ["--proxy", f"socks5://127.0.0.1:{PORT}"]
    results.append(run_probe("via REALITY tunnel (default family)", px))
    results.append(run_probe("via REALITY tunnel, IPv4 literals only", px + ["--literal-family", "4"]))
    results.append(run_probe("via REALITY tunnel, IPv6 literals only", px + ["--literal-family", "6"]))
finally:
    client.terminate()
json.dump(results, open(sys.argv[4] if len(sys.argv) > 4 else os.path.join(SP, "evidence", "youtube-matrix.json"), "w"), indent=1)
for res in results:
    print(f"\n### {res['label']}  (exit {res['exit']}, {res['seconds']}s)")
    print("\n".join(res["tail"]))
