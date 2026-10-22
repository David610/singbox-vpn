"""af_capture.py (RECONSTRUCTED 2026-10-22; the corrected original is MISSING). Fixture-only: no capture, no root."""
import importlib.util
import json
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("afc", os.path.join(HERE, "..", "diagnostics", "af_capture.py"))
afc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(afc)

SRV4, SRV6 = "62.238.46.190", "2a01:4f9:c014:13bd::1"
PHONE4, PHONE6 = "198.51.100.7", "2001:db8:77::5"
T0 = 1792650000

# Real tcpdump -n -tt -e -l output shapes (addresses replaced by documentation/test ranges, the server's own kept).
def l4tcp(t, src, dst, n): return f"{t:.6f} 96:00:09:f2:3f:49 > d2:74:7f:6e:37:e3, ethertype IPv4 (0x0800), length {n}: {src} > {dst}: Flags [P.], seq 1:{n-66}, ack 1, win 501, length {n-66}"
def l4udp(t, src, dst, n): return f"{t:.6f} 96:00:09:f2:3f:49 > d2:74:7f:6e:37:e3, ethertype IPv4 (0x0800), length {n}: {src} > {dst}: UDP, length {n-42}"
def l6tcp(t, src, dst, n): return f"{t:.6f} 96:00:09:f2:3f:49 > d2:74:7f:6e:37:e3, ethertype IPv6 (0x86dd), length {n}: {src} > {dst}: Flags [.], ack 1, win 501, length {n-86}"
def l6udp(t, src, dst, n): return f"{t:.6f} 96:00:09:f2:3f:49 > d2:74:7f:6e:37:e3, ethertype IPv6 (0x86dd), length {n}: {src} > {dst}: UDP, length {n-62}"
def l6trunc(t, src, dst, n): return f"{t:.6f} 96:00:09:f2:3f:49 > d2:74:7f:6e:37:e3, ethertype IPv6 (0x86dd), length {n}: {src} > {dst}: [|tcp]"
NOE_6TRUNC = f"{T0 + 1:.6f} IP6 {SRV6}.40000 > 2a00:1450:4001:82b::200e.443: [|tcp]"   # no -e: packet counted, bytes unknown


def cfg(**kw):
    c = afc.Config(server_addrs={SRV4, SRV6}, tunnel_ports={("tcp", 443), ("udp", 8443)})
    for k, v in kw.items():
        setattr(c, k, v)
    return c


def run(lines, **kw):
    agg = afc.Aggregator(cfg(**kw))
    for ln in lines:
        agg.feed(ln)
    return agg


def phone_and_google(t, fam=6):
    # one bin with the phone's tunnel ingress and the server's Google egress in the same 5 s
    tun = l4tcp(t, f"{PHONE4}.51000", f"{SRV4}.443", 1400) if fam == 4 else l6udp(t, f"{PHONE6}.51000", f"{SRV6}.8443", 1300)
    goo = l6udp(t + 0.5, f"{SRV6}.40001", "2a00:1450:4001:82b::200e.443", 1350) if fam == 6 else l4tcp(t + 0.5, f"{SRV4}.40001", "142.250.74.14.443", 1500)
    return [tun, goo]


