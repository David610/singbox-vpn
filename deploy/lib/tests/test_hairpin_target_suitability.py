import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("hts", os.path.join(HERE, "..", "hairpin_target_suitability.py"))
hts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hts)
TOOL = os.path.join(HERE, "..", "hairpin_target_suitability.py")


class Safety(unittest.TestCase):
    def test_every_shipped_remote_command_is_read_only(self):
        self.assertEqual(hts.commands_are_read_only(), [])

    def test_the_deny_list_would_catch_mutating_commands(self):
        for bad in ("systemctl restart sing-box", "rm -rf /etc/vpn", "vpn-admin user create --name x", "echo x > /etc/passwd",
                    "cat /etc/vpn/users.json | sh", "vpn-admin google-egress-hairpin set --x", "apt-get install x", "reboot",
                    "curl -X POST https://x", "sed -i s/a/b/ f", "tee /etc/x", "systemctl enable foo", "kill -9 1", "iptables -F"):
            self.assertTrue(hts.DENY.search(bad), bad)
        for good in ("systemctl is-active x", "systemctl show sing-box -p NRestarts", "ip -br addr", "vpn-admin doctor 2>&1 | grep FAIL",
                     "curl -4 -s -m 10 https://ifconfig.me"):
            self.assertFalse(hts.DENY.search(good), good)

    def test_nothing_runs_without_owner_authorization(self):
        for sub in ("inspect", "probe"):
            p = subprocess.run([sys.executable, TOOL, sub, "--target", "root@192.0.2.1", "--ssh-bin", "/bin/false"], capture_output=True, text=True)
            self.assertNotEqual(p.returncode, 0); self.assertIn("owner must authorize", p.stderr)

    def test_plan_executes_nothing_and_prints_the_exact_commands(self):
        calls = []
        orig = subprocess.run; subprocess.run = lambda *a, **k: calls.append(a) or orig(*a, **k)
        try:
            buf = io.StringIO()
            with redirect_stdout(buf):
                hts.main(["plan", "--target", "root@192.0.2.1"])
        finally:
            subprocess.run = orig
        self.assertEqual(calls, [])
        self.assertIn("-N -D 127.0.0.1:1090", buf.getvalue()); self.assertIn("StrictHostKeyChecking=yes", buf.getvalue())

    def test_target_syntax_is_validated(self):
        for bad in ("host", "root@host; rm -rf /", "-oProxyCommand=x@y", "root@h ost"):
            with self.assertRaises(SystemExit): hts.check_target(bad)

    def test_ssh_is_batch_strict_and_forwards_nothing(self):
        joined = " ".join(hts.SSH_BASE)
        for need in ("BatchMode=yes", "StrictHostKeyChecking=yes", "ForwardAgent=no", "ClearAllForwardings=yes"):
            self.assertIn(need, joined)

    def test_secrets_are_redacted(self):
        s = hts.redact("id 123e4567-e89b-12d3-a456-426614174000 key AbCdEfGhIjKlMnOpQrStUvWxYz0123456789abcdefgh ok")
        self.assertNotIn("123e4567", s); self.assertNotIn("AbCdEf", s); self.assertIn("ok", s)


