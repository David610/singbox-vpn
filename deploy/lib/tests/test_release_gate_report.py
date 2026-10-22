import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("rg", os.path.join(HERE, "..", "release_gate_report.py"))
rg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rg)


def gate(i, status, mandatory=True, evidence=("e.txt",), cat="C"):
    return {"id": i, "category": cat, "title": i, "mandatory": mandatory, "status": status, "evidence": list(evidence)}


class Gate(unittest.TestCase):
    def run_it(self, gates, files=("e.txt",)):
        with tempfile.TemporaryDirectory() as d:
            for f in files:
                open(os.path.join(d, f), "w").write("x")
            return rg.evaluate({"revision": "r", "gates": gates}, d)

    def test_all_mandatory_pass_with_evidence_is_ready(self):
        self.assertEqual(self.run_it([gate("a", "PASS"), gate("b", "PASS")])["verdict"], "READY")

    def test_a_pass_without_evidence_is_invalid_and_fails_the_release(self):
        r = self.run_it([gate("a", "PASS", evidence=())])
        self.assertEqual((r["gates"][0]["effective"], r["verdict"]), ("INVALID", "FAIL"))

    def test_a_pass_whose_evidence_file_is_missing_is_invalid(self):
        r = self.run_it([gate("a", "PASS", evidence=("nope.txt",))])
        self.assertEqual(r["verdict"], "FAIL"); self.assertTrue(r["problems"])

    def test_unverified_blocked_flaky_mandatory_gates_mean_not_ready(self):
        for st in ("UNVERIFIED", "BLOCKED", "FLAKY"):
            self.assertEqual(self.run_it([gate("a", "PASS"), gate("b", st, evidence=())])["verdict"], "NOT_READY", st)

    def test_fail_beats_not_ready(self):
        self.assertEqual(self.run_it([gate("a", "FAIL"), gate("b", "UNVERIFIED", evidence=())])["verdict"], "FAIL")

    def test_non_mandatory_gates_do_not_change_the_verdict(self):
        self.assertEqual(self.run_it([gate("a", "PASS"), gate("n", "FAIL", mandatory=False)])["verdict"], "READY")
        self.assertEqual(self.run_it([gate("a", "FAIL"), gate("n", "PASS", mandatory=False)])["verdict"], "FAIL")

    def test_unknown_status_is_invalid(self):
        self.assertEqual(self.run_it([gate("a", "GREEN")])["verdict"], "FAIL")

    def test_evidence_is_hashed(self):
        r = self.run_it([gate("a", "PASS")])
        self.assertEqual(len(r["gates"][0]["evidence_hashes"]["e.txt"]), 16)

    def test_no_override_exists(self):
        import inspect
        self.assertNotIn("override", inspect.getsource(rg).lower().replace("no override parameter", ""))


if __name__ == "__main__":
    unittest.main()
