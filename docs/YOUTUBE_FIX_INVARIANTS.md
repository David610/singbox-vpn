# YouTube / Shorts fix — invariants that must never be broken

**Read this before touching anything that routes, renders, deploys or
"improves" Google/YouTube traffic.** That includes `google_egress_hairpin`,
`GOOGLE_EGRESS_DOMAINS`, exit route rules, sniffing, QUIC/UDP handling,
IPv6 handling, DNS, the REALITY/Hysteria2 transports, compat modes and any
deploy of the nodes named below. This document is authoritative over
`YOUTUBE_FINAL_ROOT_CAUSE.md` §1–§17, whose theories were disproven (see §18
there).

**Owner decisions in this file are not suggestions.** A future session may
not reverse INV-7 or INV-8 on its own initiative, however convenient.

## Status

| Claim | Evidence | Class |
|---|---|---|
| Shorts and ordinary videos play in the native iPhone YouTube app through Hiddify with this fix | Owner confirmed "now it works" on 2026-10-11 after a guided test (Shorts incl. `sAcElROnYIE`, consecutive Shorts, one normal video). Protocol, network, session length and seeking were **not** recorded separately. | **Native device verified (user reported)** |
| Shorts play in Chrome (iPhone emulation) through the real VLESS+REALITY tunnel with the fix, 6/6 consecutive | `deploy/lib/youtube_shorts_probe.py`, 2026-10-11 | Browser verified |
| Google QUIC flows leave the exit through the paired node, none through the exit's own IP | tcpdump on the exit NIC: 0 UDP packets to the test Google IPs, 1718 packets to the paired node | Server verified |
| Hysteria2, cellular, 15-minute sustained session, post-reboot | not tested | **Unverified** |

Do not claim more than this table says.

## Root cause (proven 2026-10-11, do not re-derive)

YouTube's media server refuses to serve **Shorts** video to some datacenter
IPs. The player API still answers `playabilityStatus: OK` with a full format
list, but the `videoplayback` request returns a 155-byte control-only
`application/vnd.yt-ump` reply instead of media, so the page shows "Video
unavailable". Ordinary (long-form) videos from the same IP still play. It is
a property of the **egress IP**, not of the VPN.

| Egress IP (ASN) | Shorts, Chrome iPhone emulation, 6 consecutive |
|---|---|
| Owner's own broadband line | 6/6 play |
| `91.244.71.165` Evolus IT AS215120 via VLESS+REALITY | **0/6** |
| same IP via **plain SSH-forwarded TCP**, no VPN protocol at all | **0/6** (IPv4-only 0/6, IPv6-only 0/6) |
| `62.238.46.190` Hetzner AS24940, plain TCP | **0/6**, and one normal video got "Sign in to confirm you're not a bot" |
| `135.106.178.167` Selectel AS50340, plain TCP | **6/6 play**; media replies 661 KB and 326 KB |
| Cloudflare WARP from the Evolus node | 6/6 play — **prohibited, see INV-7** |

Consequences, all measured: QUIC/UDP, IPv6, REALITY/XUDP, UFW, MTU and the
server-to-Google path are **not** the cause (server-to-Google QUIC handshakes
12/12 at ~35 ms; every Initial answered within 250 ms; no ICMP errors;
UFW dropped 3 Google packets in 15 minutes). The old "CDN-edge latency /
better peering" theory (§16) is wrong: the Selectel node works because
YouTube accepts its IP, and the latency gap no longer reproduces.

## The fix and where it runs

The German exit (`de1`, `91.244.71.165`) sends only the Google/YouTube
domain set out through a dedicated user on the Selectel node (`ru1`,
`135.106.178.167`), via `vpn-admin google-egress-hairpin set`. Everything
else exits directly from the German IP. No client change, no second profile,
works in Hiddify by default.

Rendered exit rules, in this order (after the four mandatory C-16 rules):
`sniff` → reject literal-IPv6 Google destinations (`no_drop`) → route the
domain set to the `google-egress-hairpin` VLESS+REALITY outbound;
`route.final = direct`.

## Invariants

Each has an enforcing test or procedure. If you must change one, change the
test **and** this document in the same commit and say why in the message.

**INV-1 — Google/YouTube domains never egress from an IP YouTube refuses.**
The whole `GOOGLE_EGRESS_DOMAINS` set (`crates/compat-config/src/model.rs`)
goes through the hairpin. Do not narrow it, "optimise" it to `googlevideo.com`
only, or replace it with IP lists. Auth, player API, static assets and media
each live on a different domain; a hole in any of them breaks Shorts.
*Enforced by:* `exit_with_hairpin_configured_adds_one_outbound_and_sniff_plus_route_rule`.

**INV-2 — Do not "fix" this with transport, protocol or client tweaks.**
These were tried or measured and do not change the outcome: rejecting or
disabling QUIC/UDP-443, disabling IPv6, MTU/DNS changes, Vision-off,
multiplex, downgrading sing-box, Hysteria2 vs REALITY, `compat=quic-reject`,
`compat=youtube-direct`, `compat=hiddify-pinned` (kept only for its separate
routing/privacy fix). Hiddify **discards imported `route.rules`**
(`YOUTUBE_FINAL_ROOT_CAUSE.md` §12), so no client-side routing rule can work.
If Shorts break again, first rerun the probe below against each egress IP; do
not start by editing transports.

**INV-3 — The fix is server-side and client-transparent.** A user imports the
normal subscription into Hiddify and nothing else. Never add a client switch,
a second profile or a per-user opt-in as the way Shorts work.

