#!/usr/bin/env python3
"""Shorts health monitor for one egress path (opt-in; not installed by default).

Runs deploy/lib/youtube_evidence_probe.py, which counts only genuine Shorts
pages whose media was delivered AND whose clock advanced, appends ONE aggregate
JSON line (no tokens, no user traffic; only public video ids) and exits:

  0 OK          >= MIN_SHORTS genuine Shorts, all played
  1 DEGRADED    WARN_AFTER consecutive non-OK runs
  2 CRITICAL    CRIT_AFTER consecutive non-OK runs
  1 SUSPECT     the first non-OK run (not yet paged; never OK)
  3 UNVERIFIED  the probe could not run or too few genuine Shorts loaded.
                NEVER reported as OK; it counts as a non-OK run.
Exit 0 is returned only for status OK.
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import subprocess
import sys

MIN_SHORTS = 3
WARN_AFTER = 2
CRIT_AFTER = 4


def evaluate(prev_streak: int, probe: dict | None) -> tuple[str, int, str]:
    """Pure: (status, new_streak, reason). `probe` is the probe JSON or None."""
    if probe is None:
        return _bad(prev_streak, "UNVERIFIED", "probe did not run or produced no result")
    rows = probe.get("results", [])
    shorts = [r for r in rows if r.get("counts_as_short")]
    if len(shorts) < MIN_SHORTS:
        return _bad(prev_streak, "UNVERIFIED",
                    f"only {len(shorts)} genuine Shorts loaded (need {MIN_SHORTS}); non-Shorts are excluded")
    played = sum(1 for r in shorts if r.get("play"))
    if played == len(shorts):
        return "OK", 0, f"{played}/{len(shorts)} genuine Shorts played"
    return _bad(prev_streak, "FAILING", f"{played}/{len(shorts)} genuine Shorts played")


def _bad(prev: int, kind: str, reason: str) -> tuple[str, int, str]:
    n = prev + 1
    if n >= CRIT_AFTER:
        return "CRITICAL", n, f"{kind}: {reason}"
    if n >= WARN_AFTER:
        return "DEGRADED", n, f"{kind}: {reason}"
    # first miss: do not page yet, but it is never OK
    return "SUSPECT" if kind == "FAILING" else "UNVERIFIED", n, f"{kind}: {reason}"


EXIT = {"OK": 0, "SUSPECT": 1, "DEGRADED": 1, "CRITICAL": 2, "UNVERIFIED": 3}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--label", required=True, help="egress path label, e.g. exit-ipv4")
    ap.add_argument("--proxy", help="SOCKS URL of the path under test (default: direct)")
    ap.add_argument("--state-dir", default="/var/lib/singbox-vpn/shorts-health")
    ap.add_argument("--probe", default=os.path.join(os.path.dirname(__file__), "..", "youtube_evidence_probe.py"))
    ap.add_argument("--python", default=sys.executable)
    a = ap.parse_args()
    os.makedirs(a.state_dir, mode=0o750, exist_ok=True)
    state_file = os.path.join(a.state_dir, f"{a.label}.state.json")
    try:
        prev = json.load(open(state_file)).get("streak", 0)
    except (OSError, ValueError):
        prev = 0
    out = os.path.join(a.state_dir, f"{a.label}.last-probe.json")
    cmd = [a.python, a.probe, "--label", a.label, "--videos", "--shorts", *DEFAULT_SHORTS, "--out", out]
    if a.proxy:
        cmd += ["--proxy", a.proxy]
    probe = None
    try:
        subprocess.run(cmd, capture_output=True, text=True, timeout=600)
        probe = json.load(open(out))
    except Exception:  # noqa: BLE001 - any failure to obtain a result is UNVERIFIED
        probe = None
    status, streak, reason = evaluate(prev, probe)
    now = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")
    with open(state_file, "w") as f:
        json.dump({"streak": streak, "status": status, "utc": now}, f)
    line = {"utc": now, "label": a.label, "status": status, "streak": streak, "reason": reason}
    with open(os.path.join(a.state_dir, "shorts-health.jsonl"), "a") as f:
        f.write(json.dumps(line) + "\n")
    print(json.dumps(line))
    return EXIT[status]


# Rotated by the operator; must be genuine Shorts ids (checked at run time:
# an id that is not a Short is excluded from the count, not counted as a pass).
DEFAULT_SHORTS = ["sAcElROnYIE", "NlHemkN8xq4", "GJVO2DrTqUY"]

if __name__ == "__main__":
    sys.exit(main())
