#!/usr/bin/env python3
"""Hermetic stand-ins for ssh / scp / ssh-keyscan used by test-candidate-qualify.sh.

The "candidate" is a directory on this machine. Nothing here touches the network:
invoked as `fake_tool.py ssh|scp|ssh-keyscan ARGS...`, driven by env vars:

  QT_STATE        scratch dir (remote tempdirs, pid files, call log)
  QT_HOSTKEY_PUB  path to an OpenSSH ed25519 .pub file served by ssh-keyscan
  QT_SCENARIO     name of a canned remote/tunnel behaviour (see SCENARIOS)
"""
import json
import os
import re
import signal
import socket
import subprocess
import sys
import threading
import time

STATE = os.environ.get("QT_STATE", ".")
SCENARIO = os.environ.get("QT_SCENARIO", "good")

LOCAL_IPS = ["142.250.1.1", "142.250.1.2", "172.217.0.1"]
REMOTE_IPS = ["142.250.1.2", "216.58.0.9"]

SCENARIOS = {
    "good": {"egress": "203.0.113.7", "tunnel_egress": "203.0.113.7", "tls_ok": 1.0},
    "tunnel_wrong_ip": {"egress": "203.0.113.7", "tunnel_egress": "198.51.100.9", "tls_ok": 1.0},
    "tls_blocked": {"egress": "203.0.113.7", "tunnel_egress": "203.0.113.7", "tls_ok": 0.1},
    "no_egress_ip": {"egress": None, "tunnel_egress": "203.0.113.7", "tls_ok": 1.0},
    "survey_crash": {"egress": "203.0.113.7", "tunnel_egress": "203.0.113.7", "tls_ok": 1.0, "survey_exit": 7},
}


def log(line):
    with open(os.path.join(STATE, "calls.log"), "a", encoding="utf-8") as fh:
        fh.write(line + "\n")


def scenario():
    return SCENARIOS[SCENARIO]


def tls_rows(ok_ratio):
    rows = []
    snis = ["www.youtube.com", "i.ytimg.com", "youtubei.googleapis.com", "rr1---sn-aaaa.googlevideo.com",
            "www.google.com", "example.com", ""]
    n = 0
    for ip in ["142.250.1.1", "142.250.1.2", "172.217.0.1", "216.58.0.9"]:
        for sni in snis:
            n += 1
            control = sni in ("www.google.com", "example.com", "")
            ok = control or (n % 10) / 10.0 < ok_ratio
            rows.append([ip, sni, "ok" if ok else "noreply"])
    return rows


def run_remote(cmd):
    out_match = re.search(r"--out\s+(\S+)", cmd)
    if "mktemp" in cmd:
        base = os.path.join(STATE, "remote")
        os.makedirs(base, exist_ok=True)
        d = subprocess.run(["mktemp", "-d", os.path.join(base, "q.XXXXXX")], capture_output=True, text=True, check=True).stdout.strip()
        os.chmod(d, 0o700)
        print(d)
        return 0
    if "qualify_remote.py" in cmd and out_match:
        mode = "collect" if " collect" in cmd else "survey"
        sc = scenario()
        if mode == "survey" and sc.get("survey_exit"):
            sys.stderr.write("fake candidate: survey crashed\n")
            return sc["survey_exit"]
        if mode == "collect":
            doc = {"schema": 1, "mode": "collect", "egress_ipv4": sc["egress"], "ips": REMOTE_IPS,
                   "python": "3.12.0"}
        else:
            doc = {"schema": 1, "mode": "survey", "egress_ipv4": sc["egress"],
                   "tls": {"rows": tls_rows(sc["tls_ok"])},
                   "ipv6": {"text": "v6 no-AAAA"}, "quic": {"text": "NO-REPLY"}}
        with open(out_match.group(1), "w", encoding="utf-8") as fh:
            json.dump(doc, fh)
        return 0
    m = re.search(r"rm\s+-rf\s+(\S+)", cmd)
    if m:
        log("REMOTE_RM " + m.group(1))
        subprocess.run(["rm", "-rf", m.group(1)], check=False)
        return 0
    return 0


def serve_socks(port, ip):
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(8)

    def handle(c):
        try:
            c.recv(262)
            c.sendall(b"\x05\x00")
            hdr = c.recv(4)
            if hdr[3] == 3:
                n = c.recv(1)[0]
                c.recv(n + 2)
            elif hdr[3] == 1:
                c.recv(6)
            else:
                c.recv(18)
            c.sendall(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00")
            c.recv(4096)
            c.sendall(b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\n" + ip.encode())
        except OSError:
            pass
        finally:
            c.close()

    while True:
        c, _ = srv.accept()
        threading.Thread(target=handle, args=(c,), daemon=True).start()


OPT_WITH_ARG = set("bcDEeFIiJLlmOopQRSWw")


def parse_ssh(argv):
    opts, i, target, dyn, strict = [], 0, None, None, None
    while i < len(argv):
        a = argv[i]
        if a == "-o":
            opts.append(argv[i + 1])
            i += 2
            continue
        if a.startswith("-") and len(a) == 2 and a[1] in OPT_WITH_ARG:
            if a == "-D":
                dyn = argv[i + 1]
            opts.append(a + " " + argv[i + 1] if a != "-i" else "-i <key>")
            i += 2
            continue
        if a.startswith("-"):
            opts.append(a)
            i += 1
            continue
        target = a
        i += 1
        break
    return opts, target, dyn, " ".join(argv[i:])


def main():
    tool, argv = sys.argv[1], sys.argv[2:]
    if tool == "ssh-keyscan":
        host = argv[-1]
        parts = open(os.environ["QT_HOSTKEY_PUB"], encoding="utf-8").read().split()
        log("KEYSCAN " + host)
        print("%s %s %s" % (host, parts[0], parts[1]))
        return 0
    if tool == "scp":
        paths = [a for a in argv if not a.startswith("-") and not re.match(r"^[A-Za-z0-9_.@:-]+=", a)]
        # drop option values: only "-o x" / "-i x" / "-P x" take args
        paths, skip = [], False
        for idx, a in enumerate(argv):
            if skip:
                skip = False
                continue
            if a in ("-o", "-i", "-P", "-F"):
                skip = True
                continue
            if a.startswith("-"):
                continue
            paths.append(a)
        strip = lambda p: p.split(":", 1)[1] if re.match(r"^[^/]*@[^/]*:", p) else p
        *srcs, dst = [strip(p) for p in paths]
        log("SCP " + " ".join(os.path.basename(s) for s in srcs) + " -> " + os.path.basename(dst))
        return subprocess.run(["cp", "-f", *srcs, dst]).returncode
    opts, target, dyn, cmd = parse_ssh(argv)
    log("SSH " + " ".join(opts) + " " + (target or "") + " " + cmd)
    if dyn:
        port = int(dyn.rsplit(":", 1)[-1])
        with open(os.path.join(STATE, "tunnel.pid"), "w", encoding="utf-8") as fh:
            fh.write(str(os.getpid()))
        signal.signal(signal.SIGTERM, lambda *_: os._exit(0))
        ip = scenario()["tunnel_egress"]
        serve_socks(port, ip)
        return 0
    return run_remote(cmd)


if __name__ == "__main__":
    sys.exit(main())
