import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "ssh_auth_summary.py")
spec = importlib.util.spec_from_file_location("sas", TOOL)
sas = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sas)

# Documentation-range addresses only (RFC 5737); nothing here is a real host.
LOG = """\
2026-10-22T08:40:01+0000 h sshd[1]: Failed password for root from 198.51.100.7 port 4001 ssh2
2026-10-22T08:40:05+0000 h sshd[1]: Failed password for invalid user hunter2 from 198.51.100.7 port 4002 ssh2
2026-10-22T08:40:09+0000 h sshd[1]: Invalid user admin from 198.51.100.7 port 4003
2026-10-22T08:40:12+0000 h sshd[1]: Failed password for root from 198.51.100.7 port 4004 ssh2
2026-10-22T08:40:15+0000 h sshd[1]: Failed password for root from 198.51.100.7 port 4005 ssh2
2026-10-22T08:40:19+0000 h sshd[1]: Failed password for root from 198.51.100.7 port 4006 ssh2
2026-10-22T08:42:11+0000 h sshd[2]: Accepted password for root from 192.0.2.10 port 5001 ssh2
2026-10-22T08:44:00+0000 h sshd[3]: Accepted password for root from 192.0.2.10 port 5002 ssh2
2026-10-22T09:10:30+0000 h sshd[4]: Accepted password for root from 198.51.100.7 port 5003 ssh2
2026-10-22T13:07:00+0000 h sshd[5]: Accepted publickey for root from 203.0.113.9 port 5004 ssh2: ED25519 SHA256:AbCdEf0123456789
Oct 22 13:58:00 h sshd[6]: Accepted password for deploy from 203.0.113.9 port 5005 ssh2
"""


class Summary(unittest.TestCase):
    def test_counts_methods_and_labels(self):
        public, private = sas.analyse(LOG.splitlines())
        self.assertEqual(len(public["successful_logins"]), 5)
        self.assertEqual(public["successful_by_method"], {"password": 4, "publickey": 1})
        self.assertEqual(public["distinct_success_sources"], 3)
        self.assertEqual(public["failed_by_method"], {"password": 5})
        self.assertEqual(public["invalid_user_attempts"], 1)
        first = public["successful_logins"][0]
        self.assertEqual((first["source"], first["account"], first["method"]), ("S1", "root", "password"))
        self.assertEqual(private["labels"], {"S1": "192.0.2.10", "S2": "198.51.100.7", "S3": "203.0.113.9"})

    def test_the_public_summary_contains_no_address_username_guess_or_key(self):
        public, _ = sas.analyse(LOG.splitlines())
        text = json.dumps(public)
        for secret in ("192.0.2.10", "198.51.100.7", "203.0.113.9", "hunter2", "admin"):
            self.assertNotIn(secret, text)
        # The key *fingerprint* is the only key-derived value allowed, and it is a hash.
        self.assertEqual(public["successful_logins"][3]["key_fingerprint"], "SHA256:AbCdEf0123456789")

    def test_flags(self):
        public, _ = sas.analyse(LOG.splitlines(), expected={"192.0.2.10", "203.0.113.9"})
        names = [(f["flag"], f["source"]) for f in public["flags"]]
        self.assertIn(("ROOT_PASSWORD_SUCCESS", "S1"), names)
        # S2 failed 6 times (5 failed-password + 1 invalid-user counted) then succeeded.
        self.assertIn(("SUCCESS_AFTER_FAILURES", "S2"), names)
        self.assertIn(("UNEXPECTED_SOURCE", "S2"), names)
        self.assertNotIn(("UNEXPECTED_SOURCE", "S1"), names)
        self.assertNotIn(("ROOT_PASSWORD_SUCCESS", "S3"), names, "publickey / non-root must not trip it")

    def test_no_expected_list_means_no_unexpected_flag(self):
        public, _ = sas.analyse(LOG.splitlines())
        self.assertFalse([f for f in public["flags"] if f["flag"] == "UNEXPECTED_SOURCE"])

    def test_empty_and_garbage_input(self):
        public, private = sas.analyse(["", "garbage", "sshd: something else"])
        self.assertEqual(public["successful_logins"], [])
        self.assertEqual(private["labels"], {})

    def test_private_output_is_0600_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "log")
            open(log, "w").write(LOG)
            out = os.path.join(d, "private.json")
            run = lambda: subprocess.run(
                [sys.executable, "-I", TOOL, "--input", log, "--private-out", out],
                capture_output=True, text=True)
            first = run()
            self.assertEqual(first.returncode, 0, first.stderr)
            self.assertEqual(stat.S_IMODE(os.stat(out).st_mode), 0o600)
            self.assertIn("192.0.2.10", open(out).read())
            self.assertNotIn("192.0.2.10", first.stdout)
            self.assertEqual(run().returncode, 2, "an existing private file must not be overwritten")


if __name__ == "__main__":
    unittest.main()
