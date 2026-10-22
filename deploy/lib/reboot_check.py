#!/usr/bin/env python3
"""Reboot acceptance: capture the node's state before, capture it again after, and
compare. Read-only; secrets are only hashed.

    reboot_check.py capture OUT.json        # run before AND after the reboot
    reboot_check.py compare PRE.json POST.json

PASS requires: a different boot_id (it really rebooted), every service that was
active is active again, the same credentials / config / routing fingerprint /
firewall rules / egress-isolation table / listeners / effective SSH settings, and
working IPv6 neighbour discovery if the host has global IPv6 (I-54 regression).
"""
import hashlib
import json
import os
import re
import subprocess
import sys

UNITS = ["sing-box", "vpn-subscription", "nginx", "ssh", "ufw", "vpn-egress-isolation", "chrony"]
TIMERS = ["vpn-expiry-reconcile.timer", "vpn-service-watchdog.timer", "certbot.timer"]
FILES = ["/etc/vpn/deployment.toml", "/etc/vpn/compat/users/users.json", "/etc/vpn/compat/reality/private.key",
         "/etc/vpn/compat/reality/short_id.txt", "/etc/vpn/compat/hysteria/cert.pem", "/etc/vpn/compat/sing-box/config.json",
         "/etc/nginx/conf.d/vpn-subscription.conf", "/root/.ssh/authorized_keys", "/etc/sysctl.d/99-singbox-vpn-dataplane.conf"]
STABLE = ["files", "units_active", "timers_enabled", "listeners", "ufw", "nft_isolation", "sshd", "sysctl", "routing_fingerprint", "ipv6"]


def sh(cmd):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        return ""


def h(path):
    try:
        return hashlib.sha256(open(path, "rb").read()).hexdigest()[:16]
    except OSError:
        return None


def routing_fingerprint():
    try:
        sb = json.load(open("/etc/vpn/compat/sing-box/config.json"))
    except (OSError, ValueError):
        return None
    rules = [r for r in sb.get("route", {}).get("rules", []) if "auth_user" not in r]
    return hashlib.sha256(json.dumps({"final": sb.get("route", {}).get("final"), "rules": rules}, separators=(",", ":")).encode()).hexdigest()[:16]


def capture():
    d = {"boot_id": open("/proc/sys/kernel/random/boot_id").read().strip(), "uptime_s": float(open("/proc/uptime").read().split()[0])}
    d["files"] = {f: h(f) for f in FILES}
    d["units_active"] = {u: sh(f"systemctl is-active {u}").strip() for u in UNITS}
    d["timers_enabled"] = {t: sh(f"systemctl is-enabled {t}").strip() for t in TIMERS}
    ls = []
    for line in sh("ss -H -tulnp").splitlines():
        p = line.split()
        if len(p) >= 5:
            proc = re.search(r'users:\(\("([^"]+)"', line)
            addr = p[4]
            port = addr.rsplit(":", 1)[-1]
            if int(port) < 32768 or (proc and proc.group(1) in ("sing-box", "nginx", "sshd")):   # ignore ephemeral flow sockets
                ls.append(f"{p[0]} {addr} {proc.group(1) if proc else '-'}")
    d["listeners"] = sorted(set(ls))
    d["ufw"] = sorted(l.strip() for l in sh("ufw status").splitlines() if re.search(r"ALLOW|LIMIT|DENY|Status", l))
    t = sh("nft list table inet arcana_egress_isolation")
    d["nft_isolation"] = {"present": bool(t), "uid_scoped": ("skuid !=" not in t) and ("meta skuid" in t) if t else None, "sha": hashlib.sha256(re.sub(r"counter packets \d+ bytes \d+", "counter", t).encode()).hexdigest()[:16] if t else None}
    sshd = {}
    for line in sh("sshd -T").splitlines():
        k = line.split(None, 1)
        if len(k) == 2 and k[0] in ("passwordauthentication", "permitrootlogin", "pubkeyauthentication", "port", "maxauthtries"):
            sshd.setdefault(k[0], k[1])
    d["sshd"] = sshd
    d["sysctl"] = {k: sh(f"sysctl -n {k}").strip() for k in ("net.ipv4.tcp_congestion_control", "net.core.default_qdisc", "net.core.rmem_max", "net.core.wmem_max")}
    d["routing_fingerprint"] = routing_fingerprint()
    v6 = sh("ip -6 -o addr show scope global")
    gw = re.search(r"default via (\S+) dev (\S+)", sh("ip -6 route show default"))
    state = None
    if gw:
        # force a fresh neighbour exchange so a boot-time ND failure is visible
        sh(f"ip -6 neigh flush dev {gw.group(2)}")
        sh(f"ping -6 -c1 -W2 {gw.group(1)}%{gw.group(2)}")
        n = sh(f"ip -6 neigh show {gw.group(1)} dev {gw.group(2)}").split()
        state = n[-1] if n else None
    d["ipv6"] = {"has_global": bool(v6.strip()), "default_route": bool(gw), "gateway_neighbour": state}
    return d


def compare(pre, post):
    rows = []
    def add(name, ok, detail=""):
        rows.append(("PASS" if ok else "FAIL", name, detail))
    add("the host really rebooted (boot_id changed)", pre["boot_id"] != post["boot_id"], "")
    add("uptime after reboot is smaller than before", post["uptime_s"] < pre["uptime_s"], f"{pre['uptime_s']:.0f}s -> {post['uptime_s']:.0f}s")
    for u, st in pre["units_active"].items():
        if st == "active":
            add(f"service {u} active again", post["units_active"].get(u) == "active", post["units_active"].get(u, "?"))
    for k in STABLE:
        if k in ("units_active",):
            continue
        add(f"{k} unchanged across the reboot", pre.get(k) == post.get(k), "" if pre.get(k) == post.get(k) else f"before={json.dumps(pre.get(k))[:120]} after={json.dumps(post.get(k))[:120]}")
    v6 = post["ipv6"]
    if v6["has_global"] and v6["default_route"]:
        add("IPv6 gateway neighbour discovery works after boot (I-54 guard)", v6["gateway_neighbour"] not in (None, "FAILED"), str(v6["gateway_neighbour"]))
    iso = post["nft_isolation"]
    if iso["present"]:
        add("egress-isolation table is uid-scoped (no negated uid match)", bool(iso["uid_scoped"]), "")
    return rows


def main():
    if len(sys.argv) >= 3 and sys.argv[1] == "capture":
        json.dump(capture(), open(sys.argv[2], "w"), indent=1)
        print(f"captured -> {sys.argv[2]}")
        return 0
    if len(sys.argv) == 4 and sys.argv[1] == "compare":
        rows = compare(json.load(open(sys.argv[2])), json.load(open(sys.argv[3])))
        for r in rows:
            print(f"{r[0]}  {r[1]}" + (f"  ({r[2]})" if r[2] else ""))
        bad = [r for r in rows if r[0] == "FAIL"]
        print(f"RESULT: {len(rows) - len(bad)} passed, {len(bad)} failed")
        return 1 if bad else 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
