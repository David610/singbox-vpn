#!/usr/bin/env python3
"""Address-family capture for the Shorts A/B experiment: privacy-minimized, time-binned aggregates only.

RECONSTRUCTED 2026-10-22 from the documented requirements. The corrected original was lost with the fin1 rebuild and is
MISSING. It fixed two parser defects, both covered by tests here:
  * the IPv6 Google prefix list was incomplete;
  * truncated IPv6 TCP lines (`[|tcp]`) were dropped, so TCP was under-counted.
This version captures with a 256-byte snap length and `-e`, so the frame length is known even when the transport header
is truncated.

What it measures, per 5 s bin, by address family (4/6) and protocol (tcp/udp):
  tunnel_in   bytes arriving at the server's tunnel listeners (REALITY tcp, Hysteria2 udp) from a client
  google_out  bytes the server sends to Google prefixes; google_in is the reply direction
No address, port or payload is stored, and no pcap is ever written.

What it can conclude: VALID_OVERLAP only when the client's tunnel activity and Google-directed traffic occur in the same bins
inside the test window, outside any canary window. Everything else is NO_OVERLAP_INCONCLUSIVE with a reason. Even a valid
overlap is correlation: an encrypted aggregate cannot prove that given packets carried Shorts media.

  af_capture.py capture --iface eth0 --duration 600 --state S.json --tunnel tcp:443 --tunnel udp:8443 --server ADDR ...
  af_capture.py status --state S.json
  af_capture.py analyze --state S.json --from UNIX --to UNIX [--canary UNIX:UNIX ...]
"""
from __future__ import annotations

import argparse
import ipaddress
import json
import os
import re
import subprocess
import sys
import time

BIN_S = 5
MAX_DURATION_S = 1800
SNAPLEN = 256
MIN_COINCIDENT_BINS = 2
RECENT_S = 15
LIMITS = ("Server-side aggregate correlation only: it does not prove that particular encrypted packets carried Shorts media, "
          "nor which app produced them.")

# Google's major allocations (subset of https://www.gstatic.com/ipranges/goog.json). Use --google-prefixes with the
# current goog.json for a run that matters; the default exists so a missing file never silently counts nothing.
DEFAULT_GOOGLE = (
    "8.8.4.0/24", "8.8.8.0/24", "34.64.0.0/10", "35.184.0.0/13", "64.233.160.0/19", "66.102.0.0/20", "66.249.64.0/19",
    "72.14.192.0/18", "74.125.0.0/16", "108.177.0.0/17", "142.250.0.0/15", "172.217.0.0/16", "172.253.0.0/16",
    "173.194.0.0/16", "192.178.0.0/15", "209.85.128.0/17", "216.58.192.0/19", "216.239.32.0/19",
    "2001:4860::/32", "2404:6800::/32", "2404:f340::/32", "2600:1900::/28", "2607:f8b0::/32", "2620:11a:a000::/40",
    "2800:3f0::/32", "2a00:1450::/32", "2c0f:fb50::/32",
)

_TS = re.compile(r"^(\d+\.\d+) ")
_ETH = re.compile(r"ethertype IPv([46]) \(0x[0-9a-f]+\), length (\d+): (.*)$")
_NOETH = re.compile(r"^IP(6?) (.*)$")
_FLOW = re.compile(r"^(\S+) > (\S+?): (.*)$")
_PAYLEN = re.compile(r"length (\d+)")


def _split(ep: str):
    host, _, port = ep.rpartition(".")
    if not host or not port.isdigit():
        return None
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return None
    return host, int(port)


def parse_line(line: str):
    """-> (ts, family, proto, src, sport, dst, dport, frame_bytes|None) or None for anything unrecognized."""
    if len(line) > 2000:
        return None
    m = _TS.match(line)
    if not m:
        return None
    ts, rest = float(m.group(1)), line[m.end():]
    e = _ETH.search(rest)
    if e:
        fam, nbytes, body = e.group(1), int(e.group(2)), e.group(3)
    else:
        n = _NOETH.match(rest)
        if not n:
            return None
        fam, nbytes, body = ("6" if n.group(1) else "4"), None, n.group(2)
    f = _FLOW.match(body)
    if not f:
        return None
    s, d, payload = _split(f.group(1)), _split(f.group(2)), f.group(3)
    if not s or not d:
        return None
    if payload.startswith("UDP") or payload.startswith("udp"):
        proto = "udp"
    elif payload.startswith("Flags [") or payload.startswith("[|tcp]") or payload.startswith("tcp"):
        proto = "tcp"
    else:
        return None
    if str(ipaddress.ip_address(s[0]).version) != fam:
        return None
    if nbytes is None and not payload.startswith("[|"):
        pl = _PAYLEN.search(payload)
        nbytes = int(pl.group(1)) if pl else None
    return ts, fam, proto, s[0], s[1], d[0], d[1], nbytes


