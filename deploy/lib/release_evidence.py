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
REQUIRED_YOUTUBE_MODES = ("signed_in", "incognito")
REQUIRED_SHORTS_CONSECUTIVE = 10
REQUIRED_PLAYBACK_SECONDS = 900
REQUIRED_SHORT_PLAYBACK_SECONDS = 30
PLAYBACK_GATES = ("long_form_playback", "seeking", "reconnect", "app_restart",
                  "post_reconcile_playback", "google_egress_verified")
CELL_FIELDS = ("iphone_model", "ios_version", "hiddify_version", "transport", "network",
               "date", "result", "evidence_ref", "shorts_consecutive_played",
               "known_failing_short_played", "release_tag", "git_commit", "youtube_mode",
               "youtube_version", "binary_sha256")


def measured_at_least(value, minimum: int) -> bool:
    # bool is an int subclass; strings and guessed/coerced measurements are not evidence.
    return type(value) is int and value >= minimum


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
                and measured_at_least(cell.get("shorts_consecutive_played"), REQUIRED_SHORTS_CONSECUTIVE)
                and measured_at_least(cell.get("shorts_distinct_played"), REQUIRED_SHORTS_CONSECUTIVE)
                and cell["shorts_distinct_played"] <= cell["shorts_consecutive_played"]
                and measured_at_least(cell.get("shorts_min_playback_seconds"), REQUIRED_SHORT_PLAYBACK_SECONDS)
                and type(cell.get("observed_failures")) is int and cell["observed_failures"] == 0
                and cell.get("repeated_switching") is True
                and measured_at_least(cell.get("continuous_playback_seconds"), REQUIRED_PLAYBACK_SECONDS)
                and all(cell.get(f) is True for f in PLAYBACK_GATES)
                and DIGEST.fullmatch(str(doc.get("runtime_binary_sha256", "")))
                and cell.get("binary_sha256") == doc["runtime_binary_sha256"]
                and cell.get("known_failing_short_played") is True):
            good[(cell["transport"], cell["network"], cell["youtube_mode"])] = cell
    missing = [f"{t}/{n}/{m}" for t in REQUIRED_TRANSPORTS for n in REQUIRED_NETWORKS
               for m in REQUIRED_YOUTUBE_MODES if (t, n, m) not in good]
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

