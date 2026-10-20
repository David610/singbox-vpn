import importlib.util
import os
import unittest

spec = importlib.util.spec_from_file_location(
    "sh", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "monitoring", "shorts_health.py"))
sh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sh)


def probe(*plays, non_shorts=0):
    rows = [{"kind": "short", "counts_as_short": True, "play": p} for p in plays]
    rows += [{"kind": "short", "counts_as_short": False, "play": True} for _ in range(non_shorts)]
    return {"results": rows}


class ShortsHealth(unittest.TestCase):
    def test_all_genuine_shorts_playing_is_ok_and_resets_streak(self):
        self.assertEqual(sh.evaluate(3, probe(True, True, True))[:2], ("OK", 0))

    def test_non_shorts_never_inflate_the_count(self):
        # 0 of 3 genuine played, 5 non-Short items 'played' -> must NOT be OK
        st, n, _ = sh.evaluate(0, probe(False, False, False, non_shorts=5))
        self.assertNotEqual(st, "OK")
        self.assertEqual(n, 1)

    def test_too_few_genuine_shorts_is_unverified_not_ok(self):
        st, _, why = sh.evaluate(0, probe(True, True, non_shorts=4))
        self.assertEqual(st, "UNVERIFIED")
        self.assertIn("genuine", why)

    def test_probe_that_did_not_run_is_unverified_never_ok(self):
        self.assertEqual(sh.evaluate(0, None)[0], "UNVERIFIED")

    def test_streak_thresholds(self):
        s = 0
        seen = []
        for _ in range(4):
            st, s, _ = sh.evaluate(s, probe(False, False, False))
            seen.append(st)
        self.assertEqual(seen, ["SUSPECT", "DEGRADED", "DEGRADED", "CRITICAL"])

    def test_only_ok_exits_zero(self):
        self.assertEqual([k for k, v in sh.EXIT.items() if v == 0], ["OK"])

    def test_one_bad_run_between_good_runs_does_not_page(self):
        st, s, _ = sh.evaluate(0, probe(True, False, True))
        self.assertEqual((st, s), ("SUSPECT", 1))
        self.assertEqual(sh.evaluate(s, probe(True, True, True))[:2], ("OK", 0))


if __name__ == "__main__":
    unittest.main()
