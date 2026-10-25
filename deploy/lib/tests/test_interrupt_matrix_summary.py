import json, os, subprocess, sys, tempfile, unittest
TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "interrupt_matrix_summary.py")

def run(verdicts, expected, first=0, old=0):
    with tempfile.NamedTemporaryFile("w", delete=False) as f:
        for i in range(old):
            f.write(json.dumps({"stage": 99, "verdict": "FAIL"}) + "\n")
        for i, v in enumerate(verdicts, 1):
            f.write(json.dumps({"stage": i, "verdict": v}) + "\n")
    try:
        return subprocess.run([sys.executable, "-I", TOOL, f.name, str(first), str(expected)], capture_output=True, text=True).returncode
    finally:
        os.unlink(f.name)

class Summary(unittest.TestCase):
    def test_all_pass_is_zero(self): self.assertEqual(run(["PASS"] * 16, 16), 0)
    def test_any_fail_is_nonzero(self): self.assertEqual(run(["PASS"] * 15 + ["FAIL"], 16), 1)
    def test_invalid_without_fail_is_not_success(self): self.assertEqual(run(["PASS"] * 14 + ["INVALID", "PASS"], 16), 3)
    def test_fail_dominates_invalid(self): self.assertEqual(run(["INVALID", "FAIL"] + ["PASS"] * 14, 16), 1)
    def test_missing_stages_are_not_success(self): self.assertEqual(run(["PASS"] * 15, 16), 3)
    def test_no_rows_is_not_success(self): self.assertEqual(run([], 16), 3)
    def test_only_this_invocations_rows_count(self): self.assertEqual(run(["PASS"] * 16, 16, first=2, old=2), 0)
    def test_earlier_invocation_fail_does_not_count_toward_this_one(self): self.assertEqual(run(["PASS"], 1, first=1, old=1), 0)

if __name__ == "__main__": unittest.main()