class GooglePrefixes:
    def __init__(self, nets):
        self.nets = [ipaddress.ip_network(n) for n in nets]

    @classmethod
    def default(cls):
        return cls(DEFAULT_GOOGLE)

    @classmethod
    def load(cls, path):
        doc = json.load(open(path))
        return cls([p.get("ipv4Prefix") or p.get("ipv6Prefix") for p in doc.get("prefixes", []) if p.get("ipv4Prefix") or p.get("ipv6Prefix")])

    def contains(self, addr: str) -> bool:
        a = ipaddress.ip_address(addr)
        return any(a.version == n.version and a in n for n in self.nets)


class Config:
    def __init__(self, server_addrs=(), tunnel_ports=(), google: GooglePrefixes | None = None):
        self.server_addrs = set(server_addrs)
        self.tunnel_ports = set(tunnel_ports)          # {("tcp", 443), ("udp", 8443)}
        self.google = google or GooglePrefixes.default()


def _zero():
    return {f: {"tcp": 0, "udp": 0} for f in ("4", "6")}


class Aggregator:
    def __init__(self, cfg: Config):
        self.cfg, self.bins, self.malformed, self.first, self.last = cfg, {}, 0, None, None

    def feed(self, line: str) -> None:
        r = parse_line(line.rstrip("\n"))
        if r is None:
            self.malformed += 1
            return
        ts, fam, proto, src, sport, dst, dport, n = r
        srv = self.cfg.server_addrs
        if dst in srv and src not in srv and (proto, dport) in self.cfg.tunnel_ports:
            kind = "tunnel_in"
        elif src in srv and self.cfg.google.contains(dst):
            kind = "google_out"
        elif dst in srv and self.cfg.google.contains(src):
            kind = "google_in"
        else:
            return
        start = int(ts) - int(ts) % BIN_S
        b = self.bins.setdefault(start, {"tunnel_in": _zero(), "google_out": _zero(), "google_in": _zero(), "bytes_unknown_packets": 0})
        if n is None:
            b["bytes_unknown_packets"] += 1
        else:
            b[kind][fam][proto] += n
        self.first = ts if self.first is None else min(self.first, ts)
        self.last = ts if self.last is None else max(self.last, ts)

    def report(self) -> dict:
        return {"bin_s": BIN_S, "observed": [self.first, self.last], "malformed_lines": self.malformed,
                "bins": {str(k): v for k, v in sorted(self.bins.items())}, "limits": LIMITS}


def _total(d: dict) -> int:
    return sum(v for fam in d.values() for v in fam.values())


def classify_window(report: dict, a: float, b: float, canary_windows=()) -> dict:
    """Is there a valid overlap between the client's tunnel activity and Google traffic inside [a, b)?"""
    def out(verdict, reason, **kw):
        return {"verdict": verdict, "reason": reason, "window": [a, b], "limits": LIMITS, **kw}

    cap = report.get("capture") or {}
    first, last = report.get("observed") or [None, None]
    if cap.get("started") is not None:
        covered = cap["started"] <= a and (cap.get("ended") or cap.get("expires") or 0) >= b
    else:
        covered = first is not None and a < last + BIN_S and b > first
    if not covered:
        return out("NO_OVERLAP_INCONCLUSIVE", "capture_does_not_cover_window")
    canary = lambda s: any(s < c1 and s + BIN_S > c0 for c0, c1 in canary_windows)   # noqa: E731
    tun_fam, goo_fam = {"4": 0, "6": 0}, {"4": 0, "6": 0}
    tunnel = google_any = google_real = coincident = 0
    for k, v in report.get("bins", {}).items():
        s = int(k)
        if not (a <= s < b):
            continue
        t = _total(v["tunnel_in"])
        g = _total(v["google_out"]) + _total(v["google_in"])
        tunnel += t
        google_any += g
        for f in ("4", "6"):
            tun_fam[f] += sum(v["tunnel_in"][f].values())
        if canary(s):
            continue
        google_real += g
        for f in ("4", "6"):
            goo_fam[f] += sum(v["google_out"][f].values()) + sum(v["google_in"][f].values())
        if t and g:
            coincident += 1
    detail = {"coincident_bins": coincident, "tunnel_bytes_by_family": tun_fam, "google_bytes_by_family": goo_fam}
    if not tunnel:
        return out("NO_OVERLAP_INCONCLUSIVE", "no_tunnel_activity", **detail)
    if not google_any:
        return out("NO_OVERLAP_INCONCLUSIVE", "no_google_traffic", **detail)
    if not google_real:
        return out("NO_OVERLAP_INCONCLUSIVE", "google_traffic_only_in_canary_windows", **detail)
    if coincident < MIN_COINCIDENT_BINS:
        return out("NO_OVERLAP_INCONCLUSIVE", "too_few_coincident_bins", **detail)
    return out("VALID_OVERLAP", "tunnel_and_google_coincide", **detail)


