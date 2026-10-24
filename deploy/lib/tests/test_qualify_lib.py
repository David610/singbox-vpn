import importlib.util
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import unittest


def dump(obj, path):
    with open(path, "w") as fh:
        json.dump(obj, fh)

HERE = os.path.dirname(os.path.abspath(__file__))
DIAG = os.path.join(HERE, "..", "diag")
spec = importlib.util.spec_from_file_location("ql", os.path.join(DIAG, "qualify_lib.py"))
ql = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ql)


def prow(**kw):
    base = dict(id="x", err=None, started=True, start_s=1.0, played_s=25.0, max_stall_s=0.5, bot_challenge=False,
                unavailable=False, still_on_shorts=True, videoplayback_reqs=5, refusal_155B_like=0, media_bytes=1000)
    base.update(kw)
    return base


class Classify(unittest.TestCase):
    def test_outcomes(self):
        c = lambda **kw: ql.classify_row(prow(**kw), 25)
        self.assertEqual(c(), ("played", "played"))
        self.assertEqual(c(played_s=14.9), ("negative", "stalled"))                # below 60% of the dwell
        self.assertEqual(c(played_s=15.0), ("played", "played"))
        self.assertEqual(c(max_stall_s=4.0), ("negative", "stalled"))
        self.assertEqual(c(started=False, unavailable=True, played_s=0), ("negative", "media_refused"))
        self.assertEqual(c(started=False, refusal_155B_like=2, played_s=0), ("negative", "media_refused"))
        self.assertEqual(c(started=False, played_s=0), ("negative", "no_start"))
        self.assertEqual(c(bot_challenge=True, started=False), ("negative", "bot_challenge"))

    def test_not_evidence(self):
        self.assertEqual(ql.classify_row(prow(err="TimeoutError", started=False), 25), ("unknown", "probe_error"))
        self.assertEqual(ql.classify_row(prow(still_on_shorts=False), 25), ("unknown", "not_a_short"))
        self.assertEqual(ql.classify_row("garbage", 25), ("unknown", "evidence_unreadable"))
        # a long-form video is allowed to leave /shorts/
        self.assertEqual(ql.classify_row(prow(still_on_shorts=False), 25, require_short=False), ("played", "played"))

    def test_sustained_bar_is_absolute_seconds(self):
        row = prow(played_s=590.0)
        self.assertEqual(ql.classify_row(row, 900, min_play_s=600, require_short=False), ("negative", "stalled"))
        self.assertEqual(ql.classify_row(prow(played_s=600.0), 900, min_play_s=600, require_short=False)[0], "played")


def write_run(directory, *, rows=None, rounds=1, sustained_rows="auto", ids=3, min_played=None, identity="verified",
              stages=None, tls="good", sustained_video="aaaaaaaaaaa", dwell=25):
    ids_list = ["id%08d" % i for i in range(ids)]
    cfg = dict(label="t", target_host="192.0.2.1", shorts_ids=ids_list, dwell=dwell,
               min_shorts_played=min_played or ids, rounds=rounds, sustained_video=sustained_video,
               sustained_dwell=900, min_sustained_s=600, host_identity=identity)
    dump(cfg, os.path.join(directory, "config.json"))
    default = [("prereq", "ok", ""), ("identity", "ok", ""), ("collect", "ok", ""), ("survey", "ok", ""), ("tunnel", "ok", "")]
    for n in range(1, rounds + 1):
        default.append(("shorts_round_%d" % n, "ok", ""))
        dump(rows if rows is not None else [prow() for _ in ids_list], os.path.join(directory, "round_%d.json" % n))
    if sustained_video:
        default.append(("sustained", "ok", ""))
        sr = [prow(played_s=900.0, still_on_shorts=False)] if sustained_rows == "auto" else sustained_rows
        dump(sr, os.path.join(directory, "sustained.json"))
    with open(os.path.join(directory, "stages.tsv"), "w") as fh:
        for st in (stages if stages is not None else default):
            fh.write("\t".join(st) + "\n")
    ok = 1.0 if tls == "good" else 0.1
    trows = [["1.1.1.1", s, "ok" if (s in ql.CONTROL_SNI or tls == "good" or i == 0) else "noreply"]
             for i, s in enumerate(list(ql.YOUTUBE_SNI) * 5 + list(ql.CONTROL_SNI) * 5)]
    if tls == "control_down":
        trows = [[ip, s, "noreply"] for ip, s, _ in trows]
    dump({"tls": {"rows": trows}}, os.path.join(directory, "survey.json"))
    dump({"egress_ipv4": "203.0.113.7"}, os.path.join(directory, "collect.json"))
    del ok


