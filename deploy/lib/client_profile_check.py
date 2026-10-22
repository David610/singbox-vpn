#!/usr/bin/env python3
"""Server-side verification of the subscription profiles the PRODUCTION pipeline serves, per client family.

This proves the PROFILES are well-formed and match the node (server verified). It does NOT prove a native app imports or
connects: every client row in the output carries `native_app: UNVERIFIED` until a dated device record exists
(docs/DEVICE_ACCEPTANCE_TESTS.md). The subscription URL (it contains the token) is read from a file and never printed.

  client_profile_check.py --url-file /root/qa-secrets/url.txt --host fin1.example.com [--sing-box /usr/local/bin/sing-box] [--out report.json]

Client families and the URL each is told to use (docs/clients/*.md):
  sing-box (native)        ?format=singbox                         both transports, REALITY selected by default
  Hiddify (iOS/Android)    ?format=singbox&compat=hiddify-pinned   ONE route (Hiddify rebuilds its own groups and balances new
                                                                   connections across routes); or ?format=hiddify (URI list)
  Shadowrocket / URI apps  ?format=hiddify&encoding=base64         base64 URI list (plain text is rejected by Shadowrocket)
  v2rayNG (VLESS only)     ?format=uri                             Hysteria2 lines are present but unsupported by the app
"""
from __future__ import annotations

import argparse
import base64
import json
import re
import subprocess
import sys
import tempfile
import urllib.parse

B64URL43 = re.compile(r"^[A-Za-z0-9_-]{43}$")
HEX_SID = re.compile(r"^[0-9a-fA-F]{2,16}$")
SECRET_WORDS = re.compile(r"private[_-]?key|PRIVATE KEY", re.I)


def chk(name, ok, detail=""):
    return {"check": name, "result": "PASS" if ok else "FAIL", "detail": detail}


def reality_ok(o: dict, host: str, port: int | None = None):
    tls = o.get("tls", {})
    rl = tls.get("reality", {})
    return [
        chk("reality: server is the node's public host", o.get("server") == host, f"server={o.get('server')}"),
        chk("reality: port set", isinstance(o.get("server_port"), int) and (port is None or o["server_port"] == port), f"port={o.get('server_port')}"),
        chk("reality: public_key is 43-char base64url", bool(B64URL43.match(str(rl.get("public_key", "")))), "(value not printed)"),
        chk("reality: short_id is hex", bool(HEX_SID.match(str(rl.get("short_id", "")))), "(value not printed)"),
        chk("reality: flow xtls-rprx-vision", o.get("flow") == "xtls-rprx-vision", f"flow={o.get('flow')}"),
        chk("reality: uTLS fingerprint set", bool(tls.get("utls", {}).get("enabled") and tls["utls"].get("fingerprint")), ""),
        chk("reality: SNI present", bool(tls.get("server_name")), f"sni={tls.get('server_name')}"),
        chk("reality: uuid present", bool(o.get("uuid")), "(not printed)"),
    ]


def hysteria_ok(o: dict, host: str):
    tls = o.get("tls", {})
    obfs = o.get("obfs")
    return [
        chk("hysteria2: server is the node's public host", o.get("server") == host, f"server={o.get('server')}"),
        chk("hysteria2: TLS verification is NOT disabled", tls.get("insecure") is False or tls.get("insecure") is None, f"insecure={tls.get('insecure')}"),
        chk("hysteria2: SNI equals the host (certificate name)", tls.get("server_name") == host, f"sni={tls.get('server_name')}"),
        chk("hysteria2: password present", bool(o.get("password")), "(not printed)"),
        chk("hysteria2: Salamander obfuscation present", bool(obfs) and (obfs.get("type") == "salamander" if isinstance(obfs, dict) else True), ""),
    ]


