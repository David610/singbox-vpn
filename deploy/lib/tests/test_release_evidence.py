"""Unit tests for deploy/lib/release_evidence.py (stdlib unittest)."""
import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "release_evidence.py")
FIX = os.path.join(HERE, "fixtures", "release-evidence", "production-ready.json")
C = "a" * 40


def run(doc, *args):
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(doc, f)
    try:
        p = subprocess.run([sys.executable, "-I", TOOL, f.name, *args], capture_output=True, text=True)
    finally:
        os.unlink(f.name)
    level = next((l.split(": ", 1)[1] for l in p.stdout.splitlines() if l.startswith("supported_level:")), None)
    return p.returncode, level, p.stdout


class ReleaseEvidence(unittest.TestCase):
    def base(self):
        with open(FIX) as f:
            return copy.deepcopy(json.load(f))

    def test_full_evidence_is_production_ready(self):
        rc, lvl, _ = run(self.base(), "--require", "PRODUCTION_READY", "--tag", "v9.9.9", "--commit", C)
        self.assertEqual((rc, lvl), (0, "PRODUCTION_READY"))

    def test_waived_device_cells_cannot_be_device_verified_and_claim_is_rejected(self):
        d = self.base()
        for cell in d["device_matrix"]:
            if cell["network"] == "cellular":
                cell["result"] = "WAIVED"
        d["waivers"] = [{"scope": "cellular device matrix", "reason": "no SIM", "approver": "owner",
                         "date": "2026-10-25", "risk": "carrier NAT/UDP untested"}]
        rc, lvl, out = run(d)  # still claims PRODUCTION_READY
        self.assertEqual(lvl, "SERVER_QUALIFIED")
        self.assertEqual(rc, 1, out)
        self.assertIn("claimed_level", out)
        d["claimed_level"] = "SERVER_QUALIFIED"
        rc, lvl, _ = run(d)
        self.assertEqual((rc, lvl), (0, "SERVER_QUALIFIED"))  # honest limited-risk record is allowed
        rc, _, _ = run(d, "--require", "DEVICE_VERIFIED")
        self.assertEqual(rc, 1)

    def test_a_waiver_never_raises_the_level_even_with_all_cells_passing(self):
        d = self.base()
        d["waivers"] = [{"scope": "x", "reason": "x", "approver": "x", "date": "x", "risk": "x"}]
        rc, lvl, _ = run(d, "--require", "PRODUCTION_READY")
        self.assertEqual(lvl, "DEVICE_VERIFIED")
        self.assertEqual(rc, 1)

    def test_waiver_without_risk_or_approver_is_invalid(self):
        d = self.base()
        d["claimed_level"] = "DEVICE_VERIFIED"
        d["waivers"] = [{"scope": "x", "reason": "x"}]
        rc, _, out = run(d)
        self.assertEqual(rc, 1)
        self.assertIn("waivers[0].approver missing", out)

    def test_cell_for_another_commit_does_not_count(self):
        d = self.base()
        d["device_matrix"][0]["git_commit"] = "b" * 40
        rc, lvl, _ = run(d, "--require", "DEVICE_VERIFIED")
        self.assertEqual((rc, lvl), (1, "SERVER_QUALIFIED"))

    def test_incomplete_shorts_or_missing_known_failing_short_does_not_count(self):
        for field, value in (("shorts_consecutive_played", 9), ("known_failing_short_played", False),
                             ("evidence_ref", ""), ("hiddify_version", None)):
            d = self.base()
            d["device_matrix"][1][field] = value
            rc, lvl, _ = run(d, "--require", "DEVICE_VERIFIED")
            self.assertEqual((rc, lvl), (1, "SERVER_QUALIFIED"), field)

    def test_ci_on_a_different_commit_is_not_code_qualified(self):
        d = self.base()
        d["ci"]["git_commit"] = "c" * 40
        rc, lvl, _ = run(d, "--require", "CODE_QUALIFIED")
        self.assertEqual((rc, lvl), (1, "NONE"))

    def test_vps_not_run_or_other_commit_is_not_server_qualified(self):
        d = self.base()
        d["vps_acceptance"]["git_commit"] = "d" * 40
        self.assertEqual(run(d, "--require", "SERVER_QUALIFIED")[:2], (1, "CODE_QUALIFIED"))

    def test_deployed_config_drift_blocks_production_ready(self):
        d = self.base()
        d["deployment"]["config_fingerprint"] = "0" * 64
        self.assertEqual(run(d, "--require", "PRODUCTION_READY")[:2], (1, "DEVICE_VERIFIED"))

    def test_failing_security_gate_blocks_production_ready(self):
        d = self.base()
        d["security_gates"]["result"] = "FAIL"
        self.assertEqual(run(d, "--require", "PRODUCTION_READY")[:2], (1, "DEVICE_VERIFIED"))

    def test_binding_to_wrong_tag_or_commit_is_refused(self):
        self.assertEqual(run(self.base(), "--tag", "v1.0.0")[0], 1)
        self.assertEqual(run(self.base(), "--commit", "e" * 40)[0], 1)

    def test_missing_or_malformed_artifact_digest_is_refused(self):
        d = self.base()
        d["artifact_digests"] = {"x": "sha1:abc"}
        self.assertEqual(run(d)[1], "NONE")
        self.assertEqual(run(d)[0], 1)

    def test_manual_pass_text_is_not_evidence(self):
        self.assertEqual(run({"schema_version": 1, "release_tag": "v1", "device_acceptance": "PASS"})[0], 1)


if __name__ == "__main__":
    unittest.main()
