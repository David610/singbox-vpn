import importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("tp", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tunnel_perf.py"))
tp = importlib.util.module_from_spec(spec); spec.loader.exec_module(tp)

class Helpers(unittest.TestCase):
    def test_percentile_edges(self):
        self.assertEqual(tp.pct([], .95), None)
        self.assertEqual(tp.pct([5], .95), 5)
        self.assertEqual(tp.pct(list(range(1, 101)), .95), 95)
        self.assertEqual(tp.pct([3, 1, 2], 0), 1)
    def test_thresholds_are_the_documented_ones(self):
        import re
        doc = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "docs", "operations", "ACCEPTANCE_THRESHOLDS.md")).read()
        self.assertIn(">= 70%", doc); self.assertIn("<= 40 ms", doc); self.assertIn("Hysteria2 <= 35 s", doc)
        src = open(tp.__file__).read()
        self.assertIn('"--min-ratio", type=float, default=0.70', src)
        self.assertIn('"--max-added-ms", type=float, default=40.0', src)
        self.assertIn('"--max-hy2-recovery-s", type=float, default=35.0', src)
        self.assertIn('"--max-reality-recovery-s", type=float, default=10.0', src)
    def test_tool_states_what_it_does_not_measure(self):
        src = open(tp.__file__).read()
        for phrase in ("real client access network", "carrier NAT", "LOOPBACK client"):
            self.assertIn(phrase, src)

if __name__ == "__main__": unittest.main()