class Verdict(unittest.TestCase):
    def verdict(self, **kw):
        with tempfile.TemporaryDirectory() as d:
            write_run(d, **kw)
            return ql.build_verdict(d)

    def test_pass_requires_everything(self):
        v = self.verdict()
        self.assertEqual((v["verdict"], v["reason"]), ("PASS", "all_required_measurements_passed"))
        self.assertIn("scope", v)

    def test_near_miss_eight_of_nine_fails_by_default(self):
        rows = [prow() for _ in range(8)] + [prow(started=False, unavailable=True, played_s=0)]
        v = self.verdict(rows=rows, ids=9)
        self.assertEqual(v["verdict"], "FAIL")
        self.assertEqual(v["reason"], "media_refused")
        self.assertEqual(v["evidence"]["shorts_rounds"][0]["played"], 8)

    def test_threshold_is_configurable_and_recorded(self):
        rows = [prow() for _ in range(8)] + [prow(started=False, unavailable=True, played_s=0)]
        v = self.verdict(rows=rows, ids=9, min_played=8)
        self.assertEqual(v["verdict"], "PASS")
        self.assertEqual(v["workload"]["min_shorts_played"], 8)

    def test_every_round_must_pass(self):
        with tempfile.TemporaryDirectory() as d:
            write_run(d, rounds=2)
            dump([prow(started=False, unavailable=True, played_s=0)] * 3, os.path.join(d, "round_2.json"))
            v = ql.build_verdict(d)
        self.assertEqual((v["verdict"], v["reason"]), ("FAIL", "media_refused"))

    def test_missing_rows_are_inconclusive_not_failed(self):
        v = self.verdict(rows=[prow(), prow()], ids=3)
        self.assertEqual((v["verdict"], v["reason"]), ("INCONCLUSIVE", "incomplete_probe"))

    def test_missing_rows_that_cannot_reach_the_bar_still_fail_on_the_evidence_we_have(self):
        rows = [prow(started=False, unavailable=True, played_s=0), prow(started=False, unavailable=True, played_s=0)]
        v = self.verdict(rows=rows, ids=3)
        self.assertEqual(v["verdict"], "FAIL")

    def test_leaving_shorts_is_not_evidence_either_way(self):
        v = self.verdict(rows=[prow(still_on_shorts=False)] * 3)
        self.assertEqual((v["verdict"], v["reason"]), ("INCONCLUSIVE", "not_a_short"))

    def test_identity_unverified_caps_a_pass(self):
        v = self.verdict(identity="unverified")
        self.assertEqual((v["verdict"], v["reason"]), ("INCONCLUSIVE", "host_identity_unverified"))

    def test_sustained_is_mandatory(self):
        v = self.verdict(sustained_video="")
        self.assertEqual((v["verdict"], v["reason"]), ("INCONCLUSIVE", "sustained_not_run"))

    def test_sustained_stall_fails(self):
        v = self.verdict(sustained_rows=[prow(played_s=900.0, max_stall_s=9.0, still_on_shorts=False)])
        self.assertEqual((v["verdict"], v["reason"]), ("FAIL", "stalled"))

    def test_stage_failures_are_inconclusive_and_keep_their_code(self):
        for code in ("prereq_missing", "host_key_mismatch", "tunnel_egress_mismatch", "probe_timeout", "interrupted"):
            with self.subTest(code=code):
                with tempfile.TemporaryDirectory() as d:
                    write_run(d, stages=[("prereq", "failed", code)] if code == "prereq_missing" else
                              [("prereq", "ok", ""), ("run", "failed", code)], rounds=1, sustained_video="")
                    v = ql.build_verdict(d)
                self.assertEqual((v["verdict"], v["reason"]), ("INCONCLUSIVE", code))

    def test_tls_blocked_is_a_conclusive_fail_only_when_controls_work(self):
        self.assertEqual(self.verdict(tls="blocked")["reason"], "tls_blocked")
        self.assertEqual(self.verdict(tls="blocked")["verdict"], "FAIL")
        v = self.verdict(tls="control_down", stages=[("prereq", "ok", ""), ("survey", "ok", "")], rounds=1, sustained_video="")
        self.assertEqual((v["verdict"], v["reason"]), ("INCONCLUSIVE", "network_unhealthy"))

    def test_missing_and_unreadable_evidence_are_distinct(self):
        with tempfile.TemporaryDirectory() as d:
            write_run(d)
            os.remove(os.path.join(d, "round_1.json"))
            self.assertEqual(ql.build_verdict(d)["reason"], "evidence_missing")
            with open(os.path.join(d, "round_1.json"), "w") as fh:
                fh.write("{nope")
            self.assertEqual(ql.build_verdict(d)["reason"], "evidence_unreadable")

    def test_verdict_contains_no_secret_looking_fields(self):
        text = json.dumps(self.verdict())
        for needle in ("BEGIN", "password", "ssh_key", "token"):
            self.assertNotIn(needle, text)


