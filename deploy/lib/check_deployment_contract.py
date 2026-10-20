#!/usr/bin/env python3
"""Compare a node against its version-controlled, non-secret contract.

    check_deployment_contract.py [--contract FILE] [--strict] [--json OUT]

Read-only. Reports PASS / FAIL (observed differs) / UNDECIDED (the contract
value is not decided yet) / UNOBSERVABLE (could not be read). It NEVER changes
the node. Exit 1 on any FAIL; with --strict also on UNDECIDED/UNOBSERVABLE, so
a release gate can demand a fully decided contract. Secrets are never read.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tomllib

UNDECIDED = "UNDECIDED"
# Contract keys that state a requirement; they cannot be observed on a host.
POLICY_ONLY = {"youtube.required"}


def routing_fingerprint(singbox_cfg: dict) -> str:
    """sha256 over route.final and the NON-user-specific route rules, so adding or
    removing a user cannot change it but losing/reordering a policy rule does."""
    route = singbox_cfg.get("route", {})
    rules = [r for r in route.get("rules", []) if "auth_user" not in r]
    blob = json.dumps({"final": route.get("final"), "rules": rules}, sort_keys=False, separators=(",", ":"))
    return hashlib.sha256(blob.encode()).hexdigest()


def egress_mode(dep: dict) -> str:
    if dep.get("google_egress_hairpin"):
        return "hairpin"
    pol = (dep.get("egress") or {}).get("ipv6_literal", "allow")
    return {"allow": "plain", "reject_google": "reject_google", "reject_all": "reject_all"}.get(pol, f"unknown:{pol}")


def observe(dep: dict | None, sb: dict | None, state: dict | None, sshd: dict | None, aaaa: bool | None) -> dict:
    o: dict = {}
    if dep is not None:
        o["target.node_id"] = dep.get("node_id")
        o["target.role"] = dep.get("role")
        o["target.public_ipv4"] = dep.get("public_ipv4")
        o["target.public_ipv6"] = dep.get("public_ipv6")
        o["target.public_host"] = dep.get("public_host")
        o["target.subscription_host"] = dep.get("subscription_host")
        o["subscription.expected_public_port"] = (dep.get("subscription") or {}).get("public_port")
        o["youtube.egress_mode"] = egress_mode(dep)
        h = dep.get("google_egress_hairpin") or {}
        o["youtube.approved_hairpin_target"] = h.get("relay_host") if h else "none"
    if sb is not None:
        types = {i.get("type") for i in sb.get("inbounds", [])}
        o["protocols.reality"] = "vless" in types
        o["protocols.hysteria2"] = "hysteria2" in types
        o["routing.fingerprint"] = routing_fingerprint(sb)
        o["security.singbox_log_level"] = (sb.get("log") or {}).get("level")
    if state is not None:
        o["release.approved_tag"] = state.get("singbox_vpn_version")
        o["target.operating_system"] = state.get("os_family")
    if sshd is not None:
        pa = sshd.get("passwordauthentication")
        o["security.ssh_password_authentication"] = None if pa is None else pa == "yes"
        o["security.ssh_permit_root_login"] = sshd.get("permitrootlogin")
    if aaaa is not None:
        o["dns.aaaa_published"] = aaaa
    return o


def flatten(d: dict, prefix: str = "") -> dict:
    out = {}
    for k, v in d.items():
        if k.startswith("_"):
            continue
        key = f"{prefix}{k}"
        if isinstance(v, dict):
            out.update(flatten(v, key + "."))
        else:
            out[key] = v
    return out


def compare(contract: dict, observed: dict) -> list[tuple[str, str, str]]:
    rows = []
    for key, want in flatten(contract).items():
        if key == "schema_version" or key in POLICY_ONLY:
            continue
        if want == UNDECIDED:
            rows.append(("UNDECIDED", key, f"observed {observed.get(key)!r}" if key in observed else "no observation"))
        elif key not in observed or observed[key] is None:
            rows.append(("UNOBSERVABLE", key, "could not be read on this host"))
        elif observed[key] == want:
            rows.append(("PASS", key, ""))
        else:
            rows.append(("FAIL", key, f"contract {want!r} but observed {observed[key]!r}"))
    return rows


def _load_toml(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, ValueError):
        return None


def _load_json(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def _sshd():
    try:
        out = subprocess.run(["sshd", "-T"], capture_output=True, text=True, timeout=10).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    d = {}
    for line in out.splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2:
            d.setdefault(parts[0], parts[1].strip())
    return d or None


def has_real_aaaa(getent_ahostsv6_output: str) -> bool:
    """getent ahostsv6 also lists IPv4-mapped (::ffff:a.b.c.d) answers for a
    host that only has an A record; those are not an AAAA record."""
    for line in getent_ahostsv6_output.splitlines():
        parts = line.split()
        if parts and ":" in parts[0] and not parts[0].lower().startswith("::ffff:"):
            return True
    return False


def _aaaa(host):
    try:
        out = subprocess.run(["getent", "ahostsv6", host], capture_output=True, text=True, timeout=10).stdout
        return has_real_aaaa(out)
    except (OSError, subprocess.SubprocessError):
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--contract", default="deploy/contracts/fin1-staging.contract.json")
    ap.add_argument("--deployment-toml", default="/etc/vpn/deployment.toml")
    ap.add_argument("--singbox-config", default="/etc/vpn/compat/sing-box/config.json")
    ap.add_argument("--install-state", default="/var/lib/singbox-vpn/install-state.json")
    ap.add_argument("--strict", action="store_true")
    ap.add_argument("--json")
    a = ap.parse_args()
    contract = _load_json(a.contract)
    if contract is None:
        print(f"ERROR: cannot read contract {a.contract}")
        return 1
    dep = _load_toml(a.deployment_toml)
    host = (contract.get("target") or {}).get("public_host")
    obs = observe(dep, _load_json(a.singbox_config), _load_json(a.install_state), _sshd(), _aaaa(host) if host else None)
    rows = compare(contract, obs)
    for st, key, msg in rows:
        print(f"{st:<12} {key}" + (f"  — {msg}" if msg else ""))
    counts = {s: sum(1 for r in rows if r[0] == s) for s in ("PASS", "FAIL", "UNDECIDED", "UNOBSERVABLE")}
    print("SUMMARY", json.dumps(counts))
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"contract": a.contract, "summary": counts, "rows": rows}, f, indent=1)
    bad = counts["FAIL"] > 0 or (a.strict and (counts["UNDECIDED"] or counts["UNOBSERVABLE"]))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
