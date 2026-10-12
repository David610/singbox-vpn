#!/usr/bin/env python3
"""Synthetic-run tests for deploy/lib/soak_evaluate.py's fixed criteria."""
import importlib.util
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
path = Path(__file__).resolve().parents[1] / "soak_evaluate.py"
spec = importlib.util.spec_from_file_location("soak_evaluate", path)
soak = importlib.util.module_from_spec(spec)
spec.loader.exec_module(soak)


def run(count=200, step=30, **overrides):
    """A healthy 100-minute run; keyword overrides mutate per-row values."""
    rows = []
    for i in range(count):
        row = {
            "epoch": str(1_000_000 + i * step),
            "singbox_active": "1", "singbox_pid": "4242", "singbox_restarts": "0",
            "singbox_rss_kb": str(60_000 + (i % 7) * 50), "singbox_fds": str(120 + (i % 5)),
            "subscription_active": "1", "reconcile_result": "success",
            "reconcile_runs": str(i * step // 600), "watchdog_active": "1",
            "selftest": "pass" if i % 20 == 0 else "na",
        }
        for key, fn in overrides.items():
            row[key] = fn(i, row[key])
        rows.append(row)
    return rows


def verdict(rows, **kwargs):
    results = soak.evaluate(rows, **kwargs)
    return all(r["pass"] for r in results), {r["criterion"]: r for r in results}


class Soak(unittest.TestCase):
    def test_a_healthy_run_passes_every_criterion(self):
        ok, by = verdict(run())
        self.assertTrue(ok, by)
        self.assertEqual(len(by), 8)

    def test_too_short_a_run_fails_even_if_everything_else_is_fine(self):
        ok, by = verdict(run(count=40))
        self.assertFalse(by["duration"]["pass"])
        self.assertFalse(ok)

    def test_a_single_unavailable_sample_fails_availability(self):
        ok, by = verdict(run(singbox_active=lambda i, v: "0" if i == 100 else v))
        self.assertFalse(by["availability"]["pass"])

    def test_a_restart_is_detected_from_the_pid_and_from_the_counter(self):
        ok, by = verdict(run(singbox_pid=lambda i, v: "5555" if i > 120 else v))
        self.assertFalse(by["restarts"]["pass"], by["restarts"])
        ok, by = verdict(run(singbox_restarts=lambda i, v: "1" if i > 50 else v))
        self.assertFalse(by["restarts"]["pass"])
        ok, by = verdict(run(singbox_restarts=lambda i, v: "1" if i > 50 else v), allowed_restarts=1)
        self.assertTrue(by["restarts"]["pass"], "declared planned restarts must be allowed")

    def test_memory_and_descriptor_growth_fail(self):
        ok, by = verdict(run(singbox_rss_kb=lambda i, v: str(60_000 + i * 600)))
        self.assertFalse(by["rss growth"]["pass"])
        ok, by = verdict(run(singbox_fds=lambda i, v: str(120 + i)))
        self.assertFalse(by["fd growth"]["pass"])

    def test_a_failed_reconcile_or_too_few_cycles_fail(self):
        ok, by = verdict(run(reconcile_result=lambda i, v: "exit-code" if i == 90 else v))
        self.assertFalse(by["reconcile"]["pass"])
        ok, by = verdict(run(reconcile_runs=lambda i, v: "0"))
        self.assertFalse(by["reconcile"]["pass"])

    def test_a_failed_selftest_and_a_missing_selftest_both_fail(self):
        ok, by = verdict(run(selftest=lambda i, v: "fail" if i == 100 else v))
        self.assertFalse(by["selftest"]["pass"])
        ok, by = verdict(run(selftest=lambda i, v: "na"))
        self.assertFalse(by["selftest"]["pass"], "no self-test at all must not count as a pass")

    def test_inactive_watchdog_fails(self):
        ok, by = verdict(run(watchdog_active=lambda i, v: "0" if i == 3 else v))
        self.assertFalse(by["watchdog"]["pass"])

    def test_missing_data_is_never_a_pass(self):
        ok, by = verdict(run(count=2))
        self.assertFalse(ok)
        rows = run()
        for row in rows:
            row["singbox_rss_kb"] = ""
        ok, by = verdict(rows)
        self.assertFalse(by["rss growth"]["pass"])
        self.assertFalse(ok)


if __name__ == "__main__":
    unittest.main()
