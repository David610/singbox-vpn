#!/usr/bin/env python3
"""Evaluate a soak-sampler CSV against criteria fixed BEFORE the run.

Pass/fail is decided here, not by eyeballing a graph afterwards. Every
criterion is explicit and every default below is the release criterion; a
different value must be passed on the command line (and so appears in the log),
never edited into the result.

A criterion with no usable data FAILS (missing data is not a pass).

Criteria (defaults):
  duration        >= 5400 s (90 min): long enough for at least eight 10-minute
                  reconcile cycles and many watchdog intervals, so a restart,
                  drift or resource problem that a 15-minute test cannot show
                  has time to appear. Account-expiry boundaries are exercised
                  separately (see docs/reviews/RELEASE_1_3_0_PREPARATION.md);
                  standalone accounts have static credentials and no leases, and
                  nothing here requires a provisioning agent or native grants.
  availability    sing-box and the subscription service active in 100% of
                  samples (an unavailable sample is an outage window).
  restarts        sing-box restarts (NRestarts delta + MainPID changes) <= 0
                  unless the operator declares planned ones (--allowed-restarts):
                  a reconcile that restarts the data plane when nothing changed
                  is the defect this catches.
  rss growth      median RSS of the last 10% of samples vs the first 10%: at most
                  +25% and +64 MiB.
  fd growth       median open fds, last 10% vs first 10%: at most +50% and +64.
  reconcile       the reconcile service finished successfully in every sample
                  and ran at least 8 times.
  watchdog        the watchdog timer was active in every sample.
  selftest        every recorded protocol self-test passed, and at least one ran.

This tool never sees credentials, destinations or users: the sampler does not
record them.
"""
import argparse
import csv
import json
import statistics
import sys

FIELDS = ["epoch", "singbox_active", "singbox_pid", "singbox_restarts", "singbox_rss_kb",
          "singbox_fds", "subscription_active", "reconcile_result", "reconcile_runs",
          "watchdog_active", "selftest"]


def load(path):
    with open(path, newline="") as handle:
        rows = list(csv.DictReader(handle))
    return rows


def num(row, key):
    try:
        return int(row[key])
    except (KeyError, TypeError, ValueError):
        return None


def edge_median(values, fraction=0.10):
    count = max(1, int(len(values) * fraction))
    return statistics.median(values[:count]), statistics.median(values[-count:])


def evaluate(rows, min_duration=5400, allowed_restarts=0, min_reconcile_runs=8,
             rss_growth_pct=25.0, rss_growth_kib=64 * 1024, fd_growth_pct=50.0, fd_growth_abs=64):
    results = []

    def add(name, ok, detail):
        results.append({"criterion": name, "pass": bool(ok), "detail": detail})

    if len(rows) < 3 or any(f not in rows[0] for f in FIELDS):
        add("data", False, f"{len(rows)} rows or missing columns; refusing to evaluate")
        return results
    epochs = [num(r, "epoch") for r in rows]
    if None in epochs:
        add("data", False, "unparseable epoch column")
        return results
    span = epochs[-1] - epochs[0]
    add("duration", span >= min_duration, f"{span}s observed, {min_duration}s required")

    down = [r for r in rows if r["singbox_active"] != "1" or r["subscription_active"] != "1"]
    add("availability", not down, f"{len(down)} of {len(rows)} samples had sing-box or the subscription service inactive")

    restarts_counter = [num(r, "singbox_restarts") for r in rows]
    pids = [num(r, "singbox_pid") for r in rows if num(r, "singbox_pid")]
    if None in restarts_counter or not pids:
        add("restarts", False, "restart counter or PID missing")
    else:
        pid_changes = sum(1 for a, b in zip(pids, pids[1:]) if a != b)
        counter_delta = restarts_counter[-1] - restarts_counter[0]
        observed = max(pid_changes, counter_delta)
        add("restarts", observed <= allowed_restarts,
            f"{observed} sing-box restarts observed (PID changes {pid_changes}, NRestarts delta {counter_delta}), {allowed_restarts} allowed")

    rss = [v for v in (num(r, "singbox_rss_kb") for r in rows) if v]
    if len(rss) < 10:
        add("rss growth", False, "fewer than 10 usable RSS samples")
    else:
        first, last = edge_median(rss)
        grew = last - first
        ok = grew <= first * rss_growth_pct / 100.0 and grew <= rss_growth_kib
        add("rss growth", ok, f"median {first:.0f} KiB -> {last:.0f} KiB (+{grew:.0f} KiB)")

    fds = [v for v in (num(r, "singbox_fds") for r in rows) if v]
    if len(fds) < 10:
        add("fd growth", False, "fewer than 10 usable fd samples")
    else:
        first, last = edge_median(fds)
        grew = last - first
        ok = grew <= first * fd_growth_pct / 100.0 and grew <= fd_growth_abs
        add("fd growth", ok, f"median {first:.0f} -> {last:.0f} fds (+{grew:.0f})")

    bad = [r for r in rows if r["reconcile_result"] not in ("success", "")]
    runs = num(rows[-1], "reconcile_runs")
    first_runs = num(rows[0], "reconcile_runs") or 0
    cycles = None if runs is None else runs - first_runs
    add("reconcile", not bad and cycles is not None and cycles >= min_reconcile_runs,
        f"{cycles} reconcile runs observed ({min_reconcile_runs} required); {len(bad)} samples with a failed last result")

    add("watchdog", all(r["watchdog_active"] == "1" for r in rows), "watchdog timer active in every sample")

    tests = [r["selftest"] for r in rows if r["selftest"] in ("pass", "fail")]
    add("selftest", bool(tests) and all(t == "pass" for t in tests),
        f"{tests.count('pass')} passed, {tests.count('fail')} failed")
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("csv")
    parser.add_argument("--min-duration", type=int, default=5400)
    parser.add_argument("--allowed-restarts", type=int, default=0)
    parser.add_argument("--min-reconcile-runs", type=int, default=8)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    results = evaluate(load(args.csv), args.min_duration, args.allowed_restarts, args.min_reconcile_runs)
    verdict = all(r["pass"] for r in results)
    if args.json:
        print(json.dumps({"verdict": "PASS" if verdict else "FAIL", "criteria": results}, indent=2))
    else:
        for r in results:
            print(f"[{'PASS' if r['pass'] else 'FAIL'}] {r['criterion']}: {r['detail']}")
        print(f"SOAK VERDICT: {'PASS' if verdict else 'FAIL'}")
    return 0 if verdict else 1


if __name__ == "__main__":
    sys.exit(main())
