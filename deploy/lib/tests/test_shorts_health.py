import importlib.util
import json
import os
import random
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("sh", os.path.join(HERE, "..", "monitoring", "shorts_health.py"))
sh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sh)


def row(play=True, short=True, **kw):
    r = {"kind": "short", "counts_as_short": short, "play": play, "playabilityStatus": "OK", "media": {"responses": 3 if play else 0}}
    r.update(kw)
    return r


def probe(*rows, videos=()):
    return {"results": list(rows) + [{"kind": "video", "play": v} for v in videos]}


class Classify(unittest.TestCase):
    def test_classes(self):
        c = sh.classify_row
        self.assertEqual(c(row()), "PLAY")
        self.assertEqual(c(row(short=False, play=True)), "NOT_SHORT")           # a normal video redirected away is never a pass
        self.assertEqual(c(row(play=False, bot_wall=True)), "BOT_CHECK")
        self.assertEqual(c(row(play=False)), "MEDIA_WITHHELD")                  # page fine, zero media responses
        self.assertEqual(c(row(play=False, playabilityStatus="UNPLAYABLE")), "REFUSED")
        self.assertEqual(c(row(play=False, media={"responses": 5})), "STALLED")
        self.assertEqual(c(row(play=False, error="TimeoutError")), "PROBE_TIMEOUT")
        self.assertEqual(c(row(play=False, error="Error")), "PROBE_ERROR")


class Evaluate(unittest.TestCase):
    def ev(self, prev, p, **kw): return sh.evaluate(prev, p, **kw)

    def test_all_genuine_playing_is_ok_and_resets(self):
        e = self.ev(3, probe(row(), row(), row()))
        self.assertEqual((e["status"], e["streak"], e["cause"]), ("OK", 0, None))

    def test_normal_videos_never_turn_failing_shorts_into_ok(self):
        e = self.ev(0, probe(row(False), row(False), row(False), videos=(True, True)))
        self.assertNotEqual(e["status"], "OK")

    def test_non_shorts_do_not_count_toward_the_minimum(self):
        e = self.ev(0, probe(row(), row(), row(short=False)))
        self.assertEqual((e["status"], e["cause"]), ("UNVERIFIED", "HARNESS"))   # 2 genuine < 3: unverified, not OK, not a refusal

    def test_withheld_media_on_a_working_path_is_an_egress_refusal(self):
        e = self.ev(0, probe(row(False), row(False), row(False)))
        self.assertEqual((e["status"], e["cause"]), ("SUSPECT", "EGRESS_REFUSED"))

    def test_escalation_hysteresis(self):
        bad = probe(row(False), row(False), row(False))
        self.assertEqual(self.ev(0, bad)["status"], "SUSPECT")
        self.assertEqual(self.ev(1, bad)["status"], "DEGRADED")
        self.assertEqual(self.ev(2, bad)["status"], "DEGRADED")
        self.assertEqual(self.ev(3, bad)["status"], "CRITICAL")

    def test_a_dead_path_is_not_blamed_on_youtube(self):
        e = self.ev(0, None, path_ok=False, path_reason="control(s) failed: neutral=TIMEOUT")
        self.assertEqual((e["cause"], e["status"]), ("PATH_DOWN", "PATH_DOWN"))
        self.assertNotEqual(e["cause"], "EGRESS_REFUSED")

    def test_missing_probe_is_harness_not_egress(self):
        e = self.ev(0, None)
        self.assertEqual((e["status"], e["cause"]), ("UNVERIFIED", "HARNESS"))

    def test_probe_errors_alone_are_harness(self):
        e = self.ev(0, probe(row(False, error="TimeoutError"), row(False, error="TimeoutError"), row(False, error="TimeoutError")))
        self.assertEqual(e["cause"], "HARNESS")

    def test_stalled_playback_is_its_own_cause(self):
        e = self.ev(0, probe(*[row(False, media={"responses": 4}) for _ in range(3)]))
        self.assertEqual(e["cause"], "PLAYBACK_STALLED")

    def test_unverified_is_never_ok_and_counts_as_a_miss(self):
        self.assertEqual(self.ev(1, None)["streak"], 2)


