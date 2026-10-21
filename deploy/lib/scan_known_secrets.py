#!/usr/bin/env python3
"""Search a git range and a tree for the node's REAL secret values (never prints a value: labels and lengths only).

  scan_known_secrets.py [--range origin/main..HEAD] [--rev HEAD] [--secret-file LABEL=PATH ...] [--users-json PATH] [--url-file PATH]

Defaults read the staging node's secret files (REALITY private key, Hysteria2 obfuscation password, user credentials in
users.json, QA subscription URL parts). Exit 1 if any value is present in the scanned commits or tree; exit 2 if no secret value
could be collected (a scan that checked nothing must not look clean). Generic pattern scanning is NOT done here: the point is
to prove the specific live credentials never reached the repository.
"""
import argparse
import json
import re
import subprocess
import sys

HEX64 = re.compile(r"[0-9a-f]{64}")


def collect(secret_files, users_json, url_file):
    vals = {}

    def put(label, v):
        v = (v or "").strip()
        if len(v) >= 16 and not HEX64.fullmatch(v) and v not in vals.values():
            vals[f"{label}#{len(vals) + 1}"] = v

    for spec in secret_files:
        label, _, path = spec.partition("=")
        try:
            put(label, open(path, encoding="utf-8", errors="ignore").read())
        except OSError:
            pass

    def walk(x):
        if isinstance(x, dict):
            for k, v in x.items():
                if isinstance(v, str) and re.search(r"token|uuid|password|secret|key", k, re.I):
                    put(f"users.{k}", v)
                else:
                    walk(v)
        elif isinstance(x, list):
            for i in x:
                walk(i)

    if users_json:
        try:
            walk(json.load(open(users_json, encoding="utf-8")))
        except (OSError, ValueError):
            pass
    if url_file:
        try:
            for m in re.finditer(r"[A-Za-z0-9_-]{20,}", open(url_file, encoding="utf-8", errors="ignore").read()):
                put("subscription-url-part", m.group())
        except OSError:
            pass
    return vals


def git_text(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, errors="ignore").stdout


def scan(vals, rng, rev):
    corpora = {}
    if rng:
        corpora[f"commits {rng}"] = git_text("log", "-p", "--no-color", rng)
    if rev:
        corpora[f"tree {rev}"] = git_text("grep", "-I", "-h", "", rev)
    return {k: [name for name, text in corpora.items() if v in text] for k, v in vals.items()}


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--range", default="origin/main..HEAD")
    ap.add_argument("--rev", default="HEAD")
    ap.add_argument("--secret-file", action="append", default=None)
    ap.add_argument("--users-json", default="/etc/vpn/compat/users/users.json")
    ap.add_argument("--url-file", default="/root/qa-secrets/url.txt")
    a = ap.parse_args(argv)
    files = a.secret_file if a.secret_file is not None else [
        "reality-private-key=/etc/vpn/compat/reality/private.key",
        "hysteria2-obfs-password=/etc/vpn/compat/reality/hysteria_obfs_password.txt"]
    vals = collect(files, a.users_json, a.url_file)
    if not vals:
        print("NO SECRET VALUES COLLECTED: nothing was checked", file=sys.stderr)
        return 2
    res = scan(vals, a.range, a.rev)
    bad = {k: w for k, w in res.items() if w}
    for k, v in vals.items():
        print(f"{k:<32} len={len(v):<3} {'FOUND in ' + ', '.join(res[k]) if res[k] else 'not found'}")
    print(f"checked {len(vals)} secret value(s): {'FAIL, ' + str(len(bad)) + ' present in the repository' if bad else 'none present'}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