def check_singbox_doc(doc: dict, host: str, pinned: bool):
    out = []
    ob = {o.get("tag"): o for o in doc.get("outbounds", [])}
    types = [o.get("type") for o in doc.get("outbounds", [])]
    out.append(chk("no private key material anywhere", not SECRET_WORDS.search(json.dumps(doc)), ""))
    out.append(chk("no literal IPv6 server (host name only)", all(not str(o.get("server", "")).startswith("[") and ":" not in str(o.get("server", "")) for o in doc["outbounds"] if "server" in o), ""))
    vless = [o for o in doc["outbounds"] if o.get("type") == "vless"]
    hy = [o for o in doc["outbounds"] if o.get("type") == "hysteria2"]
    out.append(chk("exactly one REALITY outbound", len(vless) == 1, f"types={types}"))
    out += reality_ok(vless[0], host) if vless else []
    sel = next((o for o in doc["outbounds"] if o.get("type") == "selector"), None)
    if pinned:
        # Hiddify rebuilds its own groups and balances new connections across ROUTES: the profile must offer exactly one route.
        groups = [o for o in doc["outbounds"] if o.get("type") in ("urltest", "selector", "fallback")]
        members = {m for g in groups for m in g.get("outbounds", [])}
        routes = members & {o.get("tag") for o in doc["outbounds"] if o.get("type") in ("vless", "hysteria2", "trojan", "vmess", "shadowsocks")}
        out.append(chk("pinned: exactly one route (REALITY) is reachable from any group; no Hysteria2", not hy and routes == {vless[0].get("tag")} if vless else False, f"routes={sorted(routes)} types={types}"))
        out.append(chk("pinned: selector defaults to REALITY", bool(sel) and sel.get("default") == vless[0].get("tag"), ""))
    else:
        out.append(chk("exactly one Hysteria2 outbound", len(hy) == 1, f"types={types}"))
        out += hysteria_ok(hy[0], host) if hy else []
        out.append(chk("selector defaults deterministically to REALITY", bool(sel) and sel.get("default") == vless[0].get("tag"), ""))
        out.append(chk("auto (urltest) group exists but is not the default", any(o.get("type") == "urltest" for o in doc["outbounds"]) and sel.get("default") != "auto", ""))
    out.append(chk("route.final points at the selector", doc.get("route", {}).get("final") == (sel or {}).get("tag"), f"final={doc.get('route', {}).get('final')}"))
    return out


def parse_uris(text: str, host: str):
    out, seen = [], {}
    lines = [l.strip() for l in text.splitlines() if l.strip()]
    out.append(chk("no private key material", not SECRET_WORDS.search(text), ""))
    for l in lines:
        u = urllib.parse.urlsplit(l)
        q = dict(urllib.parse.parse_qsl(u.query))
        seen[u.scheme] = (u, q)
    out.append(chk("contains a vless and a hysteria2 link", {"vless", "hysteria2"} <= set(seen), f"schemes={sorted(seen)}"))
    if "vless" in seen:
        u, q = seen["vless"]
        out += [chk("vless link: host", u.hostname == host, f"host={u.hostname}"), chk("vless link: security=reality", q.get("security") == "reality"),
                chk("vless link: flow vision", q.get("flow") == "xtls-rprx-vision"), chk("vless link: pbk 43-char", bool(B64URL43.match(q.get("pbk", "")))),
                chk("vless link: sid hex", bool(HEX_SID.match(q.get("sid", "")))), chk("vless link: fp and sni", bool(q.get("fp") and q.get("sni"))),
                chk("vless link: encryption=none", q.get("encryption") == "none")]
    if "hysteria2" in seen:
        u, q = seen["hysteria2"]
        out += [chk("hysteria2 link: host", u.hostname == host), chk("hysteria2 link: sni equals host", q.get("sni") == host),
                chk("hysteria2 link: insecure=0", q.get("insecure") == "0"), chk("hysteria2 link: obfs and obfs-password",
                                                                              q.get("obfs") == "salamander" and bool(q.get("obfs-password")))]
    return out