class Path(unittest.TestCase):
    def test_path_ok_only_if_every_control_works(self):
        self.assertTrue(sh.classify_path({"google": {"ok": True}, "neutral": {"ok": True}})[0])
        ok, why = sh.classify_path({"google": {"ok": True}, "neutral": {"ok": False, "class": "TIMEOUT"}})
        self.assertFalse(ok); self.assertIn("neutral=TIMEOUT", why)

    def test_curl_failures_are_classified(self):
        def mk(rc, out=""):
            return lambda *a, **k: subprocess.CompletedProcess(a[0], rc, out, "")
        self.assertEqual(sh.path_controls(None, {"x": "https://u"}, mk(0, "204"))["x"]["ok"], True)
        self.assertEqual(sh.path_controls(None, {"x": "https://u"}, mk(6))["x"]["class"], "DNS")
        self.assertEqual(sh.path_controls(None, {"x": "https://u"}, mk(28))["x"]["class"], "TIMEOUT")
        self.assertEqual(sh.path_controls(None, {"x": "https://u"}, mk(0, "403"))["x"]["ok"], False)

    def test_socks_proxy_uses_remote_dns(self):
        seen = {}
        def run(cmd, **k): seen["cmd"] = cmd; return subprocess.CompletedProcess(cmd, 0, "204", "")
        sh.path_controls("socks5://127.0.0.1:1", {"x": "https://u"}, run)
        self.assertIn("socks5h://127.0.0.1:1", seen["cmd"])        # DNS must resolve through the tunnel, not locally (leak)


class Alerts(unittest.TestCase):
    def step(self, st, status, streak, cause="EGRESS_REFUSED", now=0.0):
        return sh.next_state(st, {"status": status, "streak": streak, "cause": cause, "reason": "r"}, now)

    def test_no_alert_for_the_first_miss(self):
        _, ev = self.step({}, "SUSPECT", 1)
        self.assertEqual(ev, [])

    def test_alert_once_on_entering_degraded_then_critical_then_not_again(self):
        st, ev = self.step({}, "DEGRADED", 2, now=10)
        self.assertEqual([e["event"] for e in ev], ["DEGRADED"])
        st, ev = self.step(st, "DEGRADED", 3, now=20)
        self.assertEqual(ev, [])
        st, ev = self.step(st, "CRITICAL", 4, now=30)
        self.assertEqual([e["event"] for e in ev], ["CRITICAL"])
        st, ev = self.step(st, "CRITICAL", 5, now=40)
        self.assertEqual(ev, [])

    def test_reminder_after_the_cooldown(self):
        st, _ = self.step({}, "CRITICAL", 4, now=0)
        st, ev = self.step(st, "CRITICAL", 5, now=sh.ALERT_COOLDOWN_S + 1)
        self.assertEqual([e["event"] for e in ev], ["CRITICAL"])

    def test_recovery_needs_consecutive_oks_and_does_not_flap(self):
        st, _ = self.step({}, "CRITICAL", 4, now=0)
        st, ev = self.step(st, "OK", 0, None, now=1)
        self.assertEqual(ev, [])                                    # one OK is not recovery
        st, ev = self.step(st, "SUSPECT", 1, now=2)                 # a miss resets the OK run
        st, ev = self.step(st, "OK", 0, None, now=3)
        self.assertEqual(ev, [])
        st, ev = self.step(st, "OK", 0, None, now=4)
        self.assertEqual([e["event"] for e in ev], ["RECOVERED"])
        st, ev = self.step(st, "OK", 0, None, now=5)
        self.assertEqual(ev, [])                                    # recovered once

    def test_alert_command_failure_is_reported(self):
        ok = lambda *a, **k: subprocess.CompletedProcess(a[0], 0, "", "")
        bad = lambda *a, **k: subprocess.CompletedProcess(a[0], 1, "", "")
        self.assertIsNone(sh.send_alert("true", {"event": "X"}, ok))
        self.assertIn("exited 1", sh.send_alert("false", {"event": "X"}, bad))
        self.assertIsNone(sh.send_alert(None, {"event": "X"}))


class Discovery(unittest.TestCase):
    def test_only_genuine_shorts_paths_yield_an_id(self):
        spec2 = importlib.util.spec_from_file_location("yep", os.path.join(HERE, "..", "youtube_evidence_probe.py"))
        yep = importlib.util.module_from_spec(spec2); spec2.loader.exec_module(yep)
        self.assertEqual(yep.extract_short_id("/shorts/sAcElROnYIE"), "sAcElROnYIE")
        for bad in ("/watch", "/shorts/", "/shorts/short", "/shorts/sAcElROnYIE/x", "/watch?v=dQw4w9WgXcQ", "", None, "/shorts/../../etc/pwd"):
            self.assertIsNone(yep.extract_short_id(bad), bad)


class ProbeContract(unittest.TestCase):
    def test_every_page_state_key_the_probe_reads_is_produced_by_the_page_script(self):
        # The probe once crashed with KeyError('path') on every run because one() read b["path"] that STATE_JS never returned,
        # so the shipped monitor could not have detected anything. Browser-free contract check.
        import re
        src = open(os.path.join(HERE, "..", "youtube_evidence_probe.py")).read()
        js = re.search(r'STATE_JS = """(.*?)"""', src, re.S).group(1)
        produced = set(re.findall(r"(?:^|[{,]\s*)(\w+):", js[js.index("return {"):]))
        read = set(re.findall(r'\b[ab]\["(\w+)"\]', src))
        self.assertTrue(read, "no page-state reads found: the regex is stale")
        self.assertEqual(read - produced, set(), "keys read by one() but never returned by STATE_JS")


