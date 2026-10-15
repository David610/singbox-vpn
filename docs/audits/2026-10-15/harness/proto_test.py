"""Run the real sing-box client against a subscription's outbounds, one tag at a time.

usage: proto_test.py <outbounds.json> <results.json> [tag ...]
Reports: client start, proxy egress IPv4/IPv6, DNS-through-proxy, HTTPS fetch, TTFB, and sing-box's own log tail on failure.
"""
import json
import os
import subprocess
import sys
import tempfile
import time

SP = os.path.dirname(os.path.abspath(__file__))
SINGBOX = os.path.join(SP, "sbclient", "sing-box-1.14.1-windows-amd64", "sing-box.exe")
PORT = 17890


def curl(args: list[str], timeout: int = 25) -> tuple[int, str, float]:
    t0 = time.time()
    p = subprocess.run(["curl.exe", "-sS", "--max-time", str(timeout), *args],
                       capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr).strip(), round(time.time() - t0, 3)


def run_tag(doc: dict, tag: str) -> dict:
    outs = [o for o in doc["outbounds"] if o.get("type") not in ("urltest", "selector", "direct")]
    chosen = [o for o in outs if o["tag"] == tag]
    if not chosen:
        return {"tag": tag, "error": "tag not found"}
    cfg = {
        "log": {"level": "info", "timestamp": True},
        "dns": {"servers": [{"tag": "local", "type": "udp", "server": "1.1.1.1"}, {"tag": "remote", "type": "udp", "server": "1.1.1.1", "detour": tag}], "final": "remote"},
        "inbounds": [{"type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": PORT}],
        "outbounds": chosen + [{"type": "direct", "tag": "direct"}],
        "route": {"final": tag, "default_domain_resolver": {"server": "local"}},
    }
    with tempfile.TemporaryDirectory() as td:
        cfgp = os.path.join(td, "c.json")
        logp = os.path.join(td, "log.txt")
        json.dump(cfg, open(cfgp, "w"))
        chk = subprocess.run([SINGBOX, "check", "-c", cfgp], capture_output=True, text=True)
        res: dict = {"tag": tag, "check_rc": chk.returncode, "check_out": (chk.stdout + chk.stderr).strip()[:300]}
        if chk.returncode:
            return res
        with open(logp, "w") as lf:
            proc = subprocess.Popen([SINGBOX, "run", "-c", cfgp], stdout=lf, stderr=subprocess.STDOUT)
        try:
            t0 = time.time()
            up = False
            while time.time() - t0 < 10:
                rc, out, _ = curl(["-x", f"socks5h://127.0.0.1:{PORT}", "-o", os.devnull, "-w", "%{http_code}",
                                   "https://www.gstatic.com/generate_204"], 12)
                if rc == 0 and out.strip() == "204":
                    up = True
                    break
                time.sleep(0.5)
            res["first_success_s"] = round(time.time() - t0, 2) if up else None
            res["generate_204"] = up
            px = ["-x", f"socks5h://127.0.0.1:{PORT}"]
            rc4, ip4, t4 = curl([*px, "-4", "https://api.ipify.org"])
            rc6, ip6, t6 = curl([*px, "https://api64.ipify.org"])
            res["egress_ipv4_probe"] = {"rc": rc4, "out": ip4[:60], "s": t4}
            res["egress_api64"] = {"rc": rc6, "out": ip6[:60], "s": t6}
            rcw, outw, tw = curl([*px, "-o", os.devnull, "-w", "%{http_code} ttfb=%{time_starttransfer} total=%{time_total}",
                                  "https://www.wikipedia.org/"])
            res["https_wikipedia"] = {"rc": rcw, "out": outw, "s": tw}
            rcd, outd, td_ = curl([*px, "-o", os.devnull, "-w", "%{http_code}", "https://one.one.one.one/"])
            res["dns_via_proxy_hostname_fetch"] = {"rc": rcd, "out": outd, "s": td_}
        finally:
            proc.terminate()
            try:
                proc.wait(5)
            except subprocess.TimeoutExpired:
                proc.kill()
            time.sleep(0.5)
            tail = open(logp, errors="replace").read().splitlines()[-12:]
            res["client_log_tail"] = [ln[:220] for ln in tail] if not res.get("generate_204") else "ok (suppressed)"
    return res


if __name__ == "__main__":
    doc = json.load(open(sys.argv[1]))
    tags = sys.argv[3:] or [o["tag"] for o in doc["outbounds"] if o.get("type") in ("vless", "hysteria2")]
    results = [run_tag(doc, t) for t in tags]
    json.dump(results, open(sys.argv[2], "w"), indent=1)
    for r in results:
        print(json.dumps(r, indent=1)[:2500])
