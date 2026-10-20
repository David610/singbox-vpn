#!/usr/bin/env python3
"""Release-candidate acceptance report from a gate list.

    rc_report.py GATES.json [--md OUT.md] [--repo-root DIR]

Each gate: {id, area, name, mandatory, status, evidence:[repo-relative paths], note}.
status in PASS | FAIL | FLAKY | BLOCKED | UNVERIFIED. Rules (violations exit 2):
  * PASS and FLAKY need >= 1 evidence path and every path must exist;
  * FAIL / BLOCKED / UNVERIFIED / FLAKY need a note (why, and what is needed);
  * the overall verdict is computed: ACCEPTABLE only if EVERY mandatory gate is PASS.
Exit 0 = every mandatory gate PASS; 1 = report valid but not acceptable.
"""
import collections
import json
import os
import sys

STATUSES = ("PASS", "FAIL", "FLAKY", "BLOCKED", "UNVERIFIED")


def validate(gates, root):
    errs = []
    seen = set()
    for g in gates:
        gid = g.get("id", "?")
        if gid in seen:
            errs.append(f"{gid}: duplicate id")
        seen.add(gid)
        st = g.get("status")
        if st not in STATUSES:
            errs.append(f"{gid}: invalid status {st!r}")
            continue
        ev = g.get("evidence") or []
        if st in ("PASS", "FLAKY"):
            if not ev:
                errs.append(f"{gid}: {st} without evidence")
            for e in ev:
                if not os.path.exists(os.path.join(root, e)):
                    errs.append(f"{gid}: evidence file missing: {e}")
        if st != "PASS" and not g.get("note"):
            errs.append(f"{gid}: {st} needs a note (why / what is required)")
        if not isinstance(g.get("mandatory"), bool):
            errs.append(f"{gid}: 'mandatory' must be true/false")
    return errs


def verdict(gates):
    bad = [g for g in gates if g.get("mandatory") and g["status"] != "PASS"]
    return ("ACCEPTABLE" if not bad else "NOT ACCEPTABLE"), bad


def render(doc, gates):
    v, bad = verdict(gates)
    c = collections.Counter(g["status"] for g in gates)
    cm = collections.Counter(g["status"] for g in gates if g["mandatory"])
    out = [f"# Release-candidate acceptance report — {doc.get('candidate', '?')}", "",
           f"Commit: `{doc.get('commit', '?')}` · generated from `{doc.get('source', 'gates.json')}` · date {doc.get('date', '?')}", "",
           f"## Verdict: **{v}**", "",
           f"Mandatory gates: {sum(cm.values())} — " + ", ".join(f"{k} {cm.get(k, 0)}" for k in STATUSES),
           f"All gates: {sum(c.values())} — " + ", ".join(f"{k} {c.get(k, 0)}" for k in STATUSES), ""]
    if bad:
        out += ["Blocking (mandatory, not PASS):", ""] + [f"* **{g['id']}** {g['name']} — {g['status']}: {g.get('note', '')}" for g in bad] + [""]
    area = collections.OrderedDict()
    for g in gates:
        area.setdefault(g["area"], []).append(g)
    for a, gs in area.items():
        out += [f"### {a}", "", "| Gate | Mandatory | Status | Evidence / note |", "|---|---|---|---|"]
        for g in gs:
            ev = ", ".join(f"`{e}`" for e in g.get("evidence", []))
            tail = (ev + (" — " if ev and g.get("note") else "") + (g.get("note") or "")).replace("|", "/")
            out.append(f"| {g['id']} {g['name']} | {'yes' if g['mandatory'] else 'no'} | **{g['status']}** | {tail} |")
        out.append("")
    return "\n".join(out) + "\n"


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__); return 2
    root = "."
    md = None
    if "--repo-root" in args:
        root = args[args.index("--repo-root") + 1]
    if "--md" in args:
        md = args[args.index("--md") + 1]
    doc = json.load(open(args[0]))
    gates = doc["gates"]
    errs = validate(gates, root)
    if errs:
        print("INVALID REPORT:")
        for e in errs:
            print("  -", e)
        return 2
    text = render(doc, gates)
    if md:
        open(md, "w").write(text)
    v, bad = verdict(gates)
    print(f"verdict: {v} ({len(bad)} mandatory gate(s) not PASS)")
    return 0 if v == "ACCEPTABLE" else 1


if __name__ == "__main__":
    sys.exit(main())