class Corpus(unittest.TestCase):
    def test_validation_and_rotation(self):
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write("# comment\nsAcElROnYIE\nnot-an-id\n../../etc/pw\nNlHemkN8xq4 # trailing\n\n")
        try:
            ids = sh.load_corpus(f.name)
        finally:
            os.unlink(f.name)
        self.assertEqual(ids, ["sAcElROnYIE", "NlHemkN8xq4"])       # malformed lines never reach a command line
        self.assertEqual(sh.load_corpus("/nonexistent"), sh.DEFAULT_SHORTS)
        corpus = [f"{i:011d}" for i in range(40)]
        a, b = sh.pick(corpus, 4, random.Random(1)), sh.pick(corpus, 4, random.Random(2))
        self.assertEqual(len(a), 4); self.assertNotEqual(a, b)


class EndToEnd(unittest.TestCase):
    """Whole main() with a fake probe and a fake curl on PATH; no network, no browser."""
    def run_main(self, d, probe_rows, curl_rc=0, alert_rc=0, extra=()):
        pj = os.path.join(d, "fake_probe.py")
        with open(pj, "w") as f: f.write("import json,sys\nout=sys.argv[sys.argv.index('--out')+1]\njson.dump(%r,open(out,'w'))\n" % (probe_rows,))
        bindir = os.path.join(d, "bin"); os.makedirs(bindir, exist_ok=True)
        c = os.path.join(bindir, "curl")
        with open(c, "w") as f: f.write("#!/bin/sh\n[ %d -ne 0 ] && exit %d\necho -n 204\n" % (curl_rc, curl_rc))
        os.chmod(c, 0o755)
        env = dict(os.environ, PATH=bindir + ":" + os.environ["PATH"])
        alert = os.path.join(d, "alert.sh")
        with open(alert, "w") as f: f.write("#!/bin/sh\ncat >> %s/alert-received.jsonl\nexit %d\n" % (d, alert_rc))
        os.chmod(alert, 0o755)
        p = subprocess.run([sys.executable, os.path.join(HERE, "..", "monitoring", "shorts_health.py"), "--label", "t", "--state-dir", os.path.join(d, "st"),
                            "--probe", pj, "--python", sys.executable, "--alert-cmd", alert, "--force", *extra],
                           capture_output=True, text=True, env=env)
        return p.returncode, json.loads(p.stdout.strip().splitlines()[-1])

    def test_refused_egress_escalates_alerts_once_then_recovers(self):
        bad = probe(row(False), row(False), row(False))["results"]
        good = probe(row(), row(), row())["results"]
        with tempfile.TemporaryDirectory() as d:
            seq = [("bad", 1), ("bad", 1), ("bad", 1), ("bad", 2), ("good", 0), ("good", 0)]
            alerts = []
            for kind, want_rc in seq:
                rc, line = self.run_main(d, {"results": bad if kind == "bad" else good})
                alerts.append(line["alerts"])
            self.assertEqual(alerts, [[], ["DEGRADED"], [], ["CRITICAL"], [], ["RECOVERED"]])
            self.assertEqual(line["status"], "OK")
            self.assertEqual(sum(1 for _ in open(os.path.join(d, "alert-received.jsonl"))), 3)
            self.assertNotIn("token", open(os.path.join(d, "st", "shorts-health.jsonl")).read().lower())

    def test_dead_path_skips_the_browser_and_is_not_an_egress_verdict(self):
        with tempfile.TemporaryDirectory() as d:
            rc, line = self.run_main(d, {"results": []}, curl_rc=28)
            self.assertEqual((line["cause"], line["status"]), ("PATH_DOWN", "PATH_DOWN"))
            self.assertEqual(line["summary"]["genuine"], 0)       # the probe was never run through a dead path
            self.assertEqual(rc, 3)

    def test_undeliverable_alert_is_loud(self):
        bad = probe(row(False), row(False), row(False))["results"]
        with tempfile.TemporaryDirectory() as d:
            self.run_main(d, {"results": bad})
            rc, line = self.run_main(d, {"results": bad}, alert_rc=1)   # second miss -> DEGRADED -> alert -> command fails
            self.assertEqual(rc, 4); self.assertTrue(line["alert_errors"])

    def test_rate_limit_and_lock(self):
        with tempfile.TemporaryDirectory() as d:
            self.run_main(d, {"results": probe(row(), row(), row())["results"]})
            p = subprocess.run([sys.executable, os.path.join(HERE, "..", "monitoring", "shorts_health.py"), "--label", "t", "--state-dir", os.path.join(d, "st")],
                               capture_output=True, text=True)
            self.assertEqual(json.loads(p.stdout)["status"], "SKIPPED")


if __name__ == "__main__":
    unittest.main()
