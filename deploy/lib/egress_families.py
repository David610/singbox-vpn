#!/usr/bin/env python3
"""Read-only aggregate sing-box outbound :443 observations, never addresses.

Direction is inferred using process ownership and configured listener ports.
Rows without process visibility are unknown. Counts are repeated observations,
not unique flows, Google attribution, per-user evidence or leak/playback proof.
"""
import argparse
import collections
import ipaddress
import json
import re
import subprocess
import time


def endpoint(value):
    host, port = value.rsplit(":", 1)
    return ipaddress.ip_address(host.strip("[]").split("%", 1)[0]), int(port)


def count_rows(text, listener_ports):
    counts = collections.Counter()
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 6 or fields[0] not in ("tcp", "udp"):
            counts["unparsed"] += 1
            continue
        try:
            local, local_port = endpoint(fields[4])
            peer, peer_port = endpoint(fields[5])
        except ValueError:
            counts["unparsed"] += 1
            continue
        if not re.search(r'\("sing-box",pid=\d+', line):
            counts["unattributed"] += 1
            continue
        if local_port in listener_ports:
            counts["inbound_or_listener"] += 1
            continue
        peer = peer.ipv4_mapped if isinstance(peer, ipaddress.IPv6Address) and peer.ipv4_mapped else peer
        if peer_port != 443 or peer.is_loopback:
            counts["other"] += 1
            continue
        counts[f"{fields[0]}_ipv{peer.version}"] += 1
    return dict(counts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listen-port", action="append", type=int, default=[])
    parser.add_argument("--samples", type=int, default=6)
    args = parser.parse_args()
    if not 1 <= args.samples <= 10 or any(not 1 <= p <= 65535 for p in args.listen_port):
        parser.error("samples must be 1..10 and listener ports 1..65535")
    ports = set(args.listen_port or [443])
    totals = collections.Counter()
    started = time.monotonic()
    for index in range(args.samples):
        result = subprocess.run(["ss", "-H", "-n", "-t", "-u", "-p"], capture_output=True, text=True, timeout=2)
        if result.returncode:
            raise RuntimeError("ss failed; no family observation established")
        totals.update(count_rows(result.stdout, ports))
        if index + 1 < args.samples:
            time.sleep(max(0, started + (index + 1) * 0.5 - time.monotonic()))
    print(json.dumps({"scope": "aggregate outbound :443 socket observations; not Google, video, user or leak attribution", "seconds": round(time.monotonic() - started, 2), "samples": args.samples, "counts": dict(totals)}))


if __name__ == "__main__":
    main()
