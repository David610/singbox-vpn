"""Run Hiddify's own `parse` (ParseConfigBytes + libbox.CheckConfig) on real subscription bodies."""
import base64
import json
import os
import subprocess
import sys
import urllib.request

SP = os.path.dirname(os.path.abspath(__file__))
CLI = os.path.join(SP, "hiddify-cli.exe")
WORK = os.path.join(SP, "hcli-work")
os.makedirs(WORK, exist_ok=True)
cred = json.load(open(sys.argv[1]))
base = cred["subscription_url"].split("?")[0]


def fetch(q: str) -> bytes:
    return urllib.request.urlopen(base + q, timeout=20).read()


plain = fetch("?format=hiddify")
links = plain.decode().strip().splitlines()
vless = next(l for l in links if l.startswith("vless://"))
hy2 = next(l for l in links if l.startswith("hysteria2://"))
bad_hy2 = hy2.replace("sni=vpn.xn----8sba1cvaif4e.com", "sni=www.cloudflare.com").split("&obfs=")[0]
cases = {
    "A hiddify plain share links (vless+hy2)": plain,
    "B hiddify base64 body": fetch("?format=hiddify&encoding=base64"),
    "C shadowrocket base64 (format=uri)": fetch("?format=uri&encoding=base64"),
    "D bare /sub (default sing-box JSON, outbounds+route only)": fetch(""),
    "E hiddify-pinned JSON (REALITY only)": fetch("?format=singbox&compat=hiddify-pinned"),
    "F single REALITY link": (vless + "\n").encode(),
    "G single Hysteria2 link": (hy2 + "\n").encode(),
    "H old vpn-web-shaped Hysteria2 link (decoy sni, no obfs)": (bad_hy2 + "\n").encode(),
    "I malformed body": b"this is not a subscription\n",
    "J empty body": b"",
}
rows = []
for name, body in cases.items():
    fn = os.path.join(WORK, f"case-{name[0]}.txt")
    open(fn, "wb").write(body)
    out = os.path.join(WORK, f"out-{name[0]}.json")
    if os.path.exists(out):
        os.remove(out)
    p = subprocess.run([CLI, "-D", WORK, "--disable-color", "parse", os.path.basename(fn), "-o", os.path.basename(out)], capture_output=True, text=True, timeout=120, cwd=WORK)
    summary = ""
    if os.path.exists(out):
        d = json.load(open(out))
        outs = [(o.get("type"), o.get("tag")) for o in d.get("outbounds", [])]
        summary = f"top-level keys={sorted(d)} outbounds={len(outs)} protocols={sorted({t for t, _ in outs if t not in ('selector','urltest','direct','dns','block')})} has_inbounds={'inbounds' in d}"
    msg = (p.stdout + p.stderr).strip().splitlines()
    msg = [m for m in msg if "result successfully" in m or "ERROR" in m or "FATAL" in m or "error" in m.lower() or "fail" in m.lower()][:2]
    rows.append({"case": name, "exit": p.returncode, "summary": summary, "message": [m[:200] for m in msg]})
json.dump(rows, open(sys.argv[2], "w"), indent=1)
for r in rows:
    print(f"{r['case']:<62} exit={r['exit']}  {r['summary']}  {r['message']}")
