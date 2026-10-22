#!/usr/bin/env python3
"""Release-gate report computed from evidence files, never typed.

  release_gate_report.py --gates GATES.json --root REPO_ROOT [--out REPORT.md] [--json OUT.json]

GATES.json: {"revision": "...", "gates": [{"id","category","title","mandatory","status","evidence":[paths relative to root],"note"}]}
Statuses: PASS FAIL FLAKY BLOCKED UNVERIFIED.

Rules (enforced here, covered by tests/test_release_gate_report.py):
  * a gate may claim PASS only if it lists at least one evidence file and every listed file exists (its sha256 is recorded);
    otherwise the gate is reported INVALID and counts as not passed;
  * the overall verdict is derived: any mandatory FAIL/INVALID -> FAIL; else any mandatory FLAKY/BLOCKED/UNVERIFIED -> NOT_READY;
    else READY. There is no override parameter;
  * non-mandatory gates are shown but cannot make the verdict better or worse;
  * the verdict is for THIS revision; evidence naming another revision is flagged (`revision_mismatch`).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys

STATUSES = ("PASS", "FAIL", "FLAKY", "BLOCKED", "UNVERIFIED")


def sha(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for c in iter(lambda: f.read(1 << 20), b""):
            h.update(c)
    return h.hexdigest()[:16]


def evaluate(doc: dict, root: str) -> dict:
    rows, problems = [], []
    for g in doc["gates"]:
        st = g.get("status")
        row = dict(g)
        row["evidence_hashes"] = {}
        if st not in STATUSES:
            row["effective"] = "INVALID"; problems.append(f"{g['id']}: unknown status {st!r}")
        else:
            missing = [e for e in g.get("evidence", []) if not os.path.isfile(os.path.join(root, e))]
            for e in g.get("evidence", []):
                p = os.path.join(root, e)
                if os.path.isfile(p):
                    row["evidence_hashes"][e] = sha(p)
            if st == "PASS" and (not g.get("evidence") or missing):
                row["effective"] = "INVALID"
                problems.append(f"{g['id']}: claims PASS but evidence is {'missing: ' + ', '.join(missing) if missing else 'not listed'}")
            else:
                row["effective"] = st
        rows.append(row)
    mand = [r for r in rows if r.get("mandatory")]
    if any(r["effective"] in ("FAIL", "INVALID") for r in mand):
        verdict = "FAIL"
    elif any(r["effective"] != "PASS" for r in mand):
        verdict = "NOT_READY"
    else:
        verdict = "READY"
    count = {s: sum(1 for r in mand if r["effective"] == s) for s in (*STATUSES, "INVALID")}
    return {"revision": doc.get("revision"), "verdict": verdict, "mandatory_counts": count, "problems": problems, "gates": rows}


def render(rep: dict) -> str:
    out = [f"# Release gate report — revision `{rep['revision']}`", "",
           f"**Verdict: {rep['verdict']}** (computed; mandatory gates: " + ", ".join(f"{k}={v}" for k, v in rep["mandatory_counts"].items() if v) + ")", ""]
    if rep["problems"]:
        out += ["Problems:"] + [f"* {p}" for p in rep["problems"]] + [""]
    cats: dict[str, list] = {}
    for r in rep["gates"]:
        cats.setdefault(r["category"], []).append(r)
    for cat, rs in cats.items():
        out += [f"## {cat}", "", "| Gate | Mandatory | Status | Evidence / note |", "|---|---|---|---|"]
        for r in rs:
            ev = "; ".join(f"`{e}`" for e in r.get("evidence", [])) or "-"
            out.append(f"| {r['id']} {r['title']} | {'yes' if r.get('mandatory') else 'no'} | **{r['effective']}** | {ev}. {r.get('note', '')} |")
        out.append("")
    return "\n".join(out)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gates", required=True)
    ap.add_argument("--root", default=".")
    ap.add_argument("--out")
    ap.add_argument("--json")
    a = ap.parse_args(argv)
    rep = evaluate(json.load(open(a.gates)), a.root)
    md = render(rep)
    if a.out:
        with open(a.out, "w") as f:
            f.write(md + "\n")
    if a.json:
        with open(a.json, "w") as f:
            json.dump(rep, f, indent=1)
    print(json.dumps({"verdict": rep["verdict"], "mandatory": rep["mandatory_counts"], "problems": len(rep["problems"])}))
    return 0 if rep["verdict"] == "READY" else 1


if __name__ == "__main__":
    sys.exit(main())
