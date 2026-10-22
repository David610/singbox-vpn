#!/usr/bin/env python3
"""C-16 own-address isolation THROUGH real tunnels (I-31).

For each protocol (REALITY and Hysteria2) start a real sing-box client from the served subscription and try to
reach, THROUGH the tunnel: the node's own public IPv4 and IPv6 on a service that answers (SSH banner), the
node's loopback subscription backend, and a control destination that must stay reachable.
PASS needs: every prohibited target REJECTED (connection closed with no data / refused) and the control reachable.

    c16_isolation_check.py --url-file F --own-ipv4 A --own-ipv6 B [--loopback-port 9100] --out JSON

Evidence class: server verified (loopback client on the node). The subscription URL is read from a file, never printed.
"""
import argparse, datetime, importlib.util, json, os, socket, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("sa", os.path.join(HERE, "subscription_acceptance.py"))
sa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sa)


def socks_probe(port, host, dport, timeout=6):
    """('rejected'|'reached'|'connected-silent'|'error', detail) for a SOCKS5 CONNECT to host:dport."""
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    except OSError as e:
        return "error", f"proxy unreachable {e}"
    s.settimeout(timeout)
    try:
        s.sendall(b"\x05\x01\x00"); s.recv(2)
        try:
            req = b"\x05\x01\x00\x04" + socket.inet_pton(socket.AF_INET6, host)
        except OSError:
            req = b"\x05\x01\x00\x01" + socket.inet_aton(host)
        s.sendall(req + dport.to_bytes(2, "big"))
        rep = s.recv(10)
        if len(rep) < 2 or rep[1] != 0:
            return "rejected", f"socks reply {rep[1] if len(rep) > 1 else 'none'}"
        s.settimeout(4)
        try:
            data = s.recv(64)
        except socket.timeout:
            return "connected-silent", "no data within 4 s"
        return ("reached", data[:12].decode(errors="replace").strip()) if data else ("rejected", "connection closed with no data")
    except OSError as e:
        return "rejected", type(e).__name__
    finally:
        s.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url-file", required=True)
    ap.add_argument("--own-ipv4", required=True)
    ap.add_argument("--own-ipv6")
    ap.add_argument("--loopback-port", type=int, default=9100)
    ap.add_argument("--control", default="1.1.1.1:443")
    ap.add_argument("--sing-box", default="/usr/local/bin/sing-box")
    ap.add_argument("--out")
    a = ap.parse_args()
    url = open(a.url_file).read().strip()
    doc = json.loads(sa.fetch(url.split("?")[0] + "?format=singbox")[2])
    chost, cport = a.control.rsplit(":", 1)
    targets = [("own IPv4 :22", a.own_ipv4, 22, True), ("loopback subscription backend", "127.0.0.1", a.loopback_port, True)]
    if a.own_ipv6:
        targets.insert(1, ("own IPv6 :22", a.own_ipv6, 22, True))
    targets.append((f"control {a.control}", chost, int(cport), False))
    rows, ok_all = [], True
    with tempfile.TemporaryDirectory() as wd:
        for tag in ("Reality", "Hysteria2"):
            port = sa.free_port()
            cfg = json.loads(json.dumps(doc)); cfg["log"] = {"level": "error"}
            cfg["inbounds"] = [{"type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": port}]
            cfg["route"]["final"] = tag
            path = os.path.join(wd, tag + ".json"); json.dump(cfg, open(path, "w"))
            proc = subprocess.Popen([a.sing_box, "run", "-c", path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                time.sleep(2.5)
                for name, host, dport, prohibited in targets:
                    res, detail = socks_probe(port, host, dport)
                    good = (res == "rejected") if prohibited else (res in ("reached", "connected-silent"))
                    ok_all &= good
                    rows.append({"protocol": tag, "target": name, "prohibited": prohibited, "result": res, "detail": detail, "pass": good})
                    print(f"{'PASS' if good else 'FAIL'}  {tag:9} {name:34} {res} ({detail})")
            finally:
                proc.terminate(); proc.wait(5)
    out = {"schema": 1, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
           "evidence_class": "server-verified (loopback client on the node)", "pass": ok_all, "rows": rows}
    if a.out:
        json.dump(out, open(a.out, "w"), indent=1)
    print("RESULT:", "PASS" if ok_all else "FAIL")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
