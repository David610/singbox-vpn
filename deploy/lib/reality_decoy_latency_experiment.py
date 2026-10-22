#!/usr/bin/env python3
"""Reproducible I-60 experiment: where does the per-new-connection REALITY latency come from?

Throwaway sing-box REALITY server/client pairs on loopback (nothing touches the production listeners or any other host), identical except for the
decoy (`handshake.server`). For each variant it measures N sequential NEW connections (one curl per request, no multiplexing) to a local HTTP
target, minus the same request made directly, and reads the server's own trace line `time.Since(start): X` (the time REALITY spent reading the
decoy's flight) to attribute the server-side share. Evidence class: server verified, loopback client; NOT a real client network.

  reality_decoy_latency_experiment.py --variant remote:www.microsoft.com --variant remote-ip:www.microsoft.com --variant local [--n 40]

Variants: remote:HOST (handshake.server=HOST), remote-ip:HOST (the same decoy with its address pinned, SNI unchanged; isolates DNS),
local (a TLS 1.3 server on 127.0.0.1 with a throwaway certificate; the lower bound).
"""
import argparse
import json
import os
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import time

SB = "/usr/local/bin/sing-box"


def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def wait_log(path, needle, t=10):
    end = time.time() + t
    while time.time() < end:
        try:
            if needle in open(path, errors="ignore").read(): return True
        except OSError: pass
        time.sleep(0.05)
    return False


def keypair():
    out = subprocess.run([SB, "generate", "reality-keypair"], capture_output=True, text=True, check=True).stdout
    return re.search(r"PrivateKey: (\S+)", out).group(1), re.search(r"PublicKey: (\S+)", out).group(1)


def run_variant(variant, n, d):
    kind, _, host = variant.partition(":")
    priv, pub = keypair()
    sid, uuid = "a1b2c3d4", "11111111-2222-4333-8444-555555555555"
    procs = []
    sni = host or "localhost"
    hs_server, hs_port = host, 443
    if kind == "remote-ip":
        hs_server = socket.gethostbyname(host)
    if kind == "local":
        sni, hs_server, hs_port = "decoy.test", "127.0.0.1", free_port()
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", f"{d}/k.pem", "-out", f"{d}/c.pem",
                        "-subj", "/CN=decoy.test", "-days", "1"], capture_output=True, check=True)
        procs.append(subprocess.Popen(["openssl", "s_server", "-accept", str(hs_port), "-cert", f"{d}/c.pem", "-key", f"{d}/k.pem",
                                       "-tls1_3", "-www", "-quiet"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        time.sleep(0.5)
    rp, sp, tp = free_port(), free_port(), free_port()
    server = {"log": {"level": "trace", "timestamp": True}, "inbounds": [{"type": "vless", "tag": "in", "listen": "127.0.0.1", "listen_port": rp,
              "users": [{"name": "u", "uuid": uuid, "flow": "xtls-rprx-vision"}],
              "tls": {"enabled": True, "server_name": sni, "reality": {"enabled": True, "handshake": {"server": hs_server, "server_port": hs_port},
                                                                        "private_key": priv, "short_id": [sid]}}}],
              "outbounds": [{"type": "direct", "tag": "direct"}]}
    client = {"log": {"level": "error"}, "inbounds": [{"type": "socks", "tag": "s", "listen": "127.0.0.1", "listen_port": sp}],
              "outbounds": [{"type": "vless", "tag": "p", "server": "127.0.0.1", "server_port": rp, "uuid": uuid, "flow": "xtls-rprx-vision",
                             "tls": {"enabled": True, "server_name": sni, "utls": {"enabled": True, "fingerprint": "chrome"},
                                     "reality": {"enabled": True, "public_key": pub, "short_id": sid}}}], "route": {"final": "p"}}
    for name, cfg in (("server", server), ("client", client)):
        json.dump(cfg, open(f"{d}/{name}.json", "w"))
        subprocess.run([SB, "check", "-c", f"{d}/{name}.json"], check=True)
    procs.append(subprocess.Popen([SB, "run", "-c", f"{d}/server.json"], stdout=open(f"{d}/server.log", "w"), stderr=subprocess.STDOUT))
    procs.append(subprocess.Popen([SB, "run", "-c", f"{d}/client.json"], stdout=open(f"{d}/client.log", "w"), stderr=subprocess.STDOUT))
    procs.append(subprocess.Popen([sys.executable, "-m", "http.server", str(tp), "--bind", "127.0.0.1"], cwd=d, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
    try:
        assert wait_log(f"{d}/server.log", "sing-box started"), "server did not start"
        time.sleep(1.0)
        url = f"http://127.0.0.1:{tp}/"

        def one(proxy):
            cmd = ["curl", "-s", "-o", "/dev/null", "-w", "%{time_total}", "--max-time", "20"] + (["--socks5-hostname", f"127.0.0.1:{sp}"] if proxy else []) + [url]
            r = subprocess.run(cmd, capture_output=True, text=True)
            return float(r.stdout) * 1000 if r.returncode == 0 else None

        for _ in range(3): one(True)                                   # warm-up (page cache, uTLS init)
        direct = [x for x in (one(False) for _ in range(n)) if x is not None]
        tunnel = [x for x in (one(True) for _ in range(n)) if x is not None]
        log = open(f"{d}/server.log", errors="ignore").read()
        dec = [float(m.group(1)) for m in re.finditer(r"time\.Since\(start\): ([0-9.]+)ms", log)]
        dec += [float(m.group(1)) * 1000 for m in re.finditer(r"time\.Since\(start\): ([0-9.]+)s\b", log)]
        med = statistics.median
        return {"variant": variant, "n": len(tunnel), "failed": n - len(tunnel), "direct_ms_median": round(med(direct), 1),
                "tunnel_ms_median": round(med(tunnel), 1), "added_ms_median": round(med(tunnel) - med(direct), 1),
                "added_ms_p25_p75": [round(statistics.quantiles(tunnel, n=4)[0] - med(direct), 1), round(statistics.quantiles(tunnel, n=4)[2] - med(direct), 1)],
                "server_decoy_ms_median": round(med(dec), 1) if dec else None, "decoy_samples": len(dec)}
    finally:
        for p in procs:
            p.terminate()
        for p in procs:
            try: p.wait(5)
            except subprocess.TimeoutExpired: p.kill()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--variant", action="append", required=True)
    ap.add_argument("--n", type=int, default=40)
    ap.add_argument("--rounds", type=int, default=1, help="interleave variants this many times (order effects)")
    ap.add_argument("--out")
    a = ap.parse_args()
    res = []
    for r in range(a.rounds):
        for v in a.variant:
            d = tempfile.mkdtemp(prefix="i60-")
            try:
                x = run_variant(v, a.n, d); x["round"] = r + 1; res.append(x); print(json.dumps(x), flush=True)
            finally:
                shutil.rmtree(d, ignore_errors=True)
    if a.out:
        json.dump({"evidence_class": "server verified, loopback client; not a real client network", "utc": time.strftime("%FT%TZ", time.gmtime()), "results": res}, open(a.out, "w"), indent=1)


if __name__ == "__main__":
    main()
