"""Direct vs REALITY-tunnel measurements from this PC (bounded, repeated). usage: perf_test.py <native-sub.json> <out.json> <label>"""
import json
import os
import statistics as st
import subprocess
import sys
import tempfile
import time

SP = os.path.dirname(os.path.abspath(__file__))
SINGBOX = os.path.join(SP, "sbclient", "sing-box-1.14.1-windows-amd64", "sing-box.exe")
PORT = 17896
FMT = "%{time_namelookup} %{time_connect} %{time_appconnect} %{time_starttransfer} %{time_total} %{speed_download} %{http_code}"


def curl(url: str, proxy: bool, timeout: int = 60, extra: list[str] | None = None) -> list[float] | None:
    args = ["curl.exe", "-sS", "--max-time", str(timeout), "-o", os.devnull, "-w", FMT, *(extra or [])]
    if proxy:
        args += ["-x", f"socks5h://127.0.0.1:{PORT}"]
    p = subprocess.run(args + [url], capture_output=True, text=True)
    parts = p.stdout.split()
    if p.returncode != 0 or len(parts) != 7 or parts[6] not in ("200", "204"):
        return None
    return [float(x) for x in parts[:6]]


def summarize(xs: list[float]) -> dict:
    xs = sorted(xs)
    return {"n": len(xs), "median": round(st.median(xs), 3), "p90": round(xs[int(0.9 * (len(xs) - 1))], 3),
            "min": round(xs[0], 3), "max": round(xs[-1], 3)}


def main() -> None:
    doc = json.load(open(sys.argv[1]))
    r = [o for o in doc["outbounds"] if o.get("type") == "vless"][0]
    cfg = {"log": {"level": "error"}, "dns": {"servers": [{"tag": "local", "type": "udp", "server": "1.1.1.1"},
           {"tag": "remote", "type": "udp", "server": "1.1.1.1", "detour": r["tag"]}], "final": "remote"},
           "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": PORT}],
           "outbounds": [r, {"type": "direct", "tag": "direct"}],
           "route": {"final": r["tag"], "default_domain_resolver": {"server": "local"}}}
    td = tempfile.mkdtemp()
    cp = os.path.join(td, "c.json")
    json.dump(cfg, open(cp, "w"))
    client = subprocess.Popen([SINGBOX, "run", "-c", cp], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    out: dict = {"label": sys.argv[3], "when": time.strftime("%Y-%m-%dT%H:%M:%S"), "results": {}}
    try:
        for _ in range(30):
            if curl("https://www.gstatic.com/generate_204", True, 8):
                break
            time.sleep(1)
        small = "https://www.gstatic.com/generate_204"
        big = "https://speed.cloudflare.com/__down?bytes=25000000"
        for mode, proxy in (("direct", False), ("reality_tunnel", True)):
            lat = [x for x in (curl(small, proxy, 12) for _ in range(20)) if x]
            res: dict = {"requests_ok": len(lat)}
            if lat:
                res["tcp_connect_s"] = summarize([x[1] for x in lat])
                res["tls_handshake_done_s"] = summarize([x[2] for x in lat])
                res["time_to_first_byte_s"] = summarize([x[3] for x in lat])
            dl = [x for x in (curl(big, proxy, 90) for _ in range(3)) if x]
            if dl:
                res["download_25MB_mbit_s"] = [round(x[5] * 8 / 1e6, 1) for x in dl]
                res["download_median_mbit_s"] = round(st.median([x[5] * 8 / 1e6 for x in dl]), 1)
            out["results"][mode] = res
    finally:
        client.terminate()
    json.dump(out, open(sys.argv[2], "w"), indent=1)
    print(json.dumps(out, indent=1))


main()
