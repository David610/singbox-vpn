#!/usr/bin/env python3
"""Quantitative tunnel measurements with explicit pass thresholds.

Runs REAL sing-box clients built from the served subscription against the live
listeners (REALITY and Hysteria2) and compares each with the same transfer made
directly from this host. Non-mutating (uses an existing QA subscription).

EVIDENCE CLASS: server verified, LOOPBACK client. Client and server share this
host's CPU, and the traffic never crosses a client network, so this measures
tunnel software overhead + the node's own uplink to the Internet. It says nothing
about a real client's access network, carrier NAT/UDP behaviour or phone CPU;
those need an external vantage point and are reported UNVERIFIED by this tool.

Thresholds (docs/operations/ACCEPTANCE_THRESHOLDS.md, marked calibrate there):
  throughput ratio  tunnel/direct median >= --min-ratio      (default 0.70)
  added latency     median TCP-connect delta <= --max-added-ms (default 40)
  concurrency       0 failed transfers at each level
  soak              success rate >= --min-soak-success       (default 0.99)
"""
import argparse
import datetime
import json
import os
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import importlib.util

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("sa", os.path.join(HERE, "subscription_acceptance.py"))
sa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sa)


def curl_download(url, proxy=None, timeout=90):
    cmd = ["curl", "-s", "-o", "/dev/null", "-m", str(timeout), "-w", "%{http_code} %{size_download} %{time_total}", url]
    if proxy:
        cmd[1:1] = ["--socks5-hostname", proxy]
    p = subprocess.run(cmd, capture_output=True, text=True)
    try:
        code, size, t = p.stdout.split()
        size, t = float(size), float(t)
        return (code == "200" and size > 0, size * 8 / t / 1e6 if t > 0 else 0.0)
    except ValueError:
        return (False, 0.0)


def connect_ms(proxy=None):
    cmd = ["curl", "-s", "-o", "/dev/null", "-m", "10", "-w", "%{time_connect}", "http://1.1.1.1/"]
    if proxy:
        cmd[1:1] = ["--socks5-hostname", proxy]
    p = subprocess.run(cmd, capture_output=True, text=True)
    try:
        v = float(p.stdout.strip())
        return v * 1000 if v > 0 else None
    except ValueError:
        return None


def pct(vals, q):
    s = sorted(vals)
    return s[min(len(s) - 1, int(round(q * (len(s) - 1))))] if s else None


