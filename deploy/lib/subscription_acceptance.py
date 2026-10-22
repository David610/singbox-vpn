#!/usr/bin/env python3
"""Real-subscription acceptance: fetch every supported format with TLS
verification ON, validate it against the declared host, then run the served
sing-box document as a REAL client (one per protocol) and measure the egress
address through it. Evidence class: server verified (loopback client on the
node itself — proves credentials, TLS/SNI, REALITY/Hysteria2 data path and
egress; NOT an external-network or native-client result).

Secrets: the subscription URL is read from a file and never printed or put in
the JSON evidence. Exit status 0 only if every check passed.
"""
from __future__ import annotations

import argparse
import base64
import datetime
import json
import os
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

FORMATS = [
    ("hiddify", "?format=hiddify"),
    ("uri", "?format=uri"),
    ("uri-base64", "?format=uri&encoding=base64"),
    ("singbox", "?format=singbox"),
    ("singbox-hiddify-pinned", "?format=singbox&compat=hiddify-pinned"),
    ("default", ""),
]
results: list[dict] = []


def rec(name: str, ok: bool, detail: str = "") -> bool:
    results.append({"check": name, "pass": bool(ok), "detail": detail})
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  ({detail})" if detail else ""))
    return ok


def fetch(url: str, timeout=20):
    req = urllib.request.Request(url, headers={"User-Agent": "singbox-vpn-acceptance"})
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=ssl.create_default_context()) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run_client(singbox: str, doc: dict, tag: str, workdir: str) -> tuple[bool, str]:
    """Run `doc` as a client with route.final=tag; return (ok, egress ip / error)."""
    port = free_port()
    cfg = json.loads(json.dumps(doc))
    cfg["log"] = {"level": "error"}
    cfg["inbounds"] = [{"type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": port}]
    cfg.setdefault("route", {})["final"] = tag
    path = os.path.join(workdir, f"client-{tag}.json")
    with open(path, "w") as f:
        json.dump(cfg, f)
    os.chmod(path, 0o600)
    chk = subprocess.run([singbox, "check", "-c", path], capture_output=True, text=True)
    if chk.returncode != 0:
        return False, "sing-box check failed: " + chk.stderr.strip()[:200]
    proc = subprocess.Popen([singbox, "run", "-c", path], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        for _ in range(50):
            with socket.socket() as s:
                if s.connect_ex(("127.0.0.1", port)) == 0:
                    break
            time.sleep(0.1)
        last = ""
        for attempt in range(3):
            c = subprocess.run(
                ["curl", "-sS", "-4", "-m", "20", "--socks5-hostname", f"127.0.0.1:{port}", "https://api.ipify.org"],
                capture_output=True, text=True)
            if c.returncode == 0 and c.stdout.strip():
                return True, c.stdout.strip()
            last = c.stderr.strip()[:160]
            time.sleep(1)
        return False, "no data through tunnel: " + last
    finally:
        proc.terminate()
        try:
            proc.wait(5)
        except subprocess.TimeoutExpired:
            proc.kill()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url-file", required=True, help="file holding the subscription URL (secret)")
    ap.add_argument("--public-host", required=True)
    ap.add_argument("--expect-egress-ip", help="egress IPv4 the tunnel must show")
    ap.add_argument("--sing-box", default="/usr/local/bin/sing-box")
    ap.add_argument("--out")
    a = ap.parse_args()
    url = open(a.url_file).read().strip()
    base = url.split("?")[0]
    body: dict[str, bytes] = {}

    for name, q in FORMATS:
        st, hd, b = fetch(base + q)
        h = {k.lower(): v for k, v in hd.items()}
        rec(f"{name}: HTTP 200 over verified TLS", st == 200, f"status={st}")
        rec(f"{name}: Cache-Control no-store", "no-store" in h.get("cache-control", ""), h.get("cache-control", ""))
        rec(f"{name}: X-Content-Type-Options nosniff", h.get("x-content-type-options") == "nosniff")
        rec(f"{name}: HSTS present", "strict-transport-security" in h)
        body[name] = b

    # URI body content
    uri_lines = body["uri"].decode().split()
    schemes = sorted(u.split("://")[0] for u in uri_lines)
    rec("uri: exactly one vless and one hysteria2 link", schemes == ["hysteria2", "vless"], ",".join(schemes))
    for u in uri_lines:
        p = urllib.parse.urlsplit(u)
        q = dict(urllib.parse.parse_qsl(p.query))
        rec(f"uri {p.scheme}: host is the declared public host", p.hostname == a.public_host, p.hostname or "")
        if p.scheme == "vless":
            rec("uri vless: REALITY params present (pbk,sid,sni,flow)", all(q.get(k) for k in ("pbk", "sid", "sni", "flow")))
        else:
            rec("uri hysteria2: sni equals public host, verification on, salamander obfs",
                q.get("sni") == a.public_host and q.get("insecure") == "0" and q.get("obfs") == "salamander" and bool(q.get("obfs-password")))
    try:
        dec = base64.b64decode(body["uri-base64"]).decode().split()
        rec("uri-base64: decodes to the same links", sorted(dec) == sorted(uri_lines))
    except Exception as e:  # noqa: BLE001
        rec("uri-base64: decodes to the same links", False, type(e).__name__)

    docs = {}
    for name in ("singbox", "singbox-hiddify-pinned", "default"):
        try:
            docs[name] = json.loads(body[name])
        except Exception:  # noqa: BLE001
            rec(f"{name}: valid JSON", False)
            continue
        outs = {o.get("tag"): o for o in docs[name].get("outbounds", [])}
        pinned = name == "singbox-hiddify-pinned"
        # compat=hiddify-pinned serves exactly ONE route by design (REALITY), so
        # Hiddify cannot build a balancer (docs/YOUTUBE_FINAL_ROOT_CAUSE.md 13.4).
        want = ("Reality",) if pinned else ("Reality", "Hysteria2")
        rec(f"{name}: outbounds {'/'.join(want)} point at the declared host",
            all(outs.get(t, {}).get("server") == a.public_host for t in want))
        if pinned:
            rec(f"{name}: no Hysteria2 outbound (single pinned route)", "Hysteria2" not in outs)
        else:
            rec(f"{name}: Hysteria2 SNI = public host, certificate verification on",
                outs.get("Hysteria2", {}).get("tls", {}).get("server_name") == a.public_host
                and outs.get("Hysteria2", {}).get("tls", {}).get("insecure") is False)
    rec("default format equals format=singbox", body["default"] == body["singbox"])

    # Authorization: bad tokens must not yield a profile
    bad = base + "x"
    rec("invalid token is rejected (404)", fetch(bad + "?format=uri")[0] == 404)
    rec("empty token path is rejected", fetch(base.rsplit("/", 1)[0] + "/?format=uri")[0] in (400, 401, 403, 404))

    # Real clients
    with tempfile.TemporaryDirectory(prefix="subacc-") as wd:
        for name in ("singbox", "singbox-hiddify-pinned"):
            if name not in docs:
                continue
            for tag in (("Reality",) if name == "singbox-hiddify-pinned" else ("Reality", "Hysteria2")):
                if tag not in {o.get("tag") for o in docs[name].get("outbounds", [])}:
                    rec(f"{name}: real client over {tag}", False, "outbound missing")
                    continue
                ok, info = run_client(a.sing_box, docs[name], tag, wd)
                good = ok and (not a.expect_egress_ip or info == a.expect_egress_ip)
                rec(f"{name}: REAL sing-box client over {tag} carries traffic", good,
                    f"egress={info}" if ok else info)

    failed = [r for r in results if not r["pass"]]
    doc = {"schema": 1, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
           "evidence_class": "server-verified-loopback-client", "public_host": a.public_host,
           "passed": len(results) - len(failed), "failed": len(failed), "results": results}
    if a.out:
        with open(a.out, "w") as f:
            json.dump(doc, f, indent=1)
    print(f"RESULT: {doc['passed']} passed, {doc['failed']} failed")
    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
