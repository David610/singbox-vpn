"""Fetch every subscription variant from a live server and describe the real bytes (secrets masked)."""
import base64
import json
import re
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request

SECRET_FILE, OUT = sys.argv[1], sys.argv[2]
base_url = json.load(open(SECRET_FILE))["subscription_url"].split("?")[0]

VARIANTS = [
    "",
    "?format=singbox",
    "?format=hiddify",
    "?format=uri",
    "?format=uri&encoding=base64",
    "?format=hiddify&encoding=base64",
    "?format=singbox&encoding=base64",
    "?format=singbox&compat=hiddify-pinned",
    "?format=links",
    "?format=base64",
    "?format=clash",
]


def mask(text: str) -> str:
    text = re.sub(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "<UUID>", text)
    text = re.sub(r"(pbk=)[A-Za-z0-9_%-]+", r"\1<PBK>", text)
    text = re.sub(r"(sid=)[0-9a-fA-F]+", r"\1<SID>", text)
    text = re.sub(r"(obfs-password=)[^&#\s]+", r"\1<OBFS>", text)
    text = re.sub(r"(hysteria2://)[^@]+@", r"\1<AUTH>@", text)
    return text


def describe_body(body: bytes) -> str:
    notes = []
    if body.startswith(b"\xef\xbb\xbf"):
        notes.append("UTF8-BOM")
    if b"\r\n" in body:
        notes.append("CRLF")
    text = body.decode("utf-8", "replace")
    stripped = text.strip()
    if stripped.startswith("{"):
        try:
            doc = json.loads(stripped)
            outs = [(o.get("type"), o.get("tag")) for o in doc.get("outbounds", [])]
            notes.append(f"JSON keys={sorted(doc)[:8]} outbounds={outs}")
        except json.JSONDecodeError as exc:
            notes.append(f"BAD-JSON {exc}")
    elif re.fullmatch(r"[A-Za-z0-9+/=\r\n]+", stripped or "x") and len(stripped) > 20:
        try:
            dec = base64.b64decode(stripped + "=" * (-len(stripped) % 4)).decode("utf-8", "replace")
            lines = [ln for ln in dec.splitlines() if ln.strip()]
            notes.append(f"BASE64 -> {len(lines)} line(s)")
            for ln in lines:
                notes.append("   " + mask(ln)[:230])
        except Exception as exc:  # noqa: BLE001
            notes.append(f"base64-decode-failed {exc}")
    else:
        lines = [ln for ln in text.splitlines() if ln.strip()]
        notes.append(f"PLAINTEXT {len(lines)} line(s)")
        for ln in lines:
            notes.append("   " + mask(ln)[:230])
    return "\n".join(notes)


ctx = ssl.create_default_context()
report = []
for q in VARIANTS:
    req = urllib.request.Request(base_url + q, headers={"User-Agent": "probe/1"})
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=20) as r:
            status, headers, body = r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        status, headers, body = e.code, dict(e.headers), e.read()
    interesting = {k.lower(): v for k, v in headers.items() if k.lower() in (
        "content-type", "profile-title", "profile-update-interval", "subscription-userinfo",
        "content-disposition", "cache-control", "profile-web-page-url", "support-url")}
    report.append(f"=== GET /sub/<TOKEN>{q}\nstatus={status} bytes={len(body)} headers={interesting}\n{describe_body(body)}\n")
open(OUT, "w", encoding="utf-8").write("\n".join(report))
print("\n".join(report))