class Verdicts(unittest.TestCase):
    def base(self):
        mk = lambda out: {"rc": 0, "stdout": out}
        return {"commands": {"addresses": mk("eth0"), "release": mk("{}"), "restart_hazards": mk("disabled\ninactive\nNRestarts=0"),
                             "health": mk("FAILS=0"), "egress_v4": mk("203.0.113.9\n"), "ssh_posture": mk("passwordauthentication no")}}

    def test_inspect_ok_unsuitable_inconclusive(self):
        r = self.base(); self.assertEqual(hts.verdict_inspect(r)[0], "INSPECT_OK")
        r = self.base(); r["commands"]["restart_hazards"]["stdout"] = "enabled\nactive\nNRestarts=0"; self.assertEqual(hts.verdict_inspect(r)[0], "UNSUITABLE")
        r = self.base(); r["commands"]["restart_hazards"]["stdout"] = "disabled\ninactive\nNRestarts=40"; self.assertEqual(hts.verdict_inspect(r)[0], "UNSUITABLE")
        r = self.base(); r["commands"]["health"]["stdout"] = "[FAIL] x"; self.assertEqual(hts.verdict_inspect(r)[0], "UNSUITABLE")
        r = self.base(); r["commands"]["egress_v4"]["stdout"] = ""; self.assertEqual(hts.verdict_inspect(r)[0], "UNSUITABLE")
        r = self.base(); del r["commands"]["health"]; self.assertEqual(hts.verdict_inspect(r)[0], "INCONCLUSIVE")

    def test_password_ssh_is_a_warning_not_a_blocker(self):
        r = self.base(); r["commands"]["ssh_posture"]["stdout"] = "passwordauthentication yes"
        v, why = hts.verdict_inspect(r); self.assertEqual(v, "INSPECT_OK"); self.assertTrue(any("password" in w for w in why))

    def test_probe_needs_the_control_to_fail_in_the_same_run(self):
        good = {"genuine": 12, "played": 12, "video_total": 2, "video_played": 2}
        refused = {"genuine": 12, "played": 0}
        self.assertEqual(hts.verdict_probe(good, refused)[0], "RUN_PASS")
        self.assertEqual(hts.verdict_probe(good, {"genuine": 12, "played": 12})[0], "INCONCLUSIVE")      # probe cannot fail: not trusted
        self.assertEqual(hts.verdict_probe(good, {"genuine": 1, "played": 0})[0], "INCONCLUSIVE")
        self.assertEqual(hts.verdict_probe({"genuine": 12, "played": 11}, refused)[0], "UNSUITABLE")      # 11/12 is not 12/12
        self.assertEqual(hts.verdict_probe({"genuine": 4, "played": 4}, refused)[0], "INCONCLUSIVE")     # too few genuine Shorts

    def test_a_normal_video_pass_never_substitutes_for_shorts(self):
        t = {"genuine": 12, "played": 0, "video_total": 2, "video_played": 2}
        self.assertEqual(hts.verdict_probe(t, {"genuine": 12, "played": 0})[0], "UNSUITABLE")

    def test_aggregate_requires_three_runs_an_hour_apart(self):
        r = lambda t, v="RUN_PASS": {"kind": "probe", "epoch": t, "verdict": v}
        self.assertEqual(hts.aggregate([r(0), r(4000)])[0], "INCONCLUSIVE")
        self.assertEqual(hts.aggregate([r(0), r(100), r(4000)])[0], "INCONCLUSIVE")
        self.assertEqual(hts.aggregate([r(0), r(4000), r(8000, "UNSUITABLE")])[0], "UNSUITABLE")
        v, why = hts.aggregate([r(0), r(4000), r(8000)])
        self.assertEqual(v, "C1_C2_SATISFIED"); self.assertTrue(any("UNVERIFIED" in w for w in why))   # never claims more than it covered


class EndToEndInspect(unittest.TestCase):
    def test_inspect_with_a_fake_ssh_records_only_what_it_was_given(self):
        with tempfile.TemporaryDirectory() as d:
            ssh = os.path.join(d, "ssh")
            log = os.path.join(d, "ssh.log")
            with open(ssh, "w") as f:
                f.write('#!/bin/sh\necho "$@" >> %s\ncase "$*" in\n*NRestarts*) printf "disabled\\ninactive\\nNRestarts=0\\n";;\n*"doctor"*) echo "FAILS=0";;\n'
                        '*ifconfig.me*"-4"*|*"curl -4"*) echo 203.0.113.9;;\n*) echo ok;;\nesac\n' % log)
            os.chmod(ssh, 0o755)
            out = os.path.join(d, "r.json")
            p = subprocess.run([sys.executable, TOOL, "inspect", "--target", "root@192.0.2.1", "--authorized-by-owner", "--ssh-bin", ssh, "--no-latency", "--out", out],
                               capture_output=True, text=True)
            rep = json.load(open(out))
            self.assertEqual(rep["verdict"], "INSPECT_OK", p.stdout + p.stderr)
            sent = open(log).read()
            self.assertEqual(sent.count("root@192.0.2.1"), len(hts.READ_ONLY_COMMANDS))       # exactly one ssh per whitelisted command
            self.assertNotIn("restart", sent.replace("NRestarts", "").replace("restart_hazards", ""))
            self.assertIn("BatchMode=yes", sent)
            self.assertIn("not_covered", rep)


if __name__ == "__main__":
    unittest.main()
