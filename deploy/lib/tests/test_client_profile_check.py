import copy
import importlib.util
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("cpc", os.path.join(HERE, "..", "client_profile_check.py"))
cpc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cpc)
H = "node.example.com"
PBK = "A" * 43


def doc(pinned=False):
    d = {"outbounds": [
        {"type": "vless", "tag": "Reality", "server": H, "server_port": 443, "uuid": "u", "flow": "xtls-rprx-vision",
         "tls": {"enabled": True, "server_name": "www.example.org", "utls": {"enabled": True, "fingerprint": "chrome"},
                 "reality": {"enabled": True, "public_key": PBK, "short_id": "ab12cd34"}}}],
        "route": {"final": "select"}}
    if not pinned:
        d["outbounds"].append({"type": "hysteria2", "tag": "Hysteria2", "server": H, "server_port": 443, "password": "p",
                               "obfs": {"type": "salamander", "password": "o"}, "tls": {"enabled": True, "insecure": False, "server_name": H}})
    grp = ["Reality"] if pinned else ["Reality", "Hysteria2"]
    d["outbounds"] += [{"type": "urltest", "tag": "auto", "outbounds": grp}, {"type": "selector", "tag": "select", "outbounds": grp + ["auto"], "default": "Reality"},
                       {"type": "direct", "tag": "direct"}]
    return d


def fails(checks): return [c["check"] for c in checks if c["result"] == "FAIL"]


class SingboxDoc(unittest.TestCase):
    def test_good_profiles_pass(self):
        self.assertEqual(fails(cpc.check_singbox_doc(doc(), H, False)), [])
        self.assertEqual(fails(cpc.check_singbox_doc(doc(True), H, True)), [])

    def test_negative_fixtures_each_fail_the_right_check(self):
        def mut(f, pinned=False):
            d = doc(pinned); f(d); return fails(cpc.check_singbox_doc(d, H, pinned))
        self.assertTrue(mut(lambda d: d["outbounds"][1]["tls"].update(insecure=True)))                        # TLS verification disabled
        self.assertTrue(mut(lambda d: d["outbounds"][0]["tls"]["reality"].update(public_key="short")))        # malformed pbk
        self.assertTrue(mut(lambda d: d["outbounds"][0].update(flow="")))
        self.assertTrue(mut(lambda d: d["outbounds"][0].update(server="203.0.113.7")))                          # not the public host name
        self.assertTrue(mut(lambda d: d["outbounds"][0].update(server="[2001:db8::1]")))
        self.assertTrue(mut(lambda d: d.update(note="-----BEGIN PRIVATE KEY-----")))                           # private material leaked
        self.assertTrue(mut(lambda d: d["outbounds"][3].update(default="auto")))                              # racing default
        self.assertTrue(mut(lambda d: d["route"].update(final="direct")))                                       # traffic would bypass the tunnel
        self.assertTrue(mut(lambda d: d["outbounds"][1].update(obfs=None)))                                    # obfuscation missing

    def test_pinned_profile_must_expose_exactly_one_route(self):
        d = doc(True); d["outbounds"].insert(1, doc()["outbounds"][1])                                         # a Hysteria2 outbound sneaks in
        d["outbounds"][2]["outbounds"] = ["Reality", "Hysteria2"]
        self.assertTrue(any("exactly one route" in f for f in fails(cpc.check_singbox_doc(d, H, True))))


class Uris(unittest.TestCase):
    def txt(self, **over):
        q = dict(encryption="none", security="reality", sni="www.example.org", fp="chrome", pbk=PBK, sid="ab12", type="tcp", flow="xtls-rprx-vision")
        q.update(over)
        v = "vless://u@%s:443?%s#Reality" % (H, "&".join(f"{k}={x}" for k, x in q.items()))
        return v + "\nhysteria2://p@%s:443?sni=%s&insecure=0&obfs=salamander&obfs-password=o#Hysteria2\n" % (H, H)

    def test_good_and_bad(self):
        self.assertEqual(fails(cpc.parse_uris(self.txt(), H)), [])
        self.assertTrue(fails(cpc.parse_uris(self.txt(security="tls"), H)))
        self.assertTrue(fails(cpc.parse_uris(self.txt(pbk="x"), H)))
        self.assertTrue(fails(cpc.parse_uris(self.txt().replace("insecure=0", "insecure=1"), H)))
        self.assertTrue(fails(cpc.parse_uris(self.txt().split("\n")[0], H)))                                    # hysteria2 link missing


if __name__ == "__main__":
    unittest.main()
