#!/usr/bin/env python3
"""Run the REAL Hiddify core (hiddify-core CLI, official release) against the
served subscription and measure what it actually carries.

Evidence class: client-core verified (Linux CLI of Hiddify's own core, loopback
client on the node). It is NOT the native iOS/Android/desktop app: it proves
Hiddify's own parser/builder/runtime accepts our profiles and interoperates with
the server (this includes version skew between hiddify-sing-box and the pinned
server sing-box). It cannot show app behaviour (profile storage, Connect On
Demand, the iOS missing-config error).

Two modes:

  --mode direct (default)  a single exit: the pinned profile, the multi-route profile, the share links and
                           the Hysteria2-only variants must each carry traffic out of the exit.
  --mode relay             a Russian-entry relay paired with an exit. ONLY the pinned profile
                           (`?format=singbox&compat=hiddify-pinned`) is fetched: share links are refused by
                           the relay (HTTP 400) because they cannot express the entry hop, and a relay carries
                           no Hysteria2. Before the core runs, the profile itself is checked for the intended
                           chain and for any selectable direct path; the core must then egress from the EXIT.

    hiddify_core_acceptance.py --core-dir DIR --url-file FILE --expect-egress-ip IP
                               [--mode relay] [--relay-host HOST] [--out JSON]

Needs root (runs the core as uid 65534 via setpriv). The subscription URL is read from a file and is never
printed, logged or written to the result document (errors are redacted). The core is third-party code: run it
only on a staging host, from a copy owned by root.
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
import urllib.error
import urllib.parse
import urllib.request

PORT = 12334


class FetchError(RuntimeError):
    """A subscription fetch failed. Carries the HTTP status only, never the URL (it holds the token)."""


def fetch(url):
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "hiddify-core-acceptance"})
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        raise FetchError(f"subscription fetch failed: HTTP {e.code} (URL withheld)") from None
    except (urllib.error.URLError, OSError) as e:
        raise FetchError(f"subscription fetch failed: {type(e).__name__} (URL withheld)") from None
    except ValueError:
        raise FetchError("subscription URL is malformed (URL withheld)") from None


def analyze_relay_profile(profile, relay_host):
    """Check a `compat=hiddify-pinned` profile of a RELAY: it must offer exactly one selectable route, that route
    must be the exit reached THROUGH the relay hop, and nothing may be selectable that bypasses the relay.

    Returns (problems, facts); an empty problems list means the chain is as intended."""
    problems = []
    out = {o.get("tag"): o for o in profile.get("outbounds", []) if isinstance(o, dict)}
    vless = [o for o in out.values() if o.get("type") == "vless"]
    if any(o.get("type") == "hysteria2" for o in out.values()):
        problems.append("a Hysteria2 outbound is present; a relay carries no Hysteria2")
    chained = [o for o in vless if o.get("detour")]
    if len(chained) != 1:
        problems.append(f"expected exactly one exit route reached through a relay hop, found {len(chained)}")
        return problems, {"chained_routes": len(chained)}
    via = chained[0]
    hop = out.get(via["detour"])
    if hop is None or hop.get("type") != "vless":
        problems.append("the route's detour does not point at a VLESS relay hop")
        return problems, {"chained_routes": 1}
    if hop.get("detour"):
        problems.append("the relay hop itself has a detour")
    if relay_host and hop.get("server") != relay_host:
        problems.append("the first hop is not the expected relay host")
    if via.get("server") == hop.get("server"):
        problems.append("the exit and the relay hop are the same server")
    selectors = [o for o in out.values() if o.get("type") == "selector"]
    urltests = [o for o in out.values() if o.get("type") == "urltest"]
    selectable = set()
    for s in selectors:
        selectable.update(s.get("outbounds", []))
    for u in urltests:
        selectable.update(u.get("outbounds", []))
    allowed = {via["tag"]} | {u["tag"] for u in urltests}
    for tag in sorted(selectable - allowed):
        problems.append(f"selectable outbound {tag!r} is neither the chained exit route nor a group of it")
    if hop["tag"] in selectable:
        problems.append("the relay hop is directly selectable (a client could use the entry as an exit)")
    for u in urltests:
        if set(u.get("outbounds", [])) - {via["tag"]}:
            problems.append("an automatic group contains something other than the chained exit route")
    for tag in selectable:
        if out.get(tag, {}).get("type") in ("direct", "block"):
            problems.append(f"a {out[tag]['type']} outbound is selectable")
    final = profile.get("route", {}).get("final")
    if final is not None and out.get(final, {}).get("type") == "direct":
        problems.append("route.final is a direct outbound")
    for s in selectors:
        if s.get("default") not in (None, via["tag"]):
            problems.append("a selector defaults to something other than the chained exit route")
    facts = {"chained_routes": 1, "selectable": sorted(selectable), "has_relay_hop": True}
    return problems, facts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--core-dir", required=True)
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--expect-egress-ip", required=True)
    ap.add_argument("--mode", choices=("direct", "relay"), default="direct")
    ap.add_argument("--relay-host", help="relay mode: the expected first-hop host (default: the subscription host)")
    ap.add_argument("--stage-dir", default="/var/tmp/hcore-run")
    ap.add_argument("--out")
    a = ap.parse_args()
    if os.geteuid() != 0:
        print("must run as root (drops to uid 65534 with setpriv)")
        return 1
    url = open(a.url_file).read().strip()
    if not url.startswith(("https://", "http://")):
        print("FAIL --url-file does not hold an http(s) subscription URL (content withheld)")
        return 1
    base = url.split("?")[0]
    stage = a.stage_dir
    try:
        return run(a, base, stage)
    except FetchError as e:
        print(f"FAIL {e}")
        return 1
    finally:
        shutil.rmtree(stage, ignore_errors=True)


def run(a, base, stage):
    shutil.rmtree(stage, ignore_errors=True)
    os.makedirs(stage + "/work", mode=0o755)
    shutil.copytree(a.core_dir, stage + "/bin")
    os.chmod(stage, 0o755)
    os.chmod(stage + "/work", 0o700)
    os.chown(stage + "/work", 65534, 65534)
    profile_facts = None
    if a.mode == "relay":
        pinned = fetch(base + "?format=singbox&compat=hiddify-pinned")
        relay_host = a.relay_host or urllib.parse.urlparse(base).hostname
        problems, profile_facts = analyze_relay_profile(json.loads(pinned), relay_host)
        for p in problems:
            print(f"FAIL profile: {p}")
        if problems:
            print("RESULT: the relay profile does not hold the intended chain; the core was not run")
            return 1
        print("PASS profile: one chained exit route through the relay hop, nothing selectable bypasses it")
        bodies = {"pinned-json": ("pinned.json", pinned)}
        want = {"pinned-json": {"vless"}}
    else:
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
        open(p, "wb").write(body)
        os.chmod(p, 0o644)
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
           "mode": a.mode,
           "evidence_class": "client-core verified (Hiddify core CLI, loopback on the node); NOT the native app",
           "core": ver, "passed": sum(r["pass"] for r in results), "total": len(results), "results": results}
    if profile_facts is not None:
        doc["relay_profile"] = profile_facts
    if a.out:
        json.dump(doc, open(a.out, "w"), indent=1)
    print(f"RESULT: {doc['passed']}/{doc['total']} ({ver})")
    return 0 if doc["passed"] == doc["total"] else 1


if __name__ == "__main__":
    sys.exit(main())
