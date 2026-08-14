# Russia production investigation (2026-08-14)

## Scope, evidence labels, and baseline

This investigation analyzed `74b63cd886dd54e8601520980de578ef55c81c2b` on branch
`work`. The initial tree had one pre-existing untracked file,
`fuzz/Cargo.lock`; it was not modified or committed. There is no configured Git
remote in this checkout, so ahead/behind and remote fetch results are
**UNVERIFIED**. The host was Ubuntu 24.04.4, x86_64, root, Rust/Cargo 1.94.1,
Bash 5.2.21. `systemctl` exists, but this container is not a production systemd
host; sing-box, Docker, firewalld, SELinux tools, nft, certbot, shellcheck,
cargo-deny, and cargo-audit were unavailable. No production VPS credentials,
Russian device, Russian network, or affected Hiddify installation was supplied.

Consequently, local code/test outcomes below use **PASS** or **FAIL**, while all
claims about Germany, Russia, real devices, the live Outline deployment, or a
real VPS remain **UNVERIFIED** or **BLOCKED BY ENVIRONMENT**. “CONFIRMED” means
confirmed only at the stated layer; it never upgrades a network acceptance cell.

## Architecture and product surfaces

```text
 device
  +-- Hiddify/independent sing-box -- client-owned TUN or proxy-only, DNS,
  |                                  IPv4/IPv6 and routes
  |        | VLESS+REALITY TCP/443 or optional Hysteria2 UDP/443
  |        v
  |     sing-box listeners ----------------------> direct server outbound
  |        ^                                              |
  |        | subscription JSON/URI                        v
  | reverse proxy:8443 <-- Rust subscription service --> Internet destination
  |
  +-- native Rust client-daemon -> native relay-agent -> Internet destination

 separately managed Outline client -> existing Outline listener -> Internet
```

The native Rust policy/failure-classifier stack and the third-party-client
deployment are separate products. The former does not select transports for
Hiddify. The production-compatible renderer, subscription service, server
renderer, admin diagnostics, and lifecycle scripts are code-verified and
unit/integration tested locally. The external-client TUN/DNS/route behavior,
live reverse proxy, live listeners, direct VPS egress, and Outline are not
locally reproducible. Real-VPS, real-device, and Russian-network status for
every component is **UNVERIFIED**.

## Generated profile audit (CONFIRMED by renderer tests)

The native JSON contains VLESS, UUID, Vision flow, REALITY, public key, short
ID, SNI, chrome uTLS; Hysteria2, TLS/SNI and optional Salamander; a `urltest`
named `auto`; a manual selector; direct outbound; and `route.final: select`.
The deterministic default is REALITY; `auto` remains selectable and probes
exactly `https://www.gstatic.com/generate_204`. That is only a small generic
HTTPS latency/reachability probe, not Telegram, sustained transfer, DPI, or
Russia health.

The renderer emits no DNS block/rules, IP-family policy, inbound, TUN, mixed or
SOCKS listener, bypass/region rule, kill switch, auto/strict route, MTU, TLS
fragment/padding, mux/multiplex, packet encoding, TCP keepalive, or TCP fast
open. Thus it cannot force tunneled DNS, IPv4, full-device routing, or prevent
IPv6 escape. Hiddify owns those settings and may override/ignore imported
intent. URI lists carry individual transport credentials but have no selector
or route semantics, so URI and JSON imports are not behaviorally equivalent.
Repository output does not explain mux seen in production logs; client defaults,
a different profile, or deployed drift must be tested. Hiddify field retention
and current versions are **UNVERIFIED** because source/network research and the
affected app were unavailable.

## Executed versus unavailable experiments

| Experiment | Result | Evidence classification |
|---|---|---|
| Exact renderer shape and absence assertions | **PASS** locally | CONFIRMED at serialization layer |
| Rust workspace tests/lints and repository gates | see final test log | CONFIRMED locally only |
| Disposable AlmaLinux lifecycle | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| sing-box schema/protocol interop | **BLOCKED BY ENVIRONMENT** when binary/network absent | UNVERIFIED for live path |
| Germany working observation | **UNVERIFIED** | ANECDOTAL |
| Russia mobile/Wi-Fi, Hiddify, raw sing-box, Outline A/B | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| Fragmentation packet A/B | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| REALITY mux/no-mux A/B | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |
| UDP loss/block/rate/MTU tests | **BLOCKED BY ENVIRONMENT** | UNVERIFIED |

