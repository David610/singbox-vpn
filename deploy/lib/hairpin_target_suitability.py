#!/usr/bin/env python3
"""Read-only suitability qualification of a candidate Google-egress hairpin target (docs/operations/HAIRPIN_TARGET_QUALIFICATION.md).

NOTHING here runs against any host unless the owner has authorized that host and you pass --authorized-by-owner.
Without it, `plan` prints exactly what would be executed and `inspect`/`probe` refuse.

  hairpin_target_suitability.py plan    --target root@HOST
  hairpin_target_suitability.py inspect --target root@HOST --authorized-by-owner --out inspect.json     (step A1: read-only SSH)
  hairpin_target_suitability.py probe   --target root@HOST --authorized-by-owner --out run1.json        (step A2: SOCKS tunnel, no remote command)
  hairpin_target_suitability.py aggregate run1.json run2.json run3.json                                  (offline verdict; needs 3 runs >= 1 h apart)

Safety properties (each enforced by tests/test_hairpin_target_suitability.py):
  * every remote command is a constant from READ_ONLY_COMMANDS, checked against a deny-list of mutating verbs and shell
    redirection; the tunnel step runs `ssh -N` (no remote command at all);
  * ssh runs with BatchMode, strict host-key checking, no agent/X11/extra forwarding, bounded timeouts;
  * the report holds addresses, versions and aggregate counts only; UUIDs and long key-like strings are redacted;
  * the verdict is computed, never typed: UNSUITABLE / INCONCLUSIVE / RUN_PASS. A single run is never "SUITABLE": acceptance
    C1 needs three runs >= 1 h apart (aggregate), and C4-C8 (rendered policy, fail-closed, real tunnels, restart preservation,
    native device) are reported as NOT_COVERED here and stay UNVERIFIED until their own evidence exists.
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import time

# (id, why, command). Pure reads. Edit only together with the deny-list test.
READ_ONLY_COMMANDS = [
    ("identity", "who/what/when", "hostnamectl --static; . /etc/os-release; echo \"$PRETTY_NAME\"; uname -r; id -un; date -u +%FT%TZ"),
    ("addresses", "source addresses and families", "ip -br addr; echo ---; ip -4 route; echo ---; ip -6 route"),
    ("release", "installed singbox-vpn release",
     "vpn-admin --version 2>&1; python3 -c 'import json;d=json.load(open(\"/var/lib/singbox-vpn/install-state.json\"));"
     "print({k:d.get(k) for k in (\"singbox_vpn_version\",\"role\",\"node_id\",\"acceptance\",\"os_family\")})' 2>&1"),
    ("restart_hazards", "INV-10: provisioning agent / restart loops",
     "systemctl is-enabled vpn-provisioning-agent 2>&1; systemctl is-active vpn-provisioning-agent 2>&1; "
     "systemctl show sing-box -p NRestarts -p ActiveEnterTimestamp -p MainPID"),
    ("health", "doctor without the protocol probe", "vpn-admin doctor 2>&1 | grep -E '^\\[(FAIL|WARN)\\]|FAILS=' | head -40"),
    ("capacity", "headroom", "uptime; free -m; df -h /; ss -s; nproc"),
    ("exposure", "firewall and listeners",
     "ufw status verbose 2>&1 | head -30; ss -tulnp | awk '{print $1,$5}' | sort | head -40"),
    ("ssh_posture", "effective sshd settings",
     "sshd -T 2>/dev/null | grep -E '^(permitrootlogin|passwordauthentication|pubkeyauthentication|port) '"),
    ("egress_v4", "IPv4 egress address", "curl -4 -s -m 10 https://ifconfig.me; echo"),
    ("egress_v6", "IPv6 egress address (or none)", "curl -6 -s -m 10 https://ifconfig.me || echo none"),
    ("logging", "privacy: sing-box log level",
     "python3 -c 'import json;print(json.load(open(\"/etc/vpn/compat/sing-box/config.json\")).get(\"log\"))' 2>&1"),
]

# A command containing any of these tokens is mutating or exfiltrating and must never be added to READ_ONLY_COMMANDS.
DENY = re.compile(r"(?<![\w-])(rm|mv|cp|dd|chmod|chown|kill|pkill|reboot|shutdown|apt|apt-get|dnf|yum|pip|tee|sed\s+-i|"
                  r"truncate|mkfs|mount|umount|useradd|userdel|passwd|crontab|iptables|nft\s+(add|delete|flush)|"
                  r"systemctl\s+(start|stop|restart|reload|enable|disable|mask|daemon-reload|reset-failed)|vpn-admin\s+(user|google-egress-hairpin|render)|"
                  r"curl\s+[^|;]*(-X\s*(POST|PUT|DELETE)|-d\b|--data|-T\b|-F\b))(?![\w-])|>>?\s*(?!/dev/null\b|&)\S|\|\s*(sh|bash)\b")

SSH_BASE = ["-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=10",
            "-o", "ServerAliveCountMax=3", "-o", "ForwardAgent=no", "-o", "ForwardX11=no", "-o", "ClearAllForwardings=yes"]
SECRETISH = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b|\b[A-Za-z0-9_-]{40,}\b")
TARGET_RE = re.compile(r"^[A-Za-z0-9_.-]+@[A-Za-z0-9_.:\[\]-]+$")


def redact(text: str) -> str:
    return SECRETISH.sub("[redacted]", text)


def commands_are_read_only() -> list[str]:
    return [cid for cid, _, cmd in READ_ONLY_COMMANDS if DENY.search(cmd)]


def check_target(t: str) -> str:
    if not TARGET_RE.match(t or ""):
        raise SystemExit(f"refusing: --target must look like user@host (got {t!r})")
    return t


def require_authorization(a) -> None:
    if not a.authorized_by_owner:
        raise SystemExit("refusing: this touches a host other than the staging node. The owner must authorize THIS host "
                         "(docs/operations/HAIRPIN_TARGET_QUALIFICATION.md section A), then pass --authorized-by-owner.")


# ------------------------------------------------------------------------------------------------------------ pure verdicts

def verdict_inspect(r: dict) -> tuple[str, list[str]]:
    """UNSUITABLE if a hard requirement is visibly unmet; INCONCLUSIVE if evidence is missing; otherwise INSPECT_OK."""
    bad, missing = [], []
    out = r.get("commands", {})
    for k in ("addresses", "release", "restart_hazards", "health", "egress_v4"):
        if k not in out or out[k].get("rc") is None:
            missing.append(k)
    if missing:
        return "INCONCLUSIVE", [f"no evidence for: {', '.join(missing)}"]
    haz = out["restart_hazards"].get("stdout", "")
    if re.search(r"^active$", haz, re.M) and re.search(r"^enabled$", haz, re.M):
        bad.append("provisioning agent enabled AND active (INV-10): a restart loop of sing-box is possible")
    m = re.search(r"NRestarts=(\d+)", haz)
    if m and int(m.group(1)) > 5:
        bad.append(f"sing-box NRestarts={m.group(1)} (unstable)")
    if "[FAIL]" in out["health"].get("stdout", ""):
        bad.append("doctor reports FAIL")
    v4 = out["egress_v4"].get("stdout", "").strip()
    if not re.match(r"^\d{1,3}(\.\d{1,3}){3}$", v4):
        bad.append("no IPv4 egress address could be determined")
    ssh = out.get("ssh_posture", {}).get("stdout", "")
    warn = []
    if re.search(r"passwordauthentication yes", ssh):
        warn.append("sshd allows password authentication (security finding, not a hairpin blocker)")
    return ("UNSUITABLE" if bad else "INSPECT_OK"), bad + warn


def verdict_probe(target: dict, control: dict, min_shorts: int = 10) -> tuple[str, list[str]]:
    """target/control: the probe's summary + counted genuine shorts and plays. The control (the refused staging path, same run)
    proves the probe CAN fail; without it a target pass is not trusted (C2)."""
    why = []
    if target["genuine"] < min_shorts:
        return "INCONCLUSIVE", [f"only {target['genuine']} genuine Shorts loaded through the target (need {min_shorts})"]
    if control["genuine"] < 3:
        return "INCONCLUSIVE", ["the same-run control loaded too few genuine Shorts: the probe's ability to fail is unproven (C2)"]
    if control["played"] == control["genuine"]:
        return "INCONCLUSIVE", ["the control path ALSO played everything: this run cannot distinguish a good target from a good probe"]
    if target["played"] < target["genuine"]:
        return "UNSUITABLE", [f"{target['played']}/{target['genuine']} genuine Shorts played through the target"]
    if target.get("video_played", 0) < target.get("video_total", 0):
        why.append("normal videos failed through the target (reported separately; Shorts passed)")
    return "RUN_PASS", why


def aggregate(reports: list[dict], min_gap_s: int = 3600) -> tuple[str, list[str]]:
    probes = [r for r in reports if r.get("kind") == "probe"]
    if len(probes) < 3:
        return "INCONCLUSIVE", [f"{len(probes)} probe run(s); acceptance C1 needs 3"]
    probes.sort(key=lambda r: r["epoch"])
    if any(b["epoch"] - a["epoch"] < min_gap_s for a, b in zip(probes, probes[1:])):
        return "INCONCLUSIVE", ["runs are less than 1 h apart (C1)"]
    if any(r["verdict"] != "RUN_PASS" for r in probes):
        return "UNSUITABLE", [f"run {i + 1}: {r['verdict']} {r.get('reasons')}" for i, r in enumerate(probes) if r["verdict"] != "RUN_PASS"]
    return "C1_C2_SATISFIED", ["C3-C8 are NOT covered by this tool: stability review, rendered policy, fail-closed, real-tunnel Shorts, "
                              "restart preservation and native device acceptance remain UNVERIFIED"]


# ------------------------------------------------------------------------------------------------------------ effects

def run_remote(target: str, cmd: str, ssh_bin: str = "ssh", timeout: int = 60) -> dict:
    t0 = time.time()
    try:
        p = subprocess.run([ssh_bin, *SSH_BASE, target, cmd], capture_output=True, text=True, timeout=timeout)
        return {"rc": p.returncode, "stdout": redact(p.stdout)[-6000:], "stderr": redact(p.stderr)[-1000:], "seconds": round(time.time() - t0, 2)}
    except subprocess.TimeoutExpired:
        return {"rc": None, "stdout": "", "stderr": "timeout", "seconds": round(time.time() - t0, 2)}


def latency(target: str) -> dict:
    host = target.split("@", 1)[1].strip("[]")
    out = {}
    for name, cmd in (("ping", ["ping", "-c", "5", "-W", "2", host]),):
        try:
            p = subprocess.run(cmd, capture_output=True, text=True, timeout=20)
            m = re.search(r"= ([\d.]+)/([\d.]+)/([\d.]+)", p.stdout)
            out[name] = {"min_avg_max_ms": list(map(float, m.groups())) if m else None, "loss_line": (re.search(r"[\d.]+% packet loss", p.stdout) or [None])[0]}
        except Exception as e:  # noqa: BLE001
            out[name] = {"error": type(e).__name__}
    return out


def now() -> tuple[str, float]:
    t = time.time()
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).isoformat(timespec="seconds"), t


def cmd_plan(a) -> int:
    check_target(a.target)
    print(f"# PLAN (nothing is executed). Target: {a.target}")
    print("# A1 inspect - read-only SSH, one command each:")
    for cid, why, cmd in READ_ONLY_COMMANDS:
        print(f"  ssh {' '.join(SSH_BASE)} {a.target} {cmd!r}   # {cid}: {why}")
    print("# A2 probe - SOCKS tunnel with NO remote command, then the YouTube probe through it and, in the same run, through this host (control):")
    print(f"  ssh {' '.join(SSH_BASE[:-2])} -N -D 127.0.0.1:{a.socks_port} {a.target}")
    print(f"  youtube_evidence_probe.py --proxy socks5://127.0.0.1:{a.socks_port} --discover 12 --label target   (+ --label control, direct)")
    print("# Not performed by this tool: A3 (create user on the target; restarts sing-box there) and A4 (set the hairpin on the exit).")
    return 0


def cmd_inspect(a) -> int:
    check_target(a.target); require_authorization(a)
    bad = commands_are_read_only()
    if bad:
        raise SystemExit(f"internal error: command(s) {bad} match the mutating-verb deny-list; refusing to run anything")
    iso, epoch = now()
    rep = {"kind": "inspect", "target": a.target, "utc": iso, "epoch": epoch, "commands": {}}
    for cid, _, cmd in READ_ONLY_COMMANDS:
        rep["commands"][cid] = run_remote(a.target, cmd, a.ssh_bin)
    rep["latency"] = None if a.no_latency else latency(a.target)
    rep["verdict"], rep["reasons"] = verdict_inspect(rep)
    rep["not_covered"] = ["C1-C2 (run `probe`)", "C3 stability over time", "C4-C8"]
    return emit(rep, a.out, 0 if rep["verdict"] == "INSPECT_OK" else 1)


def cmd_probe(a) -> int:
    check_target(a.target); require_authorization(a)
    iso, epoch = now()
    tunnel = subprocess.Popen([a.ssh_bin, *SSH_BASE[:-2], "-N", "-D", f"127.0.0.1:{a.socks_port}", a.target], stdin=subprocess.DEVNULL,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(4)
        if tunnel.poll() is not None:
            return emit({"kind": "probe", "target": a.target, "utc": iso, "epoch": epoch, "verdict": "INCONCLUSIVE",
                         "reasons": ["the SSH SOCKS tunnel did not stay up"]}, a.out, 1)
        sock = f"socks5://127.0.0.1:{a.socks_port}"
        res = {}
        for label, proxy in (("target", sock), ("control", None)):
            out = f"{a.out or 'probe'}.{label}.json"
            cmd = [a.python, a.probe_script, "--label", label, "--discover", str(a.shorts), "--out", out]
            if proxy:
                cmd += ["--proxy", proxy]
            subprocess.run(cmd, capture_output=True, text=True, timeout=1500)
            try:
                doc = json.load(open(out))
            except (OSError, ValueError):
                res[label] = {"genuine": 0, "played": 0}
                continue
            rows = [r for r in doc.get("results", []) if r.get("counts_as_short")]
            vids = [r for r in doc.get("results", []) if r.get("kind") == "video"]
            res[label] = {"genuine": len(rows), "played": sum(1 for r in rows if r.get("play")),
                          "video_total": len(vids), "video_played": sum(1 for v in vids if v.get("play"))}
        # source addresses as YouTube sees them (a plain what-is-my-ip through each path)
        addrs = {}
        for label, proxy in (("target", sock), ("control", None)):
            for fam in ("4", "6"):
                cmd = ["curl", f"-{fam}", "-s", "-m", "10", *(["--proxy", proxy.replace("socks5://", "socks5h://")] if proxy else []), "https://ifconfig.me"]
                p = subprocess.run(cmd, capture_output=True, text=True)
                addrs[f"{label}_v{fam}"] = p.stdout.strip() if p.returncode == 0 else None
        verdict, reasons = verdict_probe(res["target"], res["control"], a.shorts)
        rep = {"kind": "probe", "target": a.target, "utc": iso, "epoch": epoch, "summary": res, "source_addresses": addrs,
               "verdict": verdict, "reasons": reasons, "evidence_class": "browser-emulation-tcp-only",
               "not_covered": ["literal-IP forms through real tunnels (C6)", "QUIC/UDP", "C3-C8"]}
        return emit(rep, a.out, 0 if verdict == "RUN_PASS" else 1)
    finally:
        tunnel.terminate()


def cmd_aggregate(a) -> int:
    reps = [json.load(open(f)) for f in a.reports]
    verdict, reasons = aggregate(reps)
    return emit({"kind": "aggregate", "runs": len(reps), "verdict": verdict, "reasons": reasons}, a.out, 0 if verdict == "C1_C2_SATISFIED" else 1)


def emit(rep: dict, out: str | None, rc: int) -> int:
    s = json.dumps(rep, indent=1)
    if out:
        with open(out, "w") as f:
            f.write(s + "\n")
    print(json.dumps({k: rep.get(k) for k in ("kind", "verdict", "reasons")}))
    return rc


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("plan", "inspect", "probe"):
        p = sub.add_parser(name)
        p.add_argument("--target", required=True)
        p.add_argument("--authorized-by-owner", action="store_true")
        p.add_argument("--out")
        p.add_argument("--ssh-bin", default="ssh")
        p.add_argument("--no-latency", action="store_true", help="skip the ICMP latency measurement (tests)")
        p.add_argument("--socks-port", type=int, default=1090)
        p.add_argument("--shorts", type=int, default=12)
        p.add_argument("--python", default=os.environ.get("PROBE_PYTHON", sys.executable))
        p.add_argument("--probe-script", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "youtube_evidence_probe.py"))
    p = sub.add_parser("aggregate"); p.add_argument("reports", nargs="+"); p.add_argument("--out")
    a = ap.parse_args(argv)
    return {"plan": cmd_plan, "inspect": cmd_inspect, "probe": cmd_probe, "aggregate": cmd_aggregate}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
