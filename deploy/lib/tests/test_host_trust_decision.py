import copy
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "host_trust_decision.py")
spec = importlib.util.spec_from_file_location("htd", TOOL)
htd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(htd)

GOOD = {
    "schema": 1, "host_label": "fin1", "decided_on": "2026-10-23", "decided_by": "owner",
    "decision": "TRUSTED", "provider_snapshot_taken": True,
    "evidence_private_location": "owner password-manager item 'fin1 auth evidence'",
    "sessions": [{"id": f"L{i}", "classification": "OWNER", "basis": "my phone, mobile network"} for i in range(1, 6)],
    "notes": "",
}


class Decision(unittest.TestCase):
    def rec(self, **over):
        r = copy.deepcopy(GOOD)
        r.update(over)
        return r

    def test_a_consistent_trusted_record_is_valid(self):
        self.assertEqual(htd.validate(GOOD), [])

    def test_trusted_with_an_unknown_or_foreign_session_is_inconsistent(self):
        for cls in ("UNKNOWN", "NOT_OWNER"):
            r = self.rec()
            r["sessions"][2]["classification"] = cls
            self.assertTrue(any("inconsistent" in e for e in htd.validate(r)), cls)

    def test_trusted_needs_a_provider_snapshot_and_listed_sessions(self):
        self.assertTrue(htd.validate(self.rec(provider_snapshot_taken=False)))
        self.assertTrue(htd.validate(self.rec(sessions=[])))

    def test_untrusted_and_undetermined_are_valid_and_distinct(self):
        r = self.rec(decision="UNTRUSTED")
        r["sessions"][0]["classification"] = "UNKNOWN"
        self.assertEqual(htd.validate(r), [])
        self.assertEqual(htd.validate(self.rec(decision="UNDETERMINED", sessions=[])), [])
        self.assertTrue(htd.validate(self.rec(decision="UNDETERMINED")), "all-OWNER cannot stay undetermined silently")

    def test_addresses_and_key_material_are_rejected(self):
        for bad in ("logged in from 192.0.2.10", "from 2001:db8::1", "-----BEGIN OPENSSH PRIVATE KEY-----",
                    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexample", "A" * 48):
            self.assertTrue(htd.validate(self.rec(notes=bad)), bad)

    def test_times_are_not_mistaken_for_addresses(self):
        self.assertEqual(htd.validate(self.rec(notes="logins at 08:42:11 and 13:07")), [])
        for ipv6 in ("::1", "fe80::1", "2001:db8:0:0:0:0:0:1", "2001:db8::"):
            self.assertTrue(htd.validate(self.rec(notes=f"src {ipv6}")), ipv6)

    def test_missing_and_malformed_fields(self):
        r = copy.deepcopy(GOOD)
        del r["decided_by"]
        self.assertTrue(htd.validate(r))
        self.assertTrue(htd.validate(self.rec(decision="MAYBE")))
        self.assertTrue(htd.validate(self.rec(decided_on="yesterday")))
        self.assertTrue(htd.validate([]))

    def test_exit_codes(self):
        def run(doc):
            with tempfile.TemporaryDirectory() as d:
                path = os.path.join(d, "r.json")
                json.dump(doc, open(path, "w"))
                return subprocess.run([sys.executable, "-I", TOOL, path], capture_output=True, text=True).returncode
        self.assertEqual(run(GOOD), 0)
        untrusted = self.rec(decision="UNTRUSTED")
        untrusted["sessions"][0]["classification"] = "UNKNOWN"
        self.assertEqual(run(untrusted), 3)
        self.assertEqual(run(self.rec(decision="UNDETERMINED", sessions=[])), 4)
        self.assertEqual(run(self.rec(decision="TRUSTED", provider_snapshot_taken=False)), 1)
        self.assertEqual(subprocess.run([sys.executable, "-I", TOOL, "/nonexistent"], capture_output=True).returncode, 2)


if __name__ == "__main__":
    unittest.main()
