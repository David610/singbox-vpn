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
        self.assertIn("<= 40 ms", doc); self.assertIn("Hysteria2 <= 35 s", doc)
        src = open(tp.__file__).read()
        self.assertIn('"--min-mbps-reality", type=float, default=200.0', src)
        self.assertIn('"--min-mbps-hy2", type=float, default=100.0', src)
        self.assertIn('"--max-added-ms", type=float, default=40.0', src)
        self.assertIn('"--max-hy2-recovery-s", type=float, default=35.0', src)
        self.assertIn('"--max-reality-recovery-s", type=float, default=10.0', src)
    def test_latency_is_end_to_end_not_the_local_proxy_connect(self):
        src = open(tp.__file__).read()
        self.assertIn('"%{time_total}"', src)
        self.assertNotIn('"%{time_connect}"', src)
    def test_rate_limited_source_is_never_a_pass_or_a_tunnel_failure(self):
        import subprocess
        class P:  # fake curl output
            def __init__(s, out): s.stdout = out
        real = subprocess.run
        try:
            for out, want in (("429 1 0.03", "limited"), ("403 1 0.03", "limited"), ("200 25000000 0.5", "ok"), ("206 25000000 0.5", "ok"),
                              ("206 1000 0.5", "failed"), ("500 0 0.1", "failed"), ("", "failed")):
                subprocess.run = lambda *a, _o=out, **k: P(_o)
                self.assertEqual(tp.curl_download("u", None, 5, 25000000)[0], want, out)
        finally:
            subprocess.run = real
    def test_concurrency_uses_a_supported_download_size(self):
        src = open(tp.__file__).read()
        self.assertNotIn("bytes // 2", src)   # Cloudflare answers 403 for 12.5 MB: that was a tool bug, not a tunnel failure
    def test_tool_states_what_it_does_not_measure(self):
        src = open(tp.__file__).read()
        for phrase in ("real client access network", "carrier NAT", "LOOPBACK client"):
            self.assertIn(phrase, src)

if __name__ == "__main__": unittest.main()