**INV-4 — `sniff` stays first.** iOS TUN clients resolve DNS themselves and
send bare IPs; only a sniffed SNI (TLS or QUIC) lets the domain rule match.
Per-inbound `sniff` is removed in sing-box 1.13, so it must be the
`{"action":"sniff"}` route rule.
*Enforced by:* `exit_hairpin_rejects_literal_ipv6_google_destinations_without_dropping`.

**INV-5 — Literal-IPv6 Google destinations are rejected with `no_drop: true`.**
The paired node is IPv4-only, so such a flow would otherwise stall until the
client's own timeout (measured: 0/20 handshakes). Without `no_drop`, sing-box
turns reject into a silent drop after 50 rejects in 30 s, restoring the stall
mid-session. The rule matches only literal IPv6 destinations: sing-box sets
`ip_version` in route rules solely from a literal destination
(`route/route.go` `prepareMatchMetadata`), so domain flows are never caught,
even after the C-16 `resolve` action. Do not replace it with `ip_cidr: ::/0`
(that *does* match resolved addresses and would reject real traffic).
*Enforced by:* `exit_hairpin_rejects_literal_ipv6_google_destinations_without_dropping`,
and `vpn-admin doctor` fails an exit whose hairpin rules lack it.

**INV-6 — Fail closed, never fall back to the refused IP.** Exactly one route
for the domain set; no `selector`/`urltest` that offers `direct` as an
alternative; `route.final = direct` only for non-Google traffic. If the paired
node is down, YouTube must fail visibly rather than silently exit from the
refused IP and look "half working".
*Enforced by:* `exit_hairpin_has_exactly_one_route_for_google_domains_and_no_fallback`.

**INV-7 — No third-party egress services.** Cloudflare WARP is **forbidden**
(owner, 2026-10-11 — likely a Cloudflare terms issue for a multi-user
service). Do not use it, even to test. No residential/mobile proxy services,
no additional VPS, no new services without the owner's explicit approval.
The only egress nodes are the two already operated.

**INV-8 — The hairpin target is the owner's existing Selectel node.** The
owner accepted routing Google/YouTube through it after first objecting to a
Russian exit. Do not move this traffic to another provider or country without
approval. Measured cost: new QUIC sessions take ~320 ms instead of ~130 ms
(an extra REALITY connection per UDP session over the hop).

**INV-9 — The hairpin credential is dedicated and never printed.** Use a
dedicated user (`hairpin-<exit>`), never a customer's. Move the UUID between
nodes only via `--uuid-stdin`; never in argv, logs, commits, tickets or chat.
Rotate it by creating a new user, re-running `google-egress-hairpin set`, then
removing the old one.

**INV-10 — The target node must not restart sing-box.** A restart drops every
connection, including the hairpin's, so playback stutters. On `ru1` the
provisioning agent is **stopped and disabled**: with the control plane
unreachable (since 2026-10-06) it rotated all lease slots every 20 minutes,
each restarting sing-box. Do not re-enable it (`systemctl enable --now
vpn-provisioning-agent`) while the control plane is down. The hairpin user is
static with no expiry; do not give it an expiry or a native lease.

**INV-11 — Never hand-edit a rendered config on a node and leave it.**
`vpn-expiry-reconcile.timer` re-renders every 10 minutes, reverts the edit and
restarts sing-box. Change behaviour through the renderer and
`vpn-admin`. If you must hand-patch for a test, stop that timer, record the
rollback, and restore it afterwards.

**INV-12 — Acceptance is the native app.** "Fixed" means Shorts and ordinary
videos play in the native iPhone YouTube app through Hiddify, on both
protocols, over a sustained session, after a reconnect. A browser result,
`curl`, a green test suite or `playabilityStatus: OK` are **not** acceptance
(that status is `OK` even when Shorts are refused).

## Re-verifying quickly (no phone needed, gate only)

`deploy/lib/youtube_shorts_probe.py` loads consecutive Shorts in Chrome with iPhone
emulation through a SOCKS proxy and reports whether each one actually plays.
To test whether an **IP** is accepted without any VPN involved, forward plain
TCP through it (`ssh -D` or any SOCKS-over-SSH) and point the probe at that
port. An accepted IP plays 6/6; a refused one shows 0/6 with "unavailable".
Run it against: your own line (control), the exit IP, the paired node IP, and
through the real VLESS+REALITY tunnel with the hairpin on.

## Deploy and roll back

Deploy (exit side; `<relay-*>` are the target's public REALITY metadata):

```
# on the target node: dedicated static user, no expiry
vpn-admin user create --name hairpin-<exit>
# on the exit, UUID piped on stdin, never printed:
vpn-admin google-egress-hairpin set --relay-host <host> --relay-port 443 \
  --relay-server-name <handshake-server> \
  --relay-reality-public-key <public.key> --relay-reality-short-id <short_id> --uuid-stdin
vpn-admin doctor --protocol
```

Roll back: `vpn-admin google-egress-hairpin clear` on the exit (every user is
affected immediately; Shorts return to failing). Pre-change backups of
2026-10-11 are in `/root/arcana-backup-20261011-*` on both nodes.

## Known gaps — do not paper over

- The exit currently runs release v1.1.0 binaries plus a **hand-patched** live
  config (IPv6 reject rule) with `vpn-expiry-reconcile.timer` stopped. Until a
  release containing this commit is deployed, a reboot restores the timer and
  re-renders without the IPv6 reject (Shorts still work; literal-IPv6 flows
  stall until the client falls back). Deploying the release removes the hand
  patch (INV-11).
- If YouTube starts refusing the Selectel IP too, the fix fails with no
  automatic fallback. Detect it with the probe; the remedy is an owner
  decision (INV-7/INV-8), not a code change.
- The hairpin is a single point of failure for all Google/YouTube/Gmail-API
  traffic of every user on the exit.
