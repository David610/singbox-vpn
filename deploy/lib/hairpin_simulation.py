#!/usr/bin/env python3
"""Simulated TWO-NODE Google-egress hairpin test (local, no other host).

Everything runs inside a throwaway network namespace, so the host's interfaces,
services and addresses are never touched. The REAL `vpn-admin` renders both
nodes' sing-box configs (with a stub systemctl, so no live service is reloaded)
and the hairpin is configured with the REAL `vpn-admin google-egress-hairpin
set`; real sing-box processes play client, exit E, hairpin target T and a TLS
destination D that records which source address connected and with which SNI.

    client --REALITY--> E (192.88.99.3) --direct--------------------> D (192.88.99.5)   non-Google SNI
                          \\--hairpin REALITY--> T (192.88.99.4) -----> D                  Google SNI

Evidence class: SIMULATED, local. It proves the rendered routing + fail-closed
behaviour end to end with real sing-box; it proves NOTHING about whether YouTube
accepts any real address (that is the C1-C8 qualification in
docs/operations/HAIRPIN_TARGET_QUALIFICATION.md). Needs root (ip netns).
"""
import argparse
import json
import os
import shutil
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time

NS = "hpsim"
IPS = {"E": "192.88.99.3", "T": "192.88.99.4", "D": "192.88.99.5", "C": "192.88.99.6"}
results = []


def rec(name, ok, detail=""):
    results.append({"check": name, "pass": bool(ok), "detail": detail})
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"  ({detail})" if detail else ""))


def sh(cmd, **kw):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True, **kw)


def ns(cmd):
    return f"ip netns exec {NS} {cmd}"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Dest(threading.Thread):
    """TLS destination run INSIDE the namespace (as a subprocess) that logs peer ip + SNI."""


