#!/usr/bin/env python3
"""Run the REAL Hiddify core (hiddify-core CLI, official release) against the
served subscription and measure what it actually carries.

Evidence class: client-core verified (Linux CLI of Hiddify's own core, loopback
client on the node). It is NOT the native iOS/Android/desktop app: it proves
Hiddify's own parser/builder/runtime accepts our profiles and interoperates with
the server (this includes version skew between hiddify-sing-box and the pinned
server sing-box). It cannot show app behaviour (profile storage, Connect On
Demand, the iOS missing-config error).

    hiddify_core_acceptance.py --core-dir DIR --url-file FILE --expect-egress-ip IP [--out JSON]

Needs root (runs the core as uid 65534 via setpriv). The subscription URL is read
from a file and never printed. The core is third-party code: run it only on a
staging host, from a copy owned by root.
"""
import argparse
import datetime
import json
import os
import shutil
import signal
import subprocess
import sys
import time
import urllib.request

PORT = 12334


def fetch(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "hiddify-core-acceptance"}), timeout=20) as r:
        return r.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--core-dir", required=True)
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--expect-egress-ip", required=True)
    ap.add_argument("--stage-dir", default="/var/tmp/hcore-run")
    ap.add_argument("--out")
    a = ap.parse_args()
    if os.geteuid() != 0:
        print("must run as root (drops to uid 65534 with setpriv)"); return 1
    url = open(a.url_file).read().strip()
    base = url.split("?")[0]
    stage = a.stage_dir
    shutil.rmtree(stage, ignore_errors=True)
    os.makedirs(stage + "/work", mode=0o755)
    shutil.copytree(a.core_dir, stage + "/bin")
    os.chmod(stage, 0o755)
    os.chmod(stage + "/work", 0o700)
    os.chown(stage + "/work", 65534, 65534)
    bodies = {
        "pinned-json": ("pinned.json", fetch(base + "?format=singbox&compat=hiddify-pinned")),
        "full-multiroute-json": ("full.json", fetch(base + "?format=singbox")),
        "share-links": ("links.txt", fetch(base + "?format=uri")),
    }
    links = bodies["share-links"][1].decode().split()
    bodies["vless-link-only"] = ("vless.txt", ("\n".join(l for l in links if l.startswith("vless://")) + "\n").encode())
    bodies["hysteria2-link-only"] = ("hy2.txt", ("\n".join(l for l in links if l.startswith("hysteria2://")) + "\n").encode())
    full = json.loads(bodies["full-multiroute-json"][1])
    full["outbounds"] = [o for o in full["outbounds"] if o["type"] in ("hysteria2", "direct")]
    bodies["hysteria2-json-only"] = ("hy2.json", json.dumps(full).encode())
    want = {"pinned-json": {"vless"}, "full-multiroute-json": {"vless"}, "share-links": {"vless"},
            "vless-link-only": {"vless"}, "hysteria2-link-only": {"hysteria2"}, "hysteria2-json-only": {"hysteria2"}}
    for name, (fn, body) in bodies.items():
        p = f"{stage}/{fn}"
        open(p, "wb").write(body); os.chmod(p, 0o644)
    ver = subprocess.run([a.core_dir + "/hiddify-core", "version"], capture_output=True, text=True,
                         env={"LD_LIBRARY_PATH": a.core_dir}).stdout.splitlines()[0]
    env = {"HOME": stage + "/work", "LD_LIBRARY_PATH": stage + "/bin", "PATH": "/usr/bin:/bin"}
    pre = ["setpriv", "--reuid=65534", "--regid=65534", "--clear-groups"]
    results = []
    for name, (fn, _) in bodies.items():
        log = f"{stage}/work/run-{name}.log"
        proc = subprocess.Popen(pre + [stage + "/bin/hiddify-core", "-D", stage + "/work", "run", "-c", f"{stage}/{fn}",
                                       "--in-proxy-port", str(PORT), "--log-level", "info"],
                                stdout=open(log, "w"), stderr=subprocess.STDOUT, env=env, start_new_session=True)
        egress, http = "", ""
        for _ in range(40):
            if subprocess.run(["bash", "-c", f"echo >/dev/tcp/127.0.0.1/{PORT}"], capture_output=True).returncode == 0:
                break
            time.sleep(1)
        time.sleep(2)
        c = subprocess.run(["curl", "-s", "-m", "25", "-x", f"socks5h://127.0.0.1:{PORT}", "https://api.ipify.org"], capture_output=True, text=True)
        egress = c.stdout.strip()
        c = subprocess.run(["curl", "-s", "-m", "25", "-o", "/dev/null", "-w", "%{http_code}", "-x", f"http://127.0.0.1:{PORT}", "https://www.gstatic.com/generate_204"], capture_output=True, text=True)
        http = c.stdout.strip()
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        proc.wait(10)
        txt = open(log, errors="replace").read()
        seen = {k for k in ("vless", "hysteria2") if f"outbound/{k}[" in txt}
        ok = egress == a.expect_egress_ip and http == "204" and want[name] <= seen
        results.append({"case": name, "egress": egress, "http_204": http, "protocols_seen_in_core_log": sorted(seen),
                        "expected_protocols": sorted(want[name]), "pass": bool(ok)})
        print(("PASS " if ok else "FAIL ") + f"{name}: egress={egress or 'none'} http={http or 'none'} protocols={sorted(seen)}")
    doc = {"schema": 1, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
           "evidence_class": "client-core verified (Hiddify core CLI, loopback on the node); NOT the native app",
           "core": ver, "passed": sum(r["pass"] for r in results), "total": len(results), "results": results}
    if a.out:
        json.dump(doc, open(a.out, "w"), indent=1)
    shutil.rmtree(stage, ignore_errors=True)
    print(f"RESULT: {doc['passed']}/{doc['total']} ({ver})")
    return 0 if doc["passed"] == doc["total"] else 1


if __name__ == "__main__":
    sys.exit(main())
