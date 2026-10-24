#!/usr/bin/env python3
"""Runs ON the candidate VPS for candidate_qualify.sh. Python 3.9+ standard library only.

  qualify_remote.py collect --out FILE
      the candidate's own public IPv4 and its view of the Google/YouTube addresses
  qualify_remote.py survey --ips FILE --out FILE
      TLS/SNI survey over the given addresses plus informational IPv6 and QUIC probes

Output is ONE JSON document written to --out; stdout/stderr are for humans only and are never parsed.
The sibling scripts (collect_ips.py, tls_survey.py, v6_check.py, quic_vn_probe.py) are uploaded next to
this file. Nothing is installed and nothing is left running.
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
IP_ECHO = "http://api.ipify.org"


def load(name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def egress_ipv4():
    try:
        with urllib.request.urlopen(IP_ECHO, timeout=8) as resp:
            text = resp.read(64).decode("ascii", "replace").strip()
    except (OSError, ValueError):
        return None
    parts = text.split(".")
    if len(parts) == 4 and all(p.isdigit() and 0 <= int(p) <= 255 for p in parts):
        return text
    return None


def run_text(script, *args, timeout=60):
    try:
        done = subprocess.run([sys.executable, "-I", os.path.join(HERE, script), *args], capture_output=True,
                              text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "text": ""}
    return {"status": "ok" if done.returncode == 0 else "failed", "text": done.stdout[-4000:]}


def collect(out):
    import socket
    ips = set()
    names = ["www.youtube.com", "m.youtube.com", "youtubei.googleapis.com", "i.ytimg.com", "i9.ytimg.com",
             "yt3.ggpht.com", "s.youtube.com", "redirector.googlevideo.com", "manifest.googlevideo.com",
             "www.google.com", "www.gstatic.com", "accounts.google.com", "play.google.com", "fonts.gstatic.com",
             "lh3.googleusercontent.com", "jnn-pa.googleapis.com", "rr1.googlevideo.com", "rr2.googlevideo.com"]
    for name in names:
        try:
            for info in socket.getaddrinfo(name, 443, socket.AF_INET):
                ips.add(info[4][0])
        except OSError:
            pass
    doc = {"schema": 1, "mode": "collect", "egress_ipv4": egress_ipv4(), "ips": sorted(ips),
           "python": "%d.%d.%d" % sys.version_info[:3]}
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(doc, fh)
    return 0


def survey(ips_path, out):
    with open(ips_path, encoding="utf-8") as fh:
        ips = json.load(fh)
    if not isinstance(ips, list) or not ips:
        sys.stderr.write("survey: --ips must be a non-empty JSON array\n")
        return 2
    rows = load("tls_survey").survey(ips)
    doc = {"schema": 1, "mode": "survey", "egress_ipv4": egress_ipv4(), "tls": {"rows": rows, "ips": len(ips)},
           "ipv6": run_text("v6_check.py"), "quic": run_text("quic_vn_probe.py", "www.youtube.com")}
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(doc, fh)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="mode", required=True)
    c = sub.add_parser("collect")
    c.add_argument("--out", required=True)
    s = sub.add_parser("survey")
    s.add_argument("--ips", required=True)
    s.add_argument("--out", required=True)
    args = ap.parse_args()
    return collect(args.out) if args.mode == "collect" else survey(args.ips, args.out)


if __name__ == "__main__":
    sys.exit(main())