def capture_command(iface: str, duration: int, cfg: Config) -> list[str]:
    d = max(1, min(int(duration), MAX_DURATION_S))
    return ["timeout", str(d), "tcpdump", "-n", "-tt", "-e", "-l", "-s", str(SNAPLEN), "-i", iface, "ip or ip6"]


def write_state(path: str, agg: Aggregator, started: float, expires: float, pid: int, ended: float | None = None) -> None:
    doc = {**agg.report(), "capture": {"started": started, "expires": expires, "ended": ended, "pid": pid}}
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f)
    os.replace(tmp, path)


def _alive(pid) -> bool:
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


def status(path: str, now: float | None = None) -> dict:
    now = time.time() if now is None else now
    try:
        doc = json.load(open(path))
    except (OSError, ValueError):
        return {"running": False, "reason": "no capture state"}
    cap = doc.get("capture", {})
    running = cap.get("ended") is None and now < cap.get("expires", 0) and _alive(cap.get("pid"))
    recent = [v for k, v in doc.get("bins", {}).items() if now - RECENT_S <= int(k) <= now]
    return {"running": running, "expires_in_s": max(0, int(cap.get("expires", 0) - now)),
            "tunnel_seen_recently": running and any(_total(v["tunnel_in"]) for v in recent),
            "google_seen_recently": running and any(_total(v["google_out"]) + _total(v["google_in"]) for v in recent),
            "bins": len(doc.get("bins", {})), "malformed_lines": doc.get("malformed_lines", 0)}


def _tunnel(spec: str):
    proto, _, port = spec.partition(":")
    if proto not in ("tcp", "udp") or not port.isdigit():
        raise argparse.ArgumentTypeError("--tunnel PROTO:PORT, e.g. tcp:443")
    return proto, int(port)


def _window(spec: str):
    a, _, b = spec.partition(":")
    return float(a), float(b)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("capture")
    c.add_argument("--iface", required=True)
    c.add_argument("--duration", type=int, required=True, help=f"seconds, capped at {MAX_DURATION_S}")
    c.add_argument("--state", required=True)
    c.add_argument("--server", action="append", required=True, help="this server's own address (repeat per family)")
    c.add_argument("--tunnel", action="append", type=_tunnel, required=True)
    c.add_argument("--google-prefixes", help="goog.json; default: built-in major allocations")
    s = sub.add_parser("status")
    s.add_argument("--state", required=True)
    n = sub.add_parser("analyze")
    n.add_argument("--state", required=True)
    n.add_argument("--from", dest="a", type=float, required=True)
    n.add_argument("--to", dest="b", type=float, required=True)
    n.add_argument("--canary", action="append", type=_window, default=[])
    a = ap.parse_args(argv)
    if a.cmd == "status":
        print(json.dumps(status(a.state)))
        return 0
    if a.cmd == "analyze":
        print(json.dumps(classify_window(json.load(open(a.state)), a.a, a.b, a.canary)))
        return 0
    os.umask(0o077)
    cfg = Config(set(a.server), set(a.tunnel), GooglePrefixes.load(a.google_prefixes) if a.google_prefixes else GooglePrefixes.default())
    agg, started = Aggregator(cfg), time.time()
    cmd = capture_command(a.iface, a.duration, cfg)
    expires = started + int(cmd[1])
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    last_write = 0.0
    try:
        for line in p.stdout:
            agg.feed(line)
            if time.time() - last_write >= BIN_S:
                write_state(a.state, agg, started, expires, os.getpid())
                last_write = time.time()
    finally:
        p.wait()
        write_state(a.state, agg, started, expires, os.getpid(), ended=time.time())
    return 0


if __name__ == "__main__":
    sys.exit(main())