DEST_SRC = r'''
import socket, ssl, sys, json
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(sys.argv[1], sys.argv[2])
seen = {"sni": None}
def cb(sock, name, c): seen["sni"] = name
ctx.sni_callback = cb
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind((sys.argv[3], 9443)); s.listen(64)
log = open(sys.argv[4], "a", buffering=1)
while True:
    c, addr = s.accept()
    try:
        seen["sni"] = None
        t = ctx.wrap_socket(c, server_side=True)
        t.recv(4096); t.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"); t.close()
        log.write(json.dumps({"peer": addr[0], "sni": seen["sni"]}) + "\n")
    except Exception as e:
        log.write(json.dumps({"peer": addr[0], "sni": seen["sni"], "error": type(e).__name__}) + "\n")
'''


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vpn-admin", default="/usr/local/bin/vpn-admin")
    ap.add_argument("--sing-box", default="/usr/local/bin/sing-box")
    ap.add_argument("--mutate", choices=["none", "direct-route", "fallback"], default="none",
                    help="self-test: break E's rendered config on purpose; the harness MUST then fail")
    ap.add_argument("--out")
    a = ap.parse_args()
    if os.geteuid() != 0:
        print("must run as root (ip netns)"); return 1
    W = tempfile.mkdtemp(prefix="hpsim-")
    procs = []

    def spawn(cmd, log=None):
        p = subprocess.Popen(ns(cmd), shell=True, stdout=open(log or os.devnull, "w"), stderr=subprocess.STDOUT, start_new_session=True)
        procs.append(p)
        return p

    def kill(p):
        try:
            os.killpg(p.pid, 15)
        except ProcessLookupError:
            pass
        try:
            p.wait(5)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, 9)

    try:
        sh(f"ip netns del {NS} 2>/dev/null")
        assert sh(f"ip netns add {NS}").returncode == 0, "cannot create netns"
        sh(ns("ip link set lo up"))
        sh(ns("ip link add sim0 type dummy")); sh(ns("ip link set sim0 up"))
        sh(ns("ip route add default dev sim0"))           # sing-box wants a default interface; nothing can leave the namespace
        for ip in IPS.values():
            sh(ns(f"ip addr add {ip}/32 dev sim0"))
        # stub systemctl: the CLI may only believe a unit exists, never touch a real one
        stub = f"{W}/fake-systemctl"
        open(stub, "w").write('#!/bin/sh\ncase "$*" in *"-p LoadState"*) echo loaded;; *"-p ActiveState"*) echo active;; *"-p SubState"*) echo running;; *"-p MainPID"*) echo 4242;; *"-p NRestarts"*) echo 0;; *"-p Result"*) echo success;; *"-p InvocationID"*) echo 00000000000000000000000000000001;; *"-p "*"Timestamp"*) echo 123456;; *is-active*) echo active;; esac\nexit 0\n')
        os.chmod(stub, 0o755)
        # a private state lock: the host's /run/lock/singbox-vpn.lock belongs to a real install (or, on a CI runner, may already
        # exist owned by another user, which root cannot open under fs.protected_regular: "init T: Permission denied")
        env = dict(os.environ, SINGBOX_VPN_SYSTEMCTL=stub, SINGBOX_VPN_LOCK_PATH=f"{W}/state.lock")
        decoy = {}
        sh(f"openssl req -x509 -newkey rsa:2048 -nodes -keyout {W}/k.pem -out {W}/c.pem -days 2 -subj /CN=sim >/dev/null 2>&1")
        for n in ("T", "E"):
            os.makedirs(f"{W}/{n}/state/hysteria")
            sh(f"cp {W}/c.pem {W}/{n}/state/hysteria/cert.pem; cp {W}/k.pem {W}/{n}/state/hysteria/key.pem")
            decoy[n] = free_port()
            open(f"{W}/{n}/deployment.toml", "w").write(f'''schema_version = 2
node_id = "{n.lower()}"
role = "exit"
public_host = "sim-{n.lower()}.example.com"
subscription_host = "sim-{n.lower()}.example.com"
public_ipv4 = "{IPS[n]}"
state_dir = "{W}/{n}/state"
singbox_binary = "{a.sing_box}"

[reality]
listen_port = {443 if n == "T" else 8444}
handshake_server = "www.example.com"
handshake_port = 443

[hysteria2]
listen_port = 4443

[subscription]
listen_port = 9100
public_port = 8443
''')
            spawn(f"openssl s_server -accept {decoy[n]} -cert {W}/c.pem -key {W}/k.pem -tls1_3 -quiet -www")

        def admin(node, *args, stdin=None):
            return subprocess.run([a.vpn_admin, "--config", f"{W}/{node}/deployment.toml", *args], input=stdin, capture_output=True, text=True, env=env)

        for n in ("T", "E"):
            r = admin(n, "init")
            assert r.returncode == 0, f"init {n}: rc={r.returncode} stderr={r.stderr[-800:]!r} stdout={r.stdout[-400:]!r}"
        for n, user in (("T", "hairpin-e"), ("E", "alice")):
            r = admin(n, "user", "create", "--name", user, "--json")
            assert r.returncode == 0, f"user create {n}: {r.stderr[-200:]}"
        users = {n: json.load(open(f"{W}/{n}/state/users/users.json"))["users"] for n in ("T", "E")}
        t_uuid = users["T"][-1]["vless_uuid"] if users["T"][-1]["name"] == "hairpin-e" else next(u for u in users["T"] if u["name"] == "hairpin-e")["vless_uuid"]
        c_uuid = next(u for u in users["E"] if u["name"] == "alice")["vless_uuid"]
        pub = {n: open(f"{W}/{n}/state/reality/public.key").read().strip() for n in ("T", "E")}
        sid = {n: open(f"{W}/{n}/state/reality/short_id.txt").read().strip() for n in ("T", "E")}
        # the REAL hairpin command (uuid on stdin, never argv)
        r = admin("E", "google-egress-hairpin", "set", "--relay-host", IPS["T"], "--relay-port", "443", "--relay-server-name", "www.example.com",
                  f"--relay-reality-public-key={pub['T']}", "--relay-reality-short-id", sid["T"], "--uuid-stdin", stdin=t_uuid + "\n")
        rec("vpn-admin google-egress-hairpin set accepts the target and renders", r.returncode == 0, (r.stderr or r.stdout)[-160:].replace(t_uuid, "<uuid>"))
        if r.returncode != 0:
            raise SystemExit(1)

        def patch(n):
            p = f"{W}/{n}/state/sing-box/config.json"
            c = json.load(open(p))
            for ib in c["inbounds"]:
                if ib["type"] == "vless":
                    ib["tls"]["reality"]["handshake"] = {"server": "127.0.0.1", "server_port": decoy[n]}
            c["inbounds"] = [i for i in c["inbounds"] if i["type"] == "vless"]      # no hysteria2 in the sim
            for o in c["outbounds"]:
                if o.get("tag") == "direct":
                    o["inet4_bind_address"] = IPS[n]
            c["log"] = {"level": "error"}
            json.dump(c, open(f"{W}/{n}.run.json", "w"))
            return c
        ct, ce = patch("T"), patch("E")
        if a.mutate != "none":
            hp0 = [o for o in ce["outbounds"] if o.get("type") == "vless"][0]["tag"]
            for r_ in ce["route"]["rules"]:
                if r_.get("outbound") == hp0:
                    r_["outbound"] = "direct" if a.mutate == "direct-route" else "fb"
            if a.mutate == "fallback":
                ce["outbounds"].append({"type": "urltest", "tag": "fb", "outbounds": [hp0, "direct"], "url": f"http://{IPS['D']}:9080/", "interval": "1s", "tolerance": 1})
            json.dump(ce, open(f"{W}/E.run.json", "w"))
            print(f"MUTATION ACTIVE: {a.mutate}")
        hp = [o for o in ce["outbounds"] if o.get("type") == "vless"]
        rec("E has exactly one hairpin outbound pointing at T", len(hp) == 1 and hp[0]["server"] == IPS["T"], f"{len(hp)} vless outbound(s)")
        rules = ce["route"]["rules"]
        hp_rules = [r for r in rules if r.get("outbound") == hp[0]["tag"]] if hp else []
        rec("exactly one route rule sends the Google domain set to the hairpin", len(hp_rules) == 1, f"{len(hp_rules)} rule(s)")
        rec("route.final stays direct (non-Google traffic is not hairpinned)", ce["route"].get("final") == "direct", str(ce["route"].get("final")))
        for n in ("T", "E"):
            chk = subprocess.run([a.sing_box, "check", "-c", f"{W}/{n}.run.json"], capture_output=True, text=True)
            rec(f"sing-box check accepts the patched {n} config", chk.returncode == 0, chk.stderr[-100:])

        open(f"{W}/dest.py", "w").write(DEST_SRC)
        destlog = f"{W}/dest.log"
        spawn(f"python3 {W}/dest.py {W}/c.pem {W}/k.pem {IPS['D']} {destlog}")
        os.makedirs(f"{W}/www", exist_ok=True)
        spawn(f"python3 -m http.server 9080 --bind {IPS['D']} --directory {W}/www")      # plain-HTTP probe target for the mutation
        pT = spawn(f"{a.sing_box} run -c {W}/T.run.json", f"{W}/T.log")
        pE = spawn(f"{a.sing_box} run -c {W}/E.run.json", f"{W}/E.log")
        cport = 1080
        client = {"log": {"level": "error"}, "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": cport}],
                  "outbounds": [{"type": "vless", "tag": "p", "server": IPS["E"], "server_port": 8444, "uuid": c_uuid, "flow": "xtls-rprx-vision",
                                 "inet4_bind_address": IPS["C"],
                                 "tls": {"enabled": True, "server_name": "www.example.com", "utls": {"enabled": True, "fingerprint": "chrome"},
                                         "reality": {"enabled": True, "public_key": pub["E"], "short_id": sid["E"]}}}],
                  "route": {"final": "p"}}
        json.dump(client, open(f"{W}/C.json", "w"))
        spawn(f"{a.sing_box} run -c {W}/C.json", f"{W}/C.log")
        time.sleep(4)

        # the hairpin leg on its own: a plain client using E's exact hairpin outbound, straight to T
        leg = {"log": {"level": "error"}, "inbounds": [{"type": "mixed", "listen": "127.0.0.1", "listen_port": 1081}],
               "outbounds": [dict(hp[0], tag="leg")], "route": {"final": "leg"}}
        json.dump(leg, open(f"{W}/L.json", "w"))
        spawn(f"{a.sing_box} run -c {W}/L.json", f"{W}/L.log")
        time.sleep(2)
        r = sh(ns(f"curl -s -k -m 12 -o /dev/null -w %{{http_code}} --resolve leg.example.org:9443:{IPS['D']} -x socks5://127.0.0.1:1081 https://leg.example.org:9443/"))
        rec("hairpin leg alone: E's exact hairpin outbound reaches the destination through T", r.stdout.strip() == "200", f"http {r.stdout.strip() or 'none'}")

        def request(sni):
            """TLS request to D carrying SNI `sni` through client -> E. Returns (curl ok, D's log lines added)."""
            before = sum(1 for _ in open(destlog)) if os.path.exists(destlog) else 0
            r = sh(ns(f"curl -s -k -m 12 -o /dev/null -w %{{http_code}} --resolve {sni}:9443:{IPS['D']} -x socks5://127.0.0.1:{cport} https://{sni}:9443/"))
            time.sleep(0.5)
            lines = [json.loads(l) for l in open(destlog)] if os.path.exists(destlog) else []
            return r.stdout.strip() == "200", [x for x in lines[before:] if x.get("sni") == sni]

        google = ["www.youtube.com", "rr1---sn-abc.googlevideo.com", "i.ytimg.com", "www.gstatic.com", "www.google.com", "yt3.ggpht.com"]
        for d in google:
            ok, ev = request(d)
            via = ev[0]["peer"] if ev else None
            rec(f"Google-set SNI {d} exits via the hairpin target (T)", ok and via == IPS["T"], f"destination saw source {via}")
        for d in ("www.example.org", "notgoogle.com", "evilyoutube.com", "google.com.evil.example"):
            ok, ev = request(d)
            via = ev[0]["peer"] if ev else None
            rec(f"non-Google/lookalike SNI {d} exits directly from E, NOT the hairpin", ok and via == IPS["E"], f"destination saw source {via}")

        # fail-closed: the hairpin target dies
        kill(pT)
        time.sleep(4)          # long enough for a (mutated) urltest group to notice and switch
        ok, ev = request("www.youtube.com")
        rec("target DOWN: a Google request fails visibly", not ok, f"curl ok={ok}")
        rec("target DOWN: the destination NEVER sees E's address for a Google SNI (no direct fallback, INV-6)", not any(x["peer"] == IPS["E"] for x in ev), f"destination saw {[x['peer'] for x in ev] or 'nothing'}")
        ok, ev = request("www.example.org")
        rec("target DOWN: non-Google traffic is unaffected", ok and bool(ev) and ev[0]["peer"] == IPS["E"], "")
        # recovery
        pT = spawn(f"{a.sing_box} run -c {W}/T.run.json", f"{W}/T2.log")
        time.sleep(3)
        ok, ev = request("www.youtube.com")
        rec("target back UP: Google requests resume through T", ok and bool(ev) and ev[0]["peer"] == IPS["T"], "")
        # wrong credential on the hairpin -> fail closed
        kill(pE)
        cfg = json.load(open(f"{W}/E.run.json"))
        for o in cfg["outbounds"]:
            if o.get("type") == "vless":
                o["uuid"] = "00000000-0000-4000-8000-000000000000"
        json.dump(cfg, open(f"{W}/E.bad.json", "w"))
        pE = spawn(f"{a.sing_box} run -c {W}/E.bad.json", f"{W}/E2.log")
        time.sleep(3)
        ok, ev = request("www.youtube.com")
        rec("wrong hairpin credential: Google request fails and never falls back to E's address", (not ok) and not any(x["peer"] == IPS["E"] for x in ev), f"curl ok={ok}")
    except SystemExit:
        pass
    finally:
        for p in procs:
            kill(p)
        sh(f"ip netns del {NS} 2>/dev/null")
        if os.environ.get("HPSIM_KEEP"):
            print("kept work dir:", W)
        else:
            shutil.rmtree(W, ignore_errors=True)
    failed = [r for r in results if not r["pass"]]
    doc = {"schema": 1, "evidence_class": "SIMULATED two-node (local netns, real vpn-admin renderer + real sing-box); proves routing/fail-closed only, NOT any real address's YouTube acceptance",
           "passed": len(results) - len(failed), "failed": len(failed), "results": results}
    if a.out:
        json.dump(doc, open(a.out, "w"), indent=1)
    print(f"RESULT: {doc['passed']} passed, {doc['failed']} failed")
    leftover = sh(f"ip netns list | grep -c {NS}").stdout.strip()
    print(f"namespace left behind: {leftover if leftover != '0' else 'none'}")
    return 0 if not failed and results else 1


if __name__ == "__main__":
    sys.exit(main())