def fetch(url: str, extra: str = "") -> dict:
    with tempfile.NamedTemporaryFile(delete=False) as bf, tempfile.NamedTemporaryFile(delete=False) as hf:
        pass
    p = subprocess.run(["curl", "-sS", "-m", "20", "-o", bf.name, "-D", hf.name, "-w", "%{http_code}", url + extra], capture_output=True, text=True)
    body = open(bf.name, "rb").read().decode("utf-8", "replace")
    hdr = {l.split(":", 1)[0].lower(): l.split(":", 1)[1].strip() for l in open(hf.name).read().splitlines() if ":" in l}
    import os
    os.unlink(bf.name); os.unlink(hf.name)
    return {"code": p.stdout.strip(), "body": body, "headers": hdr, "curl_rc": p.returncode, "curl_err": p.stderr.strip()[:120]}


def http_checks(r: dict):
    h = r["headers"]
    return [chk("TLS certificate verified by curl (no -k)", r["curl_rc"] == 0, r["curl_err"]),
            chk("HSTS header present", "strict-transport-security" in h, ""),
            chk("response is not cacheable by intermediaries", "no-store" in h.get("cache-control", "") or "private" in h.get("cache-control", ""), f"cache-control={h.get('cache-control')}"),
            chk("no server version banner", not re.search(r"nginx/\d", h.get("server", "")), f"server={h.get('server')}")]


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--host", required=True)
    ap.add_argument("--sing-box", default="sing-box")
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    url = open(a.url_file).read().strip()
    rep = {"kind": "client-profile-check", "evidence_class": "server verified (profiles only; native apps UNVERIFIED)", "families": {}}

    def fam(name, extra, kind, pinned=False):
        r = fetch(url, extra)
        checks = [chk("HTTP 200", r["code"] == "200", f"code={r['code']}")] + http_checks(r)
        if r["code"] == "200":
            if kind == "singbox":
                try:
                    doc = json.loads(r["body"])
                    checks += check_singbox_doc(doc, a.host, pinned)
                    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
                        f.write(r["body"])
                    p = subprocess.run([a.sing_box, "check", "-c", f.name], capture_output=True, text=True)
                    checks.append(chk("real sing-box accepts the profile (sing-box check)", p.returncode == 0, p.stderr.strip()[:120]))
                    import os; os.unlink(f.name)
                except ValueError:
                    checks.append(chk("body is JSON", False))
            elif kind == "uri":
                checks += parse_uris(r["body"], a.host)
            elif kind == "b64":
                try:
                    checks += parse_uris(base64.b64decode(r["body"], validate=True).decode(), a.host)
                except Exception:  # noqa: BLE001
                    checks.append(chk("body is valid base64 of the URI list", False))
        rep["families"][name] = {"url_suffix": extra, "checks": checks, "native_app": "UNVERIFIED"}

    fam("sing-box", "?format=singbox", "singbox")
    fam("hiddify-pinned (REALITY only)", "?format=singbox&compat=hiddify-pinned", "singbox", pinned=True)
    fam("hiddify URI list", "?format=hiddify", "uri")
    fam("shadowrocket base64", "?format=hiddify&encoding=base64", "b64")
    fam("v2rayNG uri", "?format=uri", "uri")
    # guard rails: invalid requests must be refused, not silently degraded
    for name, extra in (("unknown compat is refused", "?format=singbox&compat=nope"), ("pinned with a URI format is refused", "?format=hiddify&compat=hiddify-pinned"),
                        ("unknown format is refused", "?format=nope")):
        r = fetch(url, extra)
        rep["families"].setdefault("refusals", {"checks": [], "native_app": "N/A"})["checks"].append(chk(name, r["code"] == "400", f"code={r['code']}"))
    allc = [c for f in rep["families"].values() for c in f["checks"]]
    rep["totals"] = {"PASS": sum(c["result"] == "PASS" for c in allc), "FAIL": sum(c["result"] == "FAIL" for c in allc)}
    rep["verdict"] = "PASS" if not rep["totals"]["FAIL"] else "FAIL"
    s = json.dumps(rep, indent=1)
    if a.out:
        with open(a.out, "w") as f:
            f.write(s + "\n")
    print(json.dumps({"verdict": rep["verdict"], **rep["totals"], "families": {k: sum(c["result"] == "FAIL" for c in v["checks"]) for k, v in rep["families"].items()}}))
    return 0 if rep["verdict"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
