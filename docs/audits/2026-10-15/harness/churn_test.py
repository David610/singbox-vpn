"""Does a user-management operation on the server disturb OTHER customers' live sessions?

Holds long-lived REALITY connections for one customer (qa-baseline) through the VPS, then runs
server-side operations against a DIFFERENT throwaway user, recording for each operation:
sing-box MainPID/start time before and after (a restart changes them) and every failed probe.

usage: churn_test.py <native-sub.json> <results.json> <label>
"""
import json
import os
import subprocess
import sys
import tempfile
import threading
import time

SP = os.path.dirname(os.path.abspath(__file__))
SINGBOX = os.path.join(SP, "sbclient", "sing-box-1.14.1-windows-amd64", "sing-box.exe")
SSH = r"C:\WINDOWS\System32\OpenSSH\ssh.exe"
HOME = os.path.expanduser("~")
PORT = 17893
BIG = "http://91.244.71.165:9555/"


def ssh(cmd: str, timeout: int = 90) -> tuple[int, str]:
    p = subprocess.run([SSH, "-i", f"{HOME}/.ssh/arcana_vps_ed25519", "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes",
                        "-o", f"UserKnownHostsFile={HOME}/.ssh/arcana_vps_known_hosts", "-o", "StrictHostKeyChecking=yes",
                        "root@91.244.71.165", cmd], capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout if p.returncode == 0 else p.stdout + p.stderr).strip()


def server_state() -> dict:
    rc, out = ssh("systemctl show sing-box -p MainPID -p ActiveEnterTimestampMonotonic --value | tr '\\n' ' '")
    pid, mono, *_ = (out.split() + ["?", "?"])
    return {"pid": pid, "started_mono": mono}


def probe(timeout: int = 4) -> tuple[bool, float]:
    t0 = time.time()
    p = subprocess.run(["curl.exe", "-sS", "--max-time", str(timeout), "-x", f"socks5h://127.0.0.1:{PORT}", "-o", os.devnull,
                        "-w", "%{http_code}", "https://www.gstatic.com/generate_204"], capture_output=True, text=True)
    return p.stdout.strip() == "204", round(time.time() - t0, 3)


def main() -> int:
    doc = json.load(open(sys.argv[1]))
    label = sys.argv[3]
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
    result: dict = {"label": label, "ops": [], "transfers": []}
    try:
        for _ in range(30):
            if probe()[0]:
                break
            time.sleep(1)
        else:
            result["error"] = "client never connected"
            return 2

        def start_transfers(n: int = 2):
            procs = []
            for i in range(n):
                out = os.path.join(td, f"dl-{time.time_ns()}-{i}.bin")
                procs.append(subprocess.Popen(["curl.exe", "-sS", "--max-time", "90", "-x", f"socks5h://127.0.0.1:{PORT}",
                                               "-o", out, "-w", "%{http_code} %{size_download}", BIG],
                                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
            return procs

        timeline: list[dict] = []
        stop = threading.Event()

        def prober() -> None:
            while not stop.is_set():
                ok, dt = probe()
                timeline.append({"t": round(time.time() - start, 1), "ok": ok, "s": dt})
                time.sleep(0.5)

        start = time.time()
        threading.Thread(target=prober, daemon=True).start()
        time.sleep(8)

        uid = None
        ops = [
            ("create user (other customer)", "vpn user create --name churn-a --json"),
            ("rotate-token other user", "vpn user rotate-token {uid} >/dev/null"),
            ("disable other user", "vpn user disable {uid}"),
            ("enable other user", "vpn user enable {uid}"),
            ("set-expiry other user", "vpn user set-expiry --expires-at $(( $(date +%s) + 86400 )) {uid}"),
            ("rotate-vless other user", "vpn user rotate-vless {uid}"),
            ("rotate-hysteria other user", "vpn user rotate-hysteria {uid}"),
            ("renew-native other user", "vpn user renew-native {uid}"),
            ("doctor --protocol (creates a throwaway user)", "vpn doctor --protocol >/dev/null 2>&1; true"),
            ("remove other user", "vpn user remove {uid}"),
        ]
        for name, cmd in ops:
            running = start_transfers()
            time.sleep(3)
            before = server_state()
            t0 = time.time()
            rc, out = ssh(cmd.format(uid=uid))
            if uid is None and rc == 0:
                try:
                    uid = json.loads(out)["id"]
                except (ValueError, KeyError):
                    pass
            dur = round(time.time() - t0, 2)
            after = server_state()
            survived = []
            for pr in running:
                try:
                    so, se = pr.communicate(timeout=60)
                except subprocess.TimeoutExpired:
                    pr.kill()
                    so, se = pr.communicate()
                survived.append(pr.returncode == 0 and so.strip().endswith(" 50"))
            result["ops"].append({"op": name, "rc": rc, "cmd_seconds": dur, "t_start": round(t0 - start, 1), "inflight_transfers_survived": survived,
                                  "pid_before": before["pid"], "pid_after": after["pid"],
                                  "restarted": before["pid"] != after["pid"], "note": out[-120:] if rc else ""})
        time.sleep(4)
        stop.set()
        time.sleep(1)
        result["probe_total"] = len(timeline)
        result["probe_failures"] = [p for p in timeline if not p["ok"]]
        result["probe_max_latency_s"] = max(p["s"] for p in timeline)
    finally:
        client.terminate()
        json.dump(result, open(sys.argv[2], "w"), indent=1)
    print(json.dumps(result, indent=1)[:5000])
    return 0


sys.exit(main())
