"""What does a data-plane restart do to a live REALITY download? Also measures recovery time.

usage: restart_probe.py <native-sub.json> <out.json> <label> [restart-cmd]
"""
import json
import os
import subprocess
import sys
import tempfile
import time

SP = os.path.dirname(os.path.abspath(__file__))
SINGBOX = os.path.join(SP, "sbclient", "sing-box-1.14.1-windows-amd64", "sing-box.exe")
SSH = r"C:\WINDOWS\System32\OpenSSH\ssh.exe"
HOME = os.path.expanduser("~")
PORT = 17894
URL = "http://91.244.71.165:9555/"


def ssh(cmd: str, timeout: int = 90) -> tuple[int, str]:
    p = subprocess.run([SSH, "-i", f"{HOME}/.ssh/arcana_vps_ed25519", "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes",
                        "-o", f"UserKnownHostsFile={HOME}/.ssh/arcana_vps_known_hosts", "-o", "StrictHostKeyChecking=yes",
                        "root@91.244.71.165", cmd], capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout + p.stderr).strip()


def probe(timeout: int = 3) -> bool:
    p = subprocess.run(["curl.exe", "-sS", "--max-time", str(timeout), "-x", f"socks5h://127.0.0.1:{PORT}", "-o", os.devnull,
                        "-w", "%{http_code}", "https://www.gstatic.com/generate_204"], capture_output=True, text=True)
    return p.stdout.strip() == "204"


doc = json.load(open(sys.argv[1]))
label = sys.argv[3]
restart_cmd = sys.argv[4] if len(sys.argv) > 4 else "systemctl restart sing-box"
reality = [o for o in doc["outbounds"] if o.get("type") == "vless"][0]
cfg = {"log": {"level": "warn"}, "dns": {"servers": [{"tag": "local", "type": "udp", "server": "1.1.1.1"},
       {"tag": "remote", "type": "udp", "server": "1.1.1.1", "detour": reality["tag"]}], "final": "remote"},
       "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": PORT}],
       "outbounds": [reality, {"type": "direct", "tag": "direct"}],
       "route": {"final": reality["tag"], "default_domain_resolver": {"server": "local"}}}
td = tempfile.mkdtemp()
cp = os.path.join(td, "c.json")
json.dump(cfg, open(cp, "w"))
client = subprocess.Popen([SINGBOX, "run", "-c", cp], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
res: dict = {"label": label, "restart_cmd": restart_cmd}
try:
    for _ in range(30):
        if probe():
            break
        time.sleep(1)
    dl = subprocess.Popen(["curl.exe", "-sS", "--max-time", "120", "-x", f"socks5h://127.0.0.1:{PORT}",
                           "-o", os.devnull, "-w", "%{http_code} %{size_download}", URL], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    time.sleep(5)
    t0 = time.time()
    rc, out = ssh(restart_cmd)
    res["restart_rc"] = rc
    res["restart_cmd_seconds"] = round(time.time() - t0, 2)
    # recovery: time from restart command start until a NEW connection works end to end
    rec = None
    while time.time() - t0 < 60:
        if probe(2):
            rec = round(time.time() - t0, 2)
            break
        time.sleep(0.25)
    res["new_connection_ok_after_s"] = rec
    try:
        so, se = dl.communicate(timeout=90)
    except subprocess.TimeoutExpired:
        dl.kill()
        so, se = dl.communicate()
    res["inflight_download"] = {"rc": dl.returncode, "out": so.strip(), "err": se.strip()[:140],
                                "completed": dl.returncode == 0 and so.strip().endswith(" 50")}
finally:
    client.terminate()
print(json.dumps(res, indent=1))
json.dump(res, open(sys.argv[2], "w"), indent=1)
