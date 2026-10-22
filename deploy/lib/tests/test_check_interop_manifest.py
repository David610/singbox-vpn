"""deploy/lib/ci/check_interop_manifest.py: the singbox-validate job's exact test inventory (replaces `expected=11`)."""
import importlib.util
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
spec = importlib.util.spec_from_file_location("cim", os.path.join(ROOT, "deploy", "lib", "ci", "check_interop_manifest.py"))
cim = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cim)
MANIFEST = os.path.join(ROOT, "deploy", "lib", "ci", "interop-tests.manifest")


def log(entries, extra=()):
    """A cargo test --nocapture log: one 'Running' header per suite, then 'test name ... RESULT' lines."""
    out, cur = [], None
    for suite, name, result in entries:
        if suite != cur:
            out.append(f"     Running tests/{suite}.rs (target/debug/deps/{suite}-0123456789abcdef)")
            cur = suite
        out.append(f"test {name} ... {result}")
    return "\n".join(out + list(extra)) + "\n"


def all_ok(manifest):
    return [(s, n, "ok") for s, n in manifest]


class Check(unittest.TestCase):
    def setUp(self):
        self.m = cim.load_manifest(MANIFEST)

    def test_manifest_matches_the_test_functions_in_the_sources(self):
        self.assertEqual(set(self.m), cim.source_inventory(ROOT, {s for s, _ in self.m}))

    def test_the_old_hard_coded_count_was_wrong(self):
        # reproduces run 39799935640: 12 tests passed, the job expected 11
        self.assertEqual(len(self.m), 12)
        self.assertEqual(len(cim.passed(log(all_ok(self.m)))), 12)

    def test_full_pass_is_accepted(self):
        self.assertEqual(cim.check(log(all_ok(self.m)), self.m), [])

    def test_a_missing_case_fails(self):
        errs = cim.check(log(all_ok(self.m)[1:]), self.m)
        self.assertTrue(any("did not pass" in e and self.m[0][1] in e for e in errs), errs)

    def test_a_failed_case_fails(self):
        entries = all_ok(self.m); entries[3] = (entries[3][0], entries[3][1], "FAILED")
        self.assertTrue(cim.check(log(entries), self.m))

    def test_an_ignored_case_fails(self):
        entries = all_ok(self.m); entries[0] = (entries[0][0], entries[0][1], "ignored")
        self.assertTrue(cim.check(log(entries), self.m))

    def test_a_runtime_skip_fails(self):
        errs = cim.check(log(all_ok(self.m), extra=["skipping: SING_BOX_BIN not set"]), self.m)
        self.assertTrue(any("skipping" in e for e in errs), errs)

    def test_an_unlisted_extra_test_fails(self):
        errs = cim.check(log(all_ok(self.m) + [("hysteria2_interop", "brand_new_test", "ok")]), self.m)
        self.assertTrue(any("not in the manifest" in e for e in errs), errs)

    def test_same_name_in_the_wrong_suite_fails(self):
        entries = all_ok(self.m); entries[0] = ("hysteria2_interop", entries[0][1], "ok")
        self.assertTrue(cim.check(log(entries), self.m))

    def test_a_duplicate_pass_fails(self):
        self.assertTrue(cim.check(log(all_ok(self.m) + [all_ok(self.m)[-1]]), self.m))

    def test_coloured_cargo_output_is_parsed(self):
        # CI sets CARGO_TERM_COLOR=always: cargo colours "Running" (and the ok/FAILED words). Run 39807318906 failed every
        # entry because the uncoloured regex never matched a suite header.
        coloured = log(all_ok(self.m)).replace("     Running", "\x1b[1m\x1b[32m     Running\x1b[0m").replace(
            "... ok", "... \x1b[32mok\x1b[0m")
        self.assertEqual(cim.check(coloured, self.m), [])
        bad = coloured.replace(f"test {self.m[0][1]} ... \x1b[32mok\x1b[0m", f"test {self.m[0][1]} ... \x1b[31mFAILED\x1b[0m")
        self.assertTrue(cim.check(bad, self.m))

    def test_an_empty_log_fails(self):
        self.assertTrue(cim.check("", self.m))

    def test_malformed_manifest_line_is_rejected(self):
        with self.assertRaises(ValueError):
            cim.parse_manifest("reality_interop:missing_double_colon\n")


if __name__ == "__main__":
    unittest.main()