No exact measured live differences between Outline, REALITY, and Hysteria2 can
truthfully be reported. Protocol-level known differences (Shadowsocks versus
TCP REALITY versus QUIC/UDP Hysteria2) are not evidence for why this incident
occurs.

## Ranked hypothesis tree

1. **Client TUN/routing/split-route/IPv6 behavior — LIKELY, medium confidence.**
   Evidence for: “connected” with a Russian public IP is compatible with a
   proxy-only or inactive/partial TUN, and the subscription does not configure
   TUN, DNS, or IP-family routing. Evidence against: no affected-device route or
   packet capture exists, and an authenticated but stalled tunnel could create
   similar UI symptoms. Falsifier: on the same affected version/profile,
   capture device routes plus IPv4 and IPv6 public-IP probes in VPN/TUN mode;
   prove both families leave through the VPS while the symptom remains. Result:
   **UNVERIFIED**.
2. **Hiddify integration/version defect — LIKELY, low-to-medium confidence.**
   Evidence for: the client owns routing/import behavior and can report app
   state independently of system routing. Evidence against: raw upstream
   sing-box has not succeeded on the same Russian path. Falsifier: one-variable
   A/B using the minimal raw client and Hiddify, identical credentials/network/
   destination/time. Result: **UNVERIFIED**.
3. **Post-handshake interference or PMTU/path failure — LIKELY, medium
   confidence.** Evidence for: reported brief success followed by stalls.
   Evidence against: no repeatable byte threshold, retransmission trace, RST,
   ICMP PTB, or synchronized log exists. Falsifier: three exact-size repetitions
   per transport with synchronized bounded capture, varying only MTU in a second
   phase. Result: **UNVERIFIED**.
4. **Stale/mismatched REALITY state — POSSIBLE, low confidence.** Evidence for:
   intermittent invalid-connection logs can accompany a stale profile. Evidence
   against: that message can also describe unauthenticated probes and repository
   doctor already checks stored/live fingerprints. Falsifier: correlate a
   client attempt timestamp/profile fingerprint with server log and a successful
   minimal-client authentication using the same material. Result:
   **UNVERIFIED**.
5. **UDP blocking/throttling specific to Hysteria2 — POSSIBLE, medium confidence
   for Hysteria2 only.** Evidence for: Hysteria2 requires UDP. Evidence against:
   it cannot explain REALITY/TCP or a Russian IP after claimed TUN activation.
   Falsifier: simultaneous TCP/443 and UDP/443 captures on the same path, then a
   controlled UDP-blocked network. Result: **UNVERIFIED**.
6. **Generic VPS, DNS, TCP/443, or single bad SNI — SPECULATIVE, low
   confidence.** Outline on the reported same VPS and Germany observation argue
   against a total outage, but are anecdotal. Dual-family target validation and
   synchronized A/B are the falsifiers. Result: **UNVERIFIED**.

EOF/`processed invalid connection` alone must never be classified as DPI or bad
credentials. Suggested diagnostic classes after evidence collection are
`TCP_REACHABLE_BUT_POST_HANDSHAKE_STALL`, `CENSORSHIP_SUSPECTED`,
`PMTU_OR_MTU_FAILURE`, `CLIENT_CORE_BUG`, `CLIENT_TUN_NOT_ACTIVE`,
`DNS_FAILURE`, `UDP_BLOCKED`, and `UNKNOWN`; default to `UNKNOWN` when the
required discriminating evidence is missing.

## Implemented changes, compatibility, rollback

The renderer now has an exact regression contract for required REALITY fields,
the URLTest URL, and absence of client-owned/experimental fields. This changes
no emitted profile bytes and has no compatibility impact. A new read-only
investigation helper validates a target by DNS, IPv4/IPv6 TCP+TLS+HTTP timing,
certificate hostname, optional ICMP loss, performs a root/operator-triggered
bounded capture filtered to one client and ports 443, and summarizes packet
metadata without payload. It has no install/service/firewall integration, so
rollback is deletion of the helper and test. Its validation test covers bounds
and injection-resistant inputs. Real-device testing remains required.

## Architecture choices and recommendation