def start_client(singbox, doc, tag, wd):
    port = sa.free_port()
    cfg = json.loads(json.dumps(doc))
    cfg["log"] = {"level": "error"}
    cfg["inbounds"] = [{"type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": port}]
    cfg.setdefault("route", {})["final"] = tag
    path = os.path.join(wd, f"{tag}.json")
    json.dump(cfg, open(path, "w"))
    os.chmod(path, 0o600)
    proc = subprocess.Popen([singbox, "run", "-c", path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(50):
        if subprocess.run(["bash", "-c", f"echo >/dev/tcp/127.0.0.1/{port}"], capture_output=True).returncode == 0:
            break
        time.sleep(0.1)
    return proc, f"127.0.0.1:{port}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--sing-box", default="/usr/local/bin/sing-box")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--bytes", type=int, default=25_000_000)
    ap.add_argument("--latency-samples", type=int, default=60)
    ap.add_argument("--concurrency", default="1,4,8")
    ap.add_argument("--soak-seconds", type=int, default=0)
    ap.add_argument("--restart-recovery", type=int, default=0,
                    help="N: restart sing-box N times per protocol and time the client's recovery (SERVICE-AFFECTING)")
    ap.add_argument("--max-reality-recovery-s", type=float, default=10.0)
    ap.add_argument("--max-hy2-recovery-s", type=float, default=35.0)
    ap.add_argument("--min-ratio", type=float, default=0.70)
    ap.add_argument("--max-added-ms", type=float, default=40.0)
    ap.add_argument("--min-soak-success", type=float, default=0.99)
    ap.add_argument("--out")
    a = ap.parse_args()
    url = open(a.url_file).read().strip()
    dl = f"https://speed.cloudflare.com/__down?bytes={a.bytes}"
    small = "https://speed.cloudflare.com/__down?bytes=100000"
    doc = json.loads(sa.fetch(url.split("?")[0] + "?format=singbox")[2])
    res = {"schema": 1, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
           "evidence_class": "server-verified, loopback client (shares host CPU); NOT a real client network path",
           "download": dl, "runs": a.runs, "thresholds": {"min_ratio": a.min_ratio, "max_added_ms": a.max_added_ms, "min_soak_success": a.min_soak_success},
           "direct": {}, "tunnels": {}, "verdicts": []}
    direct = [curl_download(dl)[1] for _ in range(a.runs)]
    dlat = [x for x in (connect_ms() for _ in range(a.latency_samples)) if x]
    res["direct"] = {"mbps_runs": [round(x, 1) for x in direct], "mbps_median": round(statistics.median(direct), 1),
                     "connect_ms_median": round(statistics.median(dlat), 1) if dlat else None, "connect_ms_p95": round(pct(dlat, .95), 1) if dlat else None}
    print(f"direct: {res['direct']}")
    verd = []
    with tempfile.TemporaryDirectory() as wd:
        for tag in ("Reality", "Hysteria2"):
            proc, sock = start_client(a.sing_box, doc, tag, wd)
            try:
                t = {}
                runs = [curl_download(dl, sock) for _ in range(a.runs)]
                ok = [m for g, m in runs if g]
                t["transfer_failures"] = sum(1 for g, _ in runs if not g)
                t["mbps_runs"] = [round(m, 1) for _, m in runs]
                t["mbps_median"] = round(statistics.median(ok), 1) if ok else 0.0
                t["ratio_vs_direct"] = round(t["mbps_median"] / res["direct"]["mbps_median"], 2) if res["direct"]["mbps_median"] else None
                lat = [x for x in (connect_ms(sock) for _ in range(a.latency_samples)) if x]
                t["connect_ms_median"] = round(statistics.median(lat), 1) if lat else None
                t["connect_ms_p95"] = round(pct(lat, .95), 1) if lat else None
                t["connect_failures"] = a.latency_samples - len(lat)
                t["added_ms_median"] = round(t["connect_ms_median"] - res["direct"]["connect_ms_median"], 1) if lat and dlat else None
                conc = {}
                for n in [int(x) for x in a.concurrency.split(",") if x]:
                    outs = []
                    def work():
                        outs.append(curl_download(f"https://speed.cloudflare.com/__down?bytes={a.bytes // 2}", sock))
                    th = [threading.Thread(target=work) for _ in range(n)]
                    t0 = time.time(); [x.start() for x in th]; [x.join() for x in th]; el = time.time() - t0
                    good = sum(1 for g, _ in outs if g)
                    conc[str(n)] = {"failed": n - good, "aggregate_mbps": round(good * (a.bytes // 2) * 8 / el / 1e6, 1) if el else 0}
                t["concurrency"] = conc
                if a.soak_seconds:
                    end = time.time() + a.soak_seconds; okc = bad = 0; maxgap = 0.0; last = time.time()
                    while time.time() < end:
                        g, _ = curl_download(small, sock, 20)
                        now = time.time()
                        if g: okc += 1; maxgap = max(maxgap, now - last); last = now
                        else: bad += 1
                        time.sleep(5)
                    t["soak"] = {"seconds": a.soak_seconds, "ok": okc, "failed": bad, "success_rate": round(okc / max(1, okc + bad), 4), "max_gap_s": round(maxgap, 1)}
                if a.restart_recovery:
                    rec_times, never = [], 0
                    for _ in range(a.restart_recovery):
                        subprocess.run(["systemctl", "restart", "sing-box"], check=False)
                        t0 = time.time(); got = None
                        while time.time() - t0 < 90:
                            if curl_download(small, sock, 6)[0]:
                                got = time.time() - t0; break
                            time.sleep(0.5)
                        if got is None: never += 1
                        else: rec_times.append(got)
                        time.sleep(3)
                    t["restart_recovery_s"] = {"samples": [round(x, 1) for x in rec_times], "never_recovered": never,
                                               "max": round(max(rec_times), 1) if rec_times else None,
                                               "median": round(statistics.median(rec_times), 1) if rec_times else None}
                res["tunnels"][tag] = t
                v = lambda name, ok, detail: verd.append({"check": f"{tag}: {name}", "pass": bool(ok), "detail": detail})
                v("no failed transfers", t["transfer_failures"] == 0 and t["connect_failures"] == 0, f"transfers {t['transfer_failures']}, connects {t['connect_failures']}")
                v(f"throughput ratio >= {a.min_ratio}", (t["ratio_vs_direct"] or 0) >= a.min_ratio, f"{t['mbps_median']} vs direct {res['direct']['mbps_median']} Mbps = {t['ratio_vs_direct']}")
                v(f"added connect latency <= {a.max_added_ms} ms", t["added_ms_median"] is not None and t["added_ms_median"] <= a.max_added_ms, f"{t['added_ms_median']} ms (median)")
                v("concurrency: 0 failed transfers", all(c["failed"] == 0 for c in conc.values()), json.dumps(conc))
                if a.restart_recovery:
                    lim = a.max_reality_recovery_s if tag == "Reality" else a.max_hy2_recovery_s
                    rr = t["restart_recovery_s"]
                    v(f"recovers after a server restart within {lim} s ({a.restart_recovery}/{a.restart_recovery})",
                      rr["never_recovered"] == 0 and rr["max"] is not None and rr["max"] <= lim, json.dumps(rr))
                if a.soak_seconds:
                    v(f"soak success >= {a.min_soak_success}", t["soak"]["success_rate"] >= a.min_soak_success, json.dumps(t["soak"]))
                print(f"{tag}: {json.dumps(t)}")
            finally:
                proc.terminate(); proc.wait(5)
    res["verdicts"] = verd
    res["not_measured"] = ["real client access network", "carrier NAT / UDP filtering", "phone CPU/battery", "24 h soak (unless --soak-seconds >= 86400)"]
    for x in verd:
        print(("PASS " if x["pass"] else "FAIL ") + x["check"] + "  (" + x["detail"][:110] + ")")
    if a.out:
        json.dump(res, open(a.out, "w"), indent=1)
    return 0 if all(x["pass"] for x in verd) else 1


if __name__ == "__main__":
    sys.exit(main())