class Parse(unittest.TestCase):
    def test_four_line_shapes_and_truncated_tcp(self):
        p = afc.parse_line
        self.assertEqual(p(l4tcp(T0, f"{SRV4}.40000", "142.250.74.14.443", 1500))[1:], ("4", "tcp", SRV4, 40000, "142.250.74.14", 443, 1500))
        self.assertEqual(p(l4udp(T0, f"{SRV4}.40000", "142.250.74.14.443", 1292))[1:4], ("4", "udp", SRV4))
        self.assertEqual(p(l6tcp(T0, f"{SRV6}.40000", "2a00:1450:4001:82b::200e.443", 1294))[1:],
                         ("6", "tcp", SRV6, 40000, "2a00:1450:4001:82b::200e", 443, 1294))
        self.assertEqual(p(l6udp(T0, f"{SRV6}.40000", "2607:f8b0:4004:c1b::5e.443", 1350))[1:3], ("6", "udp"))
        # the original defect: IPv6 TCP shown as [|tcp] was dropped; it is TCP and the frame length still counts
        self.assertEqual(p(l6trunc(T0, f"{SRV6}.40000", "2a00:1450:4001:82b::200e.443", 1294))[1:3] + (p(l6trunc(T0, f"{SRV6}.4", "2a00:1450::1.443", 1294))[7],), ("6", "tcp", 1294))
        r = p(NOE_6TRUNC)
        self.assertEqual((r[1], r[2], r[7]), ("6", "tcp", None))

    def test_malformed_lines_are_counted_not_fatal(self):
        agg = run(["", "garbage", "1.0 IP6 ::1 > ::2: weird", "tcpdump: listening on eth0", "x" * 5000])
        self.assertEqual(agg.malformed, 5)
        self.assertEqual(agg.bins, {})


class Google(unittest.TestCase):
    def test_default_prefixes_cover_google_ipv6_allocations(self):
        g = afc.GooglePrefixes.default()
        for a in ("2a00:1450:4001:82b::200e", "2607:f8b0:4004:c1b::5e", "2404:6800:4003::1", "2001:4860:4860::8888",
                  "2800:3f0:4001::1", "142.250.74.14", "172.217.16.142", "216.58.208.110", "74.125.1.1", "173.194.1.1"):
            self.assertTrue(g.contains(a), a)
        for a in ("1.1.1.1", "2606:4700::1111", SRV6, PHONE6):
            self.assertFalse(g.contains(a), a)

    def test_prefixes_load_from_goog_json(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "goog.json")
            json.dump({"prefixes": [{"ipv4Prefix": "203.0.113.0/24"}, {"ipv6Prefix": "2001:db8:aaaa::/48"}]}, open(p, "w"))
            g = afc.GooglePrefixes.load(p)
        self.assertTrue(g.contains("203.0.113.9") and g.contains("2001:db8:aaaa::1"))
        self.assertFalse(g.contains("142.250.74.14"))    # the file replaces the default


class Aggregate(unittest.TestCase):
    def test_bins_hold_only_aggregates_by_family_and_protocol(self):
        agg = run(phone_and_google(T0, 6) + phone_and_google(T0 + 1, 4) + [l6trunc(T0 + 2, f"{SRV6}.4", "2a00:1450::1.443", 1294)])
        b = agg.bins[T0 - T0 % 5]
        self.assertEqual(b["tunnel_in"]["6"]["udp"], 1300)
        self.assertEqual(b["tunnel_in"]["4"]["tcp"], 1400)
        self.assertEqual(b["google_out"]["6"]["udp"], 1350)
        self.assertEqual(b["google_out"]["6"]["tcp"], 1294)
        self.assertEqual(b["google_out"]["4"]["tcp"], 1500)
        doc = json.dumps(agg.report())
        for secret in (PHONE4, PHONE6, "51000", "2a00:1450:4001:82b::200e", "142.250.74.14"):
            self.assertNotIn(secret, doc)          # privacy: no client or destination addresses, no ports

    def test_google_reply_direction_counts_as_google_traffic(self):
        agg = run([l6udp(T0, "2a00:1450:4001:82b::200e.443", f"{SRV6}.40001", 1350)])
        self.assertEqual(agg.bins[T0 - T0 % 5]["google_in"]["6"]["udp"], 1350)

    def test_bytes_unknown_is_reported_not_guessed(self):
        agg = run([NOE_6TRUNC])
        b = agg.bins[(T0 + 1) - (T0 + 1) % 5]
        self.assertEqual((b["google_out"]["6"]["tcp"], b["bytes_unknown_packets"]), (0, 1))