* **Option A — minimal change:** retain REALITY primary and optional Hysteria2;
  make independent raw sing-box the control client and require explicit TUN and
  dual-family public-IP acceptance. Lowest risk, but one VPS/IP remains a single
  censorship and outage domain.
* **Option B — genuinely different fallback:** after the controlled experiment,
  keep the existing Outline deployment separately managed or add a separately
  ported maintained Shadowsocks implementation. Do not touch or import Outline
  credentials/lifecycle. This is preferred only if synchronized Russian A/B
  proves a Shadowsocks-family advantage and port preflight/coexistence tests pass.
* **Option C — Russia-first:** multiple providers/ASNs/endpoints plus at least two
  measured wire families, client-specific onboarding, and continuous Russian
  mobile/fixed acceptance. This changes v1 product scope and operations.

Recommendation today: **Option A while collecting evidence**. Hiddify should be
listed as provisional, not universally recommended. REALITY may remain the
deterministic primary only as an unverified default, Hysteria2 should remain an
optional UDP path, and no new Shadowsocks service should be added until the
Outline control experiment identifies a reproducible advantage. vpn1 cannot
truthfully be called Russia-ready: status is **UNVERIFIED**.

## Exact operator procedure

On the VPS, first validate the configured target without printing credentials:

```bash
sudo /opt/vpn1/deploy/lib/vpn-investigate.sh target www.google.com 443
sudo vpn-admin doctor --protocol
sudo vpn-admin doctor --report --report-output /root/vpn-doctor.txt
```

For each Russian phone network (mobile, then Wi-Fi), record UTC time, phone/app/
core versions, profile fingerprint, region, VPN/TUN versus Proxy Only, and
selected transport. Disable mux and fragmentation for baseline. Verify both
`https://api4.ipify.org` and `https://api6.ipify.org` (an unavailable family is
recorded, not silently passed), then DNS, browser, Telegram text/media/call, and
three repetitions each of 1, 4, 8, 16, 20, 32, 64, 256 KiB, 1 MiB, and 10 MiB
from an operator-controlled exact-size endpoint. Record TTFB, completion,
received bytes and error. Repeat, changing only client: Hiddify, minimal raw
upstream sing-box (one loopback SOCKS inbound, one REALITY outbound, no DNS,
selector, urltest, mux, or routes), independent client, and Outline. Run each
stable path for ten minutes, screen-off idle, and Wi-Fi/cellular handover.

At the synchronized UTC start, restrict capture to the phone's observed source
IP (never a broad public capture):

```bash
sudo /opt/vpn1/deploy/lib/vpn-investigate.sh capture CLIENT_PUBLIC_IP /root/run.pcap 120
sudo journalctl -u sing-box --since '2026-08-14 12:00:00 UTC' \
  --until '2026-08-14 12:02:30 UTC' --output short-iso > /root/run-singbox.log
sudo /opt/vpn1/deploy/lib/vpn-investigate.sh summarize /root/run.pcap > /root/run-summary.txt
sudo chmod 600 /root/run.pcap /root/run-singbox.log /root/run-summary.txt
```

Redact source IPs and inspect logs for secrets before sharing. Compare same VPS,
phone, network, destination, duration, and timestamps. Then perform one-variable
A/B for mux; fragmentation on/off requires a client-side capture and ClientHello
packet-length/timing comparison. MTU 1280/1360/1400/1500 comes only after the
baseline and never becomes a global default from one run. Test system DNS,
8.8.8.8/1.1.1.1 UDP, DoH/DoT, raw IP/hostname, IPv4/IPv6/dual stack, private and
split DNS separately. Record every unexecuted cell **UNVERIFIED**.

## Known limitations and next gate

The helper cannot infer authentication from packets, inspect phone TUN state,
create an exact-size HTTP server, or test censorship. Captures truncated to 160
bytes intentionally minimize payload exposure, so detailed TLS fingerprinting
needs a separately approved client/server capture procedure. Hiddify release/
issue research was **BLOCKED BY ENVIRONMENT** by unavailable authenticated web
access and must be repeated against upstream source/release artifacts. Live
Outline measurements, certificate/key coherence, resource/load stress,
AlmaLinux lifecycle/reboot/SELinux/firewalld, and Russian acceptance remain
**UNVERIFIED**. The next architecture decision gate is a complete synchronized
Outline/REALITY/Hysteria2/raw-sing-box matrix—not another speculative protocol
setting change.
