#!/usr/bin/env python3
"""Machine-readable release evidence validator.

    release_evidence.py FILE [--require LEVEL] [--tag TAG] [--commit SHA]

Computes the highest readiness level the evidence *supports* (never the level
it claims) and exits 1 if the file is malformed, bound to a different tag or
commit, or supports less than --require. Levels, each including the last:

  CODE_QUALIFIED    CI green on exactly this commit
  SERVER_QUALIFIED  + VPS lifecycle acceptance PASS on exactly this commit
  DEVICE_VERIFIED   + every required native-device matrix cell PASS on this
                    tag/commit, with an evidence reference
  PRODUCTION_READY  + deployed config fingerprint equals the approved contract,
                    security gates PASS, and NO waiver of any kind

A waiver is a documented limited-risk release decision; it can never raise a
level. Any waived or non-PASS required device cell caps the level below
DEVICE_VERIFIED. Stdlib only. Never reads or prints secrets.
"""
from __future__ import annotations

import argparse
import json
import re
import sys

LEVELS = ["NONE", "CODE_QUALIFIED", "SERVER_QUALIFIED", "DEVICE_VERIFIED", "PRODUCTION_READY"]
SHA = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
# Native-device cells that must each be PASS (INV-12 / DEVICE_ACCEPTANCE_TESTS.md).
REQUIRED_TRANSPORTS = ("reality", "hysteria2")
REQUIRED_NETWORKS = ("wifi", "cellular")
REQUIRED_SHORTS_CONSECUTIVE = 10
CELL_FIELDS = ("iphone_model", "ios_version", "hiddify_version", "transport", "network",
               "date", "result", "evidence_ref", "shorts_consecutive_played",
               "known_failing_short_played", "release_tag", "git_commit")


def compute(doc: dict, tag: str | None, commit: str | None) -> tuple[str, list[str]]:
    errs: list[str] = []

    def need(cond: bool, msg: str) -> bool:
        if not cond:
            errs.append(msg)
        return cond

    if not need(doc.get("schema_version") == 1, "schema_version must be 1"):
        return "NONE", errs
    need(isinstance(doc.get("release_tag"), str) and doc["release_tag"], "release_tag missing")
    need(bool(SHA.match(str(doc.get("git_commit", "")))), "git_commit must be a 40-hex SHA")
    if tag:
        need(doc.get("release_tag") == tag, f"evidence is for {doc.get('release_tag')!r}, not {tag!r}")
    if commit:
        need(doc.get("git_commit") == commit, "evidence git_commit differs from the commit under release")
    digs = doc.get("artifact_digests")
    need(isinstance(digs, dict) and digs and all(DIGEST.match(str(v)) for v in digs.values()),
         "artifact_digests must map artifact -> sha256:<64 hex>")
    if errs:
        return "NONE", errs
    c = doc["git_commit"]
    level = "NONE"

    ci = doc.get("ci") or {}
    if ci.get("conclusion") == "success" and ci.get("git_commit") == c and ci.get("run_url"):
        level = "CODE_QUALIFIED"
    else:
        errs.append("CODE_QUALIFIED not met: ci.conclusion=success, ci.git_commit=this commit and ci.run_url required")
        return level, errs

    vps = doc.get("vps_acceptance") or {}
    if (vps.get("result") == "PASS" and vps.get("git_commit") == c and vps.get("environment")
            and vps.get("evidence_ref") and not vps.get("not_run")):
        level = "SERVER_QUALIFIED"
    else:
        errs.append("SERVER_QUALIFIED not met: vps_acceptance PASS on this commit with environment+evidence_ref required")
        return level, errs

    cells = doc.get("device_matrix") or []
    waivers = doc.get("waivers") or []
    good = {}
    for cell in cells:
        if (cell.get("result") == "PASS" and cell.get("git_commit") == c
                and cell.get("release_tag") == doc["release_tag"]
                and all(cell.get(f) not in (None, "") for f in CELL_FIELDS)
                and cell.get("shorts_consecutive_played", 0) >= REQUIRED_SHORTS_CONSECUTIVE
                and cell.get("known_failing_short_played") is True):
            good[(cell["transport"], cell["network"])] = cell
    missing = [f"{t}/{n}" for t in REQUIRED_TRANSPORTS for n in REQUIRED_NETWORKS if (t, n) not in good]
    if missing:
        errs.append("DEVICE_VERIFIED not met: no complete PASS cell for " + ", ".join(missing)
                    + (" (waived)" if waivers else ""))
        return level, errs
    level = "DEVICE_VERIFIED"

    dep = doc.get("deployment") or {}
    sec = doc.get("security_gates") or {}
    prod_ok = (bool(dep.get("config_fingerprint")) and dep.get("config_fingerprint") == dep.get("approved_fingerprint")
               and dep.get("environment") and sec.get("result") == "PASS" and not waivers)
    if prod_ok:
        level = "PRODUCTION_READY"
    else:
        errs.append("PRODUCTION_READY not met: deployed fingerprint must equal the approved one, security gates PASS, no waivers")
    return level, errs


def validate_waivers(doc: dict) -> list[str]:
    out = []
    for i, w in enumerate(doc.get("waivers") or []):
        for f in ("scope", "reason", "approver", "date", "risk"):
            if not w.get(f):
                out.append(f"waivers[{i}].{f} missing")
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--require", choices=LEVELS[1:])
    ap.add_argument("--tag")
    ap.add_argument("--commit")
    a = ap.parse_args()
    try:
        doc = json.load(open(a.file))
    except (OSError, ValueError) as e:
        print(f"ERROR: cannot read evidence: {e}")
        return 1
    level, errs = compute(doc, a.tag, a.commit)
    wav = validate_waivers(doc)
    claimed = doc.get("claimed_level")
    print(f"supported_level: {level}")
    if doc.get("waivers"):
        print(f"waivers: {len(doc['waivers'])} (limited-risk decision; does not raise the level)")
    bad = list(wav)
    if claimed and LEVELS.index(claimed) > LEVELS.index(level):
        bad.append(f"claimed_level {claimed} exceeds what the evidence supports ({level})")
    if a.require and LEVELS.index(level) < LEVELS.index(a.require):
        bad.append(f"required {a.require} but evidence supports only {level}")
    if level == "NONE":
        bad += errs
    for m in errs if level != "NONE" else []:
        print("note:", m)
    for m in bad:
        print("ERROR:", m)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
