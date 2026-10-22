import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "check_deployment_contract.py")
spec = importlib.util.spec_from_file_location("cdc", TOOL)
cdc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cdc)

DEP = {"node_id": "fin1", "role": "exit", "public_host": "h.example", "subscription_host": "h.example",
       "public_ipv4": "203.0.113.1", "subscription": {"public_port": 8443}, "egress": {"ipv6_literal": "reject_google"}}
SB = {"inbounds": [{"type": "vless"}, {"type": "hysteria2"}], "log": {"level": "fatal"},
      "route": {"final": "direct", "rules": [{"action": "route", "auth_user": ["u1"], "outbound": "direct"},
                                              {"action": "sniff"}, {"action": "reject", "ip_version": 6}]}}
C = {"schema_version": 1, "target": {"public_host": "h.example", "public_ipv4": "203.0.113.1", "role": "exit"},
     "protocols": {"reality": True, "hysteria2": True}, "subscription": {"expected_public_port": 8443},
     "security": {"ssh_password_authentication": False, "singbox_log_level": "fatal"}}


def rows(contract, **kw):
    obs = cdc.observe(kw.get("dep", DEP), kw.get("sb", SB), kw.get("state", {"singbox_vpn_version": "v1"}),
                      kw.get("sshd", {"passwordauthentication": "no", "permitrootlogin": "prohibit-password"}), kw.get("aaaa", False))
    return {k: s for s, k, _ in cdc.compare(contract, obs)}


class Contract(unittest.TestCase):
    def test_matching_node_passes_everything(self):
        self.assertEqual(set(rows(C).values()), {"PASS"})

    def test_password_ssh_is_a_visible_failure(self):
        r = rows(C, sshd={"passwordauthentication": "yes", "permitrootlogin": "yes"})
        self.assertEqual(r["security.ssh_password_authentication"], "FAIL")

    def test_hostname_drift_fails(self):
        d = dict(DEP, public_host="old.example")
        self.assertEqual(rows(C, dep=d)["target.public_host"], "FAIL")

    def test_wrong_subscription_port_fails(self):
        d = dict(DEP, subscription={"public_port": 9443})
        self.assertEqual(rows(C, dep=d)["subscription.expected_public_port"], "FAIL")

    def test_missing_protocol_inbound_fails(self):
        sb = dict(SB, inbounds=[{"type": "vless"}])
        self.assertEqual(rows(C, sb=sb)["protocols.hysteria2"], "FAIL")

    def test_undecided_values_are_reported_not_passed(self):
        c = json.loads(json.dumps(C)); c["youtube"] = {"egress_mode": "UNDECIDED"}
        self.assertEqual(rows(c)["youtube.egress_mode"], "UNDECIDED")

    def test_unreadable_host_state_is_unobservable_never_pass(self):
        obs = cdc.observe(None, None, None, None, None)
        self.assertEqual({s for s, _, _ in cdc.compare(C, obs)}, {"UNOBSERVABLE"})

    def test_routing_fingerprint_ignores_users_but_catches_rule_changes(self):
        base = cdc.routing_fingerprint(SB)
        more_users = json.loads(json.dumps(SB)); more_users["route"]["rules"].insert(0, {"action": "route", "auth_user": ["u2"], "outbound": "direct"})
        self.assertEqual(cdc.routing_fingerprint(more_users), base)
        lost = json.loads(json.dumps(SB)); lost["route"]["rules"] = [r for r in lost["route"]["rules"] if r.get("action") != "sniff"]
        self.assertNotEqual(cdc.routing_fingerprint(lost), base)
        reordered = json.loads(json.dumps(SB)); reordered["route"]["rules"] = [r for r in SB["route"]["rules"] if "auth_user" not in r][::-1]
        self.assertNotEqual(cdc.routing_fingerprint(reordered), base)
        final = json.loads(json.dumps(SB)); final["route"]["final"] = "select"
        self.assertNotEqual(cdc.routing_fingerprint(final), base)

    def test_egress_mode_detection(self):
        self.assertEqual(cdc.egress_mode({}), "plain")
        self.assertEqual(cdc.egress_mode({"egress": {"ipv6_literal": "reject_google"}}), "reject_google")
        self.assertEqual(cdc.egress_mode({"google_egress_hairpin": {"relay_host": "x"}}), "hairpin")

    def test_ipv4_mapped_answers_are_not_an_aaaa_record(self):
        mapped = "::ffff:203.0.113.1  STREAM h.example\n::ffff:203.0.113.1  DGRAM\n"
        self.assertFalse(cdc.has_real_aaaa(mapped))
        self.assertTrue(cdc.has_real_aaaa("2001:db8::1  STREAM h.example\n"))
        self.assertFalse(cdc.has_real_aaaa(""))

    def test_policy_only_keys_are_not_compared(self):
        c = dict(C, youtube={"required": True})
        self.assertNotIn("youtube.required", rows(c))

    def test_cli_exit_codes_and_strict_mode(self):
        with tempfile.TemporaryDirectory() as t:
            c = os.path.join(t, "c.json"); json.dump({"schema_version": 1, "youtube": {"egress_mode": "UNDECIDED"}}, open(c, "w"))
            def run(*a):
                return subprocess.run([sys.executable, "-I", TOOL, "--contract", c, "--deployment-toml", "/nonexistent",
                                       "--singbox-config", "/nonexistent", "--install-state", "/nonexistent", *a], capture_output=True, text=True).returncode
            self.assertEqual(run(), 0)             # UNDECIDED alone is reported, not a failure
            self.assertEqual(run("--strict"), 1)   # but a strict gate demands a decided contract


if __name__ == "__main__":
    unittest.main()
