import copy
import http.server
import importlib.util
import json
import os
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("hca", os.path.join(HERE, "..", "hiddify_core_acceptance.py"))
hca = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hca)

RELAY = "relay.example.test"
EXIT = "exit.example.test"
HOP = "Reality §hide§"
VIA = "exit · via relay"


def good_profile():
    """The shape `?format=singbox&compat=hiddify-pinned` serves for a relay user (credentials omitted)."""
    return {
        "outbounds": [
            {"type": "vless", "tag": HOP, "server": RELAY, "server_port": 443, "flow": "xtls-rprx-vision"},
            {"type": "vless", "tag": VIA, "server": EXIT, "server_port": 443, "flow": "xtls-rprx-vision", "detour": HOP},
            {"type": "urltest", "tag": "auto", "outbounds": [VIA], "url": "https://www.gstatic.com/generate_204"},
            {"type": "selector", "tag": "select", "outbounds": [VIA, "auto"], "default": VIA},
            {"type": "direct", "tag": "direct"},
        ],
        "route": {"final": "select"},
    }


def problems(profile, relay_host=RELAY):
    return hca.analyze_relay_profile(profile, relay_host)[0]


class RelayProfileAnalysis(unittest.TestCase):
    def test_the_served_shape_is_accepted(self):
        found, facts = hca.analyze_relay_profile(good_profile(), RELAY)
        self.assertEqual(found, [])
        self.assertEqual(facts["chained_routes"], 1)
        self.assertEqual(facts["selectable"], sorted([VIA, "auto"]))

    def test_an_unexpected_relay_host_is_reported(self):
        self.assertTrue(any("expected relay" in p for p in problems(good_profile(), "other.example.test")))

    def test_a_profile_without_a_chained_route_is_a_direct_exit_profile(self):
        p = good_profile()
        del p["outbounds"][1]["detour"]
        self.assertTrue(any("exactly one exit route" in x for x in problems(p)))

    def test_two_chained_routes_are_refused(self):
        p = good_profile()
        second = copy.deepcopy(p["outbounds"][1])
        second["tag"] = "second"
        p["outbounds"].insert(2, second)
        self.assertTrue(any("exactly one exit route" in x for x in problems(p)))

    def test_a_selectable_direct_outbound_is_a_bypass(self):
        p = good_profile()
        p["outbounds"][3]["outbounds"].append("direct")
        found = problems(p)
        self.assertTrue(any("direct" in x for x in found), found)

    def test_a_directly_selectable_relay_hop_is_a_bypass(self):
        p = good_profile()
        p["outbounds"][3]["outbounds"].append(HOP)
        self.assertTrue(any("relay hop is directly selectable" in x for x in problems(p)))

    def test_route_final_direct_is_a_bypass(self):
        p = good_profile()
        p["route"]["final"] = "direct"
        self.assertTrue(any("route.final" in x for x in problems(p)))

    def test_a_hysteria2_outbound_is_unexpected_on_a_relay(self):
        p = good_profile()
        p["outbounds"].append({"type": "hysteria2", "tag": "hy2", "server": RELAY})
        self.assertTrue(any("Hysteria2" in x for x in problems(p)))

    def test_an_automatic_group_must_contain_only_the_chained_route(self):
        p = good_profile()
        p["outbounds"].append({"type": "vless", "tag": "stray", "server": EXIT})
        p["outbounds"][2]["outbounds"].append("stray")
        self.assertTrue(problems(p))

    def test_same_server_for_relay_and_exit_is_refused(self):
        p = good_profile()
        p["outbounds"][1]["server"] = RELAY
        self.assertTrue(any("same server" in x for x in problems(p)))

    def test_a_selector_defaulting_elsewhere_is_refused(self):
        p = good_profile()
        p["outbounds"][3]["default"] = "auto"
        self.assertTrue(any("defaults" in x for x in problems(p)))


class FetchRedaction(unittest.TestCase):
    """A failed fetch must never put the subscription URL (it holds the token) into the error or the output."""

    def serve(self, status):
        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(status)
                self.end_headers()

            def log_message(self, *a):
                pass

        srv = http.server.HTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.shutdown)
        return srv.server_address[1]

    def test_http_errors_do_not_leak_the_url(self):
        port = self.serve(400)
        secret = "TOKEN-SHOULD-NEVER-APPEAR-1234567890"
        with self.assertRaises(hca.FetchError) as ctx:
            hca.fetch(f"http://127.0.0.1:{port}/sub/{secret}?format=uri")
        self.assertNotIn(secret, str(ctx.exception))
        self.assertIn("400", str(ctx.exception))

    def test_malformed_urls_do_not_leak_the_url(self):
        secret = "TOKEN-SHOULD-NEVER-APPEAR-1234567890"
        with self.assertRaises(hca.FetchError) as ctx:
            hca.fetch(f"no-scheme/sub/{secret}")
        self.assertNotIn(secret, str(ctx.exception))

    def test_connection_errors_do_not_leak_the_url(self):
        secret = "TOKEN-SHOULD-NEVER-APPEAR-1234567890"
        with self.assertRaises(hca.FetchError) as ctx:
            hca.fetch(f"http://127.0.0.1:1/sub/{secret}")
        self.assertNotIn(secret, str(ctx.exception))


class ModesAreSeparated(unittest.TestCase):
    def test_cli_exposes_both_modes_with_direct_as_the_default(self):
        src = open(os.path.join(HERE, "..", "hiddify_core_acceptance.py")).read()
        self.assertIn('choices=("direct", "relay"), default="direct"', src)
        # relay mode must fetch the pinned profile only
        relay_branch = src.split('if a.mode == "relay":')[1].split("else:")[0]
        self.assertIn("compat=hiddify-pinned", relay_branch)
        self.assertNotIn("format=uri", relay_branch)
        self.assertNotIn("hysteria2-json-only", relay_branch)


if __name__ == "__main__":
    unittest.main()