class Helpers(unittest.TestCase):
    def test_merge_ips_validates_and_dedupes(self):
        with tempfile.TemporaryDirectory() as d:
            a, b, out = [os.path.join(d, n) for n in ("a.json", "b.json", "out.json")]
            json.dump(["1.1.1.1", "2.2.2.2"], open(a, "w"))
            json.dump({"ips": ["2.2.2.2", "3.3.3.3"]}, open(b, "w"))
            self.assertEqual(ql.main(["merge-ips", out, a, b]), 0)
            self.assertEqual(json.load(open(out)), ["1.1.1.1", "2.2.2.2", "3.3.3.3"])
            json.dump(["not-an-ip"], open(a, "w"))
            self.assertEqual(ql.main(["merge-ips", out, a]), 1)
            self.assertEqual(ql.main(["merge-ips", out, os.path.join(d, "missing.json")]), 1)

    def socks_server(self, ip):
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(4)

        def serve():
            while True:
                try:
                    c, _ = srv.accept()
                except OSError:
                    return
                try:
                    c.recv(16)
                    c.sendall(b"\x05\x00")
                    hdr = c.recv(4)
                    n = c.recv(1)[0]
                    c.recv(n + 2)
                    c.sendall(b"\x05\x00\x00\x01" + b"\x00" * 6)
                    c.recv(2048)
                    c.sendall(b"HTTP/1.0 200 OK\r\n\r\n" + ip.encode())
                finally:
                    c.close()
            del hdr

        threading.Thread(target=serve, daemon=True).start()
        return srv

    def test_socks_check_proves_where_the_tunnel_exits(self):
        srv = self.socks_server("203.0.113.7")
        port = srv.getsockname()[1]
        try:
            self.assertEqual(ql.main(["socks-check", "--port", str(port), "--expect-ip", "203.0.113.7", "--deadline", "5"]), 0)
            self.assertEqual(ql.main(["socks-check", "--port", str(port), "--expect-ip", "198.51.100.1", "--deadline", "5"]), 3)
        finally:
            srv.close()

    def test_socks_check_times_out_when_nothing_listens_and_stops_if_the_tunnel_died(self):
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        self.assertEqual(ql.main(["socks-check", "--port", str(port), "--expect-ip", "1.2.3.4", "--deadline", "1"]), 4)
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        self.assertEqual(ql.main(["socks-check", "--port", str(port), "--expect-ip", "1.2.3.4", "--deadline", "30", "--pid", str(dead.pid)]), 4)

    def test_cli_verdict_exit_codes(self):
        with tempfile.TemporaryDirectory() as d:
            write_run(d)
            self.assertEqual(ql.main(["verdict", "--dir", d]), 0)
            self.assertTrue(os.path.isfile(os.path.join(d, "verdict.json")))
            write_run(d, rows=[prow(started=False, unavailable=True, played_s=0)] * 3)
            self.assertEqual(ql.main(["verdict", "--dir", d]), 1)
            write_run(d, identity="unverified")
            self.assertEqual(ql.main(["verdict", "--dir", d]), 2)


if __name__ == "__main__":
    unittest.main()
