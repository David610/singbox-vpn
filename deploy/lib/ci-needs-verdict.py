#!/usr/bin/env python3
"""Fail unless every job in $NEEDS_JSON (GitHub's toJSON(needs)) succeeded.

`offline-full-verdict` is the required status check. By default a job whose
dependency failed is SKIPPED, and a skipped required check counts as passing,
so the verdict job runs `if: always()` and calls this to turn "failure",
"cancelled" and "skipped" into a failed check. Unusable input fails closed.
"""
import json
import os
import sys


def main() -> int:
    raw = os.environ.get("NEEDS_JSON", "")
    try:
        needs = json.loads(raw)
    except ValueError:
        print("ci-needs-verdict: NEEDS_JSON is missing or not JSON", file=sys.stderr)
        return 1
    if not isinstance(needs, dict) or not needs:
        print("ci-needs-verdict: NEEDS_JSON must be a non-empty object of jobs", file=sys.stderr)
        return 1
    bad = []
    for job, info in sorted(needs.items()):
        result = info.get("result") if isinstance(info, dict) else None
        if result != "success":
            bad.append(f"{job}={result!r}")
    if bad:
        print("ci-needs-verdict: required jobs did not succeed: " + ", ".join(bad), file=sys.stderr)
        return 1
    print(f"ci-needs-verdict: all {len(needs)} required jobs succeeded")
    return 0


if __name__ == "__main__":
    sys.exit(main())
