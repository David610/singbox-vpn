import copy, importlib.util, os, unittest
spec = importlib.util.spec_from_file_location("rc", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "reboot_check.py"))
rc = importlib.util.module_from_spec(spec); spec.loader.exec_module(rc)

PRE = {"boot_id": "a", "uptime_s": 5000.0, "files": {"/etc/vpn/deployment.toml": "x1", "/root/.ssh/authorized_keys": "k1"},
       "units_active": {"sing-box": "active", "nginx": "active", "chrony": "inactive"}, "timers_enabled": {"certbot.timer": "enabled"},
       "listeners": ["tcp 0.0.0.0:443 sing-box"], "ufw": ["Status: active", "443/tcp ALLOW"],
       "nft_isolation": {"present": True, "uid_scoped": True, "sha": "n1"}, "sshd": {"passwordauthentication": "no"},
       "sysctl": {"net.core.default_qdisc": "fq"}, "routing_fingerprint": "r1",
       "ipv6": {"has_global": True, "default_route": True, "gateway_neighbour": "REACHABLE"}}

def post(**kw):
    p = copy.deepcopy(PRE); p["boot_id"] = "b"; p["uptime_s"] = 60.0
    for k, v in kw.items(): p[k] = v
    return p

def verdict(p): return {r[1]: r[0] for r in rc.compare(PRE, p)}

class RebootCheck(unittest.TestCase):
    def test_clean_reboot_passes(self):
        self.assertNotIn("FAIL", verdict(post()).values())
    def test_no_reboot_is_a_failure(self):
        p = post(); p["boot_id"] = "a"; p["uptime_s"] = 6000.0
        self.assertEqual(verdict(p)["the host really rebooted (boot_id changed)"], "FAIL")
    def test_service_not_back_fails(self):
        p = post(); p["units_active"]["sing-box"] = "failed"
        self.assertEqual(verdict(p)["service sing-box active again"], "FAIL")
    def test_inactive_before_is_not_required_after(self):
        self.assertNotIn("service chrony active again", verdict(post()))
    def test_routing_drift_fails(self):
        self.assertEqual(verdict(post(routing_fingerprint="r2"))["routing_fingerprint unchanged across the reboot"], "FAIL")
    def test_firewall_change_fails(self):
        self.assertEqual(verdict(post(ufw=["Status: inactive"]))["ufw unchanged across the reboot"], "FAIL")
    def test_ssh_password_auth_back_on_fails(self):
        self.assertEqual(verdict(post(sshd={"passwordauthentication": "yes"}))["sshd unchanged across the reboot"], "FAIL")
    def test_ipv6_neighbour_failed_after_boot_fails(self):
        p = post(); p["ipv6"] = {"has_global": True, "default_route": True, "gateway_neighbour": "FAILED"}
        v = verdict(p)
        self.assertEqual(v["IPv6 gateway neighbour discovery works after boot (I-54 guard)"], "FAIL")
    def test_negated_uid_isolation_table_fails(self):
        p = post(); p["nft_isolation"] = {"present": True, "uid_scoped": False, "sha": "n1"}
        self.assertEqual(verdict(p)["egress-isolation table is uid-scoped (no negated uid match)"], "FAIL")
    def test_no_ipv6_host_is_not_judged(self):
        p = post(); p["ipv6"] = {"has_global": False, "default_route": False, "gateway_neighbour": None}
        self.assertNotIn("IPv6 gateway neighbour discovery works after boot (I-54 guard)", verdict(p))

if __name__ == "__main__": unittest.main()
