#!/usr/bin/env python3
"""External-network acceptance probe. Run it from a machine OUTSIDE the node's
provider network (your laptop on home broadband, a phone hotspot, another VPS).
Stdlib only; optional `sing-box` and `curl` enable the data-plane checks.

    external_probe.py --host fin1.example --expect-ip 62.238.46.190 \
        [--url-file sub-url.txt --sing-box /path/to/sing-box] [--out result.json]

What it establishes, per check, with an honest label of what it can and cannot
prove (a TCP connect says nothing about UDP/QUIC, which is only exercised through
a real Hysteria2 client). The subscription URL is read from a file and never printed.
Exit 0 only if every executed check passed; skipped checks are listed as SKIPPED
(never as passed).
"""
import argparse
import datetime
import json
import os
import socket
import ssl
import subprocess
import sys
import time

results = []


def rec(name, ok, detail="", skipped=False):
    results.append({"check": name, "result": "SKIPPED" if skipped else ("PASS" if ok else "FAIL"), "detail": detail})
    print(f"{'SKIP' if skipped else ('PASS' if ok else 'FAIL')}  {name}" + (f"  ({detail})" if detail else ""))


def tcp(host, port, timeout=8):
    t = time.time()
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True, f"{(time.time() - t) * 1000:.0f} ms"
    except OSError as e:
        return False, type(e).__name__


def is_local_address(ip):
    """True if ip is configured on this machine (bind succeeds only for local addresses)."""
    fam = socket.AF_INET6 if ":" in ip else socket.AF_INET
    try:
        with socket.socket(fam, socket.SOCK_STREAM) as s:
            s.bind((ip, 0))
        return True
    except OSError:
        return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", required=True)
    ap.add_argument("--expect-ip", required=True)
    ap.add_argument("--sub-port", type=int, default=8443)
    ap.add_argument("--url-file")
    ap.add_argument("--sing-box")
    ap.add_argument("--out")
    a = ap.parse_args()

    local = is_local_address(a.expect_ip)
    if local:
        print("NOTE: --expect-ip is an address of THIS machine, so this run is NOT external-network evidence.")
    # 1. DNS: exactly what the contract says (A -> expected IP; AAAA only if deliberately published)
    try:
        infos = socket.getaddrinfo(a.host, None, proto=socket.IPPROTO_TCP)
        v4 = sorted({i[4][0] for i in infos if i[0] == socket.AF_INET})
        v6 = sorted({i[4][0] for i in infos if i[0] == socket.AF_INET6})
        rec("DNS A record equals the expected IPv4", v4 == [a.expect_ip], f"A={v4}")
        rec("AAAA record (informational; publish only after IPv6 inbound is accepted)", True, f"AAAA={v6 or 'none'}")
    except OSError as e:
        rec("DNS resolves", False, type(e).__name__)
    # 2. TCP reachability from this vantage
    for port, what in ((443, "REALITY"), (a.sub_port, "subscription HTTPS")):
        ok, d = tcp(a.expect_ip, port)
        rec(f"TCP {port} ({what}) reachable from this network", ok, d)
    ok, d = tcp(a.expect_ip, 22)
    rec("TCP 22 (SSH) reachable (expected, rate-limited)", ok, d)
    # 3. TLS to the subscription endpoint: verified chain, SAN covers the host, not expiring
    try:
        ctx = ssl.create_default_context()
        with socket.create_connection((a.expect_ip, a.sub_port), timeout=10) as s, ctx.wrap_socket(s, server_hostname=a.host) as t:
            cert = t.getpeercert()
        sans = [v for k, v in cert.get("subjectAltName", ()) if k == "DNS"]
        left = (ssl.cert_time_to_seconds(cert["notAfter"]) - time.time()) / 86400
        rec("subscription TLS: trusted chain, hostname verified (no -k)", a.host in sans, f"SAN={sans}")
        rec("subscription certificate not expiring within 14 days", left >= 14, f"{left:.0f} days left")
    except (OSError, ssl.SSLError) as e:
        rec("subscription TLS verifies", False, f"{type(e).__name__}: {str(e)[:80]}")
    # 4. Data plane through the real client (needs the subscription URL + sing-box)
    if a.url_file and a.sing_box:
        here = os.path.dirname(os.path.abspath(__file__))
        out = a.out + ".data.json" if a.out else "/dev/null"
        p = subprocess.run([sys.executable, "-I", os.path.join(here, "subscription_acceptance.py"), "--url-file", a.url_file,
                            "--public-host", a.host, "--expect-egress-ip", a.expect_ip, "--sing-box", a.sing_box, "--out", out],
                           capture_output=True, text=True)
        tail = [l for l in p.stdout.splitlines() if l.startswith(("RESULT", "FAIL"))]
        rec("real REALITY and Hysteria2 clients carry traffic from this network (all subscription checks)", p.returncode == 0,
            "; ".join(tail)[:200])
    else:
        rec("data-plane check through real REALITY/Hysteria2 clients", False,
            "needs --url-file and --sing-box; UDP/QUIC (Hysteria2) cannot be judged from a TCP connect", skipped=True)
    doc = {"schema": 1, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
           "evidence_class": ("NOT EXTERNAL: ran on the node itself (server verified at most)" if local
                              else "external-network (record the vantage network in the report)"),
           "host": a.host, "results": results}
    if a.out:
        json.dump(doc, open(a.out, "w"), indent=1)
    failed = [r for r in results if r["result"] == "FAIL"]
    skipped = [r for r in results if r["result"] == "SKIPPED"]
    print(f"RESULT: {sum(r['result'] == 'PASS' for r in results)} passed, {len(failed)} failed, {len(skipped)} skipped")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
