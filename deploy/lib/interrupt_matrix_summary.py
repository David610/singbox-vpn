#!/usr/bin/env python3
"""Aggregate verdict of one interrupt-matrix invocation. usage: interrupt_matrix_summary.py MATRIX_JSONL FIRST_LINE EXPECTED_STAGES
Exit 0 only if EXPECTED_STAGES stages ran and every one is PASS; 1 if any is FAIL; 3 if none FAILed but a stage is
INVALID or missing. A caller must never read the harness exit code as qualification while a stage is not PASS."""
import json
import sys

path, first, expected = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
rows = []
try:
    with open(path) as f:
        rows = [json.loads(line) for line in f.read().splitlines()[first:] if line.strip()]
except FileNotFoundError:
    pass
by = {}
for r in rows:
    by.setdefault(r.get("verdict", "INVALID"), []).append(r.get("stage"))
fail, invalid = by.get("FAIL", []), by.get("INVALID", [])
passed = by.get("PASS", [])
missing = expected - len(rows)
print(f"matrix summary: PASS={len(passed)} FAIL={len(fail)} INVALID={len(invalid)} missing={max(missing, 0)} of {expected}")
if fail:
    print(f"matrix FAIL stages: {fail}")
if invalid:
    print(f"matrix INVALID stages (not PASS, rerun them): {invalid}")
sys.exit(1 if fail else 0 if len(passed) == expected and not invalid and missing <= 0 else 3)
