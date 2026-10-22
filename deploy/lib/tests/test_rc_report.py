import importlib.util, os, shutil, tempfile, unittest
spec = importlib.util.spec_from_file_location("rr", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "rc_report.py"))
rr = importlib.util.module_from_spec(spec); spec.loader.exec_module(rr)

def g(id_, status, mandatory=True, evidence=None, note=""):
    return {"id": id_, "area": "A", "name": id_, "mandatory": mandatory, "status": status, "evidence": evidence or [], "note": note}

class Report(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.d, ignore_errors=True)
        open(os.path.join(self.d, "ev.txt"), "w").write("x")
    def test_pass_without_evidence_is_invalid(self):
        self.assertTrue(any("without evidence" in e for e in rr.validate([g("G1", "PASS")], self.d)))
    def test_pass_with_missing_evidence_file_is_invalid(self):
        self.assertTrue(any("missing" in e for e in rr.validate([g("G1", "PASS", evidence=["nope.txt"])], self.d)))
    def test_pass_with_existing_evidence_is_valid(self):
        self.assertEqual(rr.validate([g("G1", "PASS", evidence=["ev.txt"])], self.d), [])
    def test_non_pass_requires_a_note(self):
        for st in ("FAIL", "BLOCKED", "UNVERIFIED", "FLAKY"):
            self.assertTrue(rr.validate([g("G1", st, evidence=["ev.txt"])], self.d), st)
            self.assertEqual(rr.validate([g("G1", st, evidence=["ev.txt"], note="why")], self.d), [], st)
    def test_invalid_status_and_duplicates(self):
        self.assertTrue(rr.validate([g("G1", "MOSTLY")], self.d))
        self.assertTrue(any("duplicate" in e for e in rr.validate([g("G1", "FAIL", note="n"), g("G1", "FAIL", note="n")], self.d)))
    def test_verdict_needs_every_mandatory_gate_to_pass(self):
        gs = [g("A", "PASS", evidence=["ev.txt"]), g("B", "UNVERIFIED", note="n"), g("C", "FAIL", mandatory=False, note="n")]
        v, bad = rr.verdict(gs)
        self.assertEqual((v, [x["id"] for x in bad]), ("NOT ACCEPTABLE", ["B"]))
        self.assertEqual(rr.verdict([g("A", "PASS", evidence=["ev.txt"]), g("C", "FAIL", mandatory=False, note="n")])[0], "ACCEPTABLE")
    def test_flaky_is_never_acceptable(self):
        self.assertEqual(rr.verdict([g("A", "FLAKY", evidence=["ev.txt"], note="n")])[0], "NOT ACCEPTABLE")
    def test_render_lists_blockers(self):
        text = rr.render({"candidate": "rc"}, [g("B", "BLOCKED", note="needs owner")])
        self.assertIn("NOT ACCEPTABLE", text); self.assertIn("needs owner", text)

if __name__ == "__main__": unittest.main()
