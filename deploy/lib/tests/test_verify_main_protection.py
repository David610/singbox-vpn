import copy
import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "verify-main-protection.py")
LIVE = os.path.join(HERE, "fixtures", "branch-protection", "live-2026-10-22-protect-main.json")

import importlib.util

spec = importlib.util.spec_from_file_location("vmp", TOOL)
vmp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vmp)


def with_checks(ruleset, contexts=vmp.REQUIRED_CHECKS, strict=True):
    doc = copy.deepcopy(ruleset)
    doc["rules"].append({
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": strict,
            "do_not_enforce_on_create": False,
            "required_status_checks": [{"context": c} for c in contexts],
        },
    })
    return doc


def failing(findings):
    return sorted(f["check"] for f in findings if not f["ok"])


class Verify(unittest.TestCase):
    def setUp(self):
        self.live = json.load(open(LIVE))

    def test_the_live_ruleset_of_2026_10_22_is_reported_as_not_enforced(self):
        failed = failing(vmp.verify(self.live))
        self.assertIn("a required_status_checks rule exists", failed)
        for context in vmp.REQUIRED_CHECKS:
            self.assertIn(f"required check '{context}'", failed)
        # ...while everything that already protects main is intact.
        for kept in ("no bypass actors", "deletion is blocked", "a pull request is required",
                     "non-fast-forward updates are blocked", "enforcement is active"):
            self.assertNotIn(kept, failed)

    def test_the_target_state_passes(self):
        self.assertEqual(failing(vmp.verify(with_checks(self.live))), [])

    def test_a_missing_single_check_is_named(self):
        doc = with_checks(self.live, ("offline-full-verdict", "codeql-rust"))
        self.assertEqual(failing(vmp.verify(doc)), ["required check 'codeql-actions'"])

    def test_non_strict_policy_does_not_count_as_current_commits(self):
        doc = with_checks(self.live, strict=False)
        self.assertEqual(failing(vmp.verify(doc)), ["checks must be up to date with the base (current commits)"])

    def test_weakening_existing_protection_is_caught(self):
        for drop in ("deletion", "non_fast_forward", "pull_request"):
            doc = with_checks(self.live)
            doc["rules"] = [r for r in doc["rules"] if r["type"] != drop]
            self.assertTrue(failing(vmp.verify(doc)), drop)
        doc = with_checks(self.live)
        doc["bypass_actors"] = [{"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always"}]
        self.assertEqual(failing(vmp.verify(doc)), ["no bypass actors"])
        doc = with_checks(self.live)
        doc["enforcement"] = "evaluate"
        self.assertEqual(failing(vmp.verify(doc)), ["enforcement is active"])
        doc = with_checks(self.live)
        doc["conditions"]["ref_name"]["exclude"] = ["refs/heads/main"]
        self.assertEqual(failing(vmp.verify(doc)), ["applies to the default branch"])

    def test_the_emitted_update_payload_fixes_the_gap_and_weakens_nothing(self):
        body = vmp.update_payload(self.live)
        self.assertEqual(failing(vmp.verify(body)), [])
        before = [r for r in self.live["rules"] if r["type"] != "required_status_checks"]
        after = [r for r in body["rules"] if r["type"] != "required_status_checks"]
        self.assertEqual(before, after, "existing rules must be preserved exactly")
        self.assertEqual(body["bypass_actors"], [])
        self.assertEqual(body["conditions"], self.live["conditions"])
        self.assertNotIn("id", body)
        # idempotent: applying it to an already-correct ruleset changes nothing
        self.assertEqual(vmp.update_payload(body), body)

    def test_cli_exit_codes_and_unreadable_input(self):
        run = lambda *a: subprocess.run([sys.executable, "-I", TOOL, *a], capture_output=True, text=True)
        self.assertEqual(run(LIVE).returncode, 1)
        self.assertIn("NOT ENFORCED", run(LIVE).stdout)
        good = os.path.join(os.environ.get("TMPDIR", "/tmp"), f"vmp-good-{os.getpid()}.json")
        try:
            json.dump(with_checks(self.live), open(good, "w"))
            self.assertEqual(run(good).returncode, 0)
        finally:
            os.unlink(good)
        self.assertEqual(run("/nonexistent.json").returncode, 2)


if __name__ == "__main__":
    unittest.main()