class Overlap(unittest.TestCase):
    def window(self, agg, a, b, canary=()):
        return afc.classify_window(agg.report(), a, b, canary_windows=list(canary))

    def test_simultaneous_tunnel_and_google_is_valid_overlap(self):
        agg = run(sum((phone_and_google(T0 + 5 * i, 6) for i in range(6)), []))
        r = self.window(agg, T0, T0 + 30)
        self.assertEqual(r["verdict"], "VALID_OVERLAP")
        self.assertEqual(r["google_bytes_by_family"]["6"], 6 * 1350)
        self.assertIn("not prove", r["limits"])      # correlation, never a claim about packet contents

    def test_missing_capture_window_is_inconclusive(self):
        agg = run(sum((phone_and_google(T0 + 5 * i, 6) for i in range(6)), []))
        r = self.window(agg, T0 + 600, T0 + 660)
        self.assertEqual((r["verdict"], r["reason"]), ("NO_OVERLAP_INCONCLUSIVE", "capture_does_not_cover_window"))

    def test_no_phone_activity_is_inconclusive(self):
        agg = run([l6udp(T0 + 5 * i, f"{SRV6}.40001", "2a00:1450:4001:82b::200e.443", 1350) for i in range(6)])
        r = self.window(agg, T0, T0 + 30)
        self.assertEqual((r["verdict"], r["reason"]), ("NO_OVERLAP_INCONCLUSIVE", "no_tunnel_activity"))

    def test_canary_only_google_traffic_is_inconclusive(self):
        # the earlier mis-timed capture: only the agent's own canary requests reached Google
        lines = [l4tcp(T0 + 5 * i, f"{PHONE4}.51000", f"{SRV4}.443", 200) for i in range(6)]
        lines += [l6tcp(T0 + 12, f"{SRV6}.40002", "2a00:1450:4001:82b::200e.443", 1294)]
        agg = run(lines)
        r = self.window(agg, T0, T0 + 30, canary=[(T0 + 10, T0 + 15)])
        self.assertEqual((r["verdict"], r["reason"]), ("NO_OVERLAP_INCONCLUSIVE", "google_traffic_only_in_canary_windows"))

    def test_tunnel_without_google_is_inconclusive(self):
        agg = run([l4tcp(T0 + 5 * i, f"{PHONE4}.51000", f"{SRV4}.443", 1400) for i in range(6)])
        self.assertEqual(self.window(agg, T0, T0 + 30)["reason"], "no_google_traffic")

    def test_single_coincident_bin_is_not_enough(self):
        lines = phone_and_google(T0, 6) + [l4tcp(T0 + 20, f"{PHONE4}.51000", f"{SRV4}.443", 1400)]
        r = self.window(run(lines), T0, T0 + 30)
        self.assertEqual((r["verdict"], r["reason"]), ("NO_OVERLAP_INCONCLUSIVE", "too_few_coincident_bins"))


class Lifecycle(unittest.TestCase):
    def test_capture_command_is_bounded_and_keeps_no_packets(self):
        cmd = afc.capture_command("eth0", 10_000, cfg())
        self.assertEqual(cmd[:2], ["timeout", str(afc.MAX_DURATION_S)])          # automatic expiry, capped
        self.assertIn("-s", cmd)
        self.assertGreaterEqual(int(cmd[cmd.index("-s") + 1]), 128)           # large enough for IPv6+TCP headers
        self.assertNotIn("-w", cmd)                                           # no pcap is ever written
        for f in ("-n", "-tt", "-e", "-l"):
            self.assertIn(f, cmd)

    def test_status_reports_live_tunnel_and_expiry(self):
        agg = run(phone_and_google(T0, 6))
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "state.json")
            afc.write_state(p, agg, started=T0 - 10, expires=T0 + 300, pid=os.getpid())
            s = afc.status(p, now=T0 + 3)
            self.assertEqual((s["running"], s["tunnel_seen_recently"], s["google_seen_recently"]), (True, True, True))
            self.assertEqual(s["expires_in_s"], 297)
            s = afc.status(p, now=T0 + 400)
            self.assertFalse(s["running"])                                    # past expiry: never reported as live
            self.assertFalse(s["tunnel_seen_recently"])
            self.assertEqual(afc.status(os.path.join(d, "missing.json"), now=T0)["running"], False)


if __name__ == "__main__":
    unittest.main()
