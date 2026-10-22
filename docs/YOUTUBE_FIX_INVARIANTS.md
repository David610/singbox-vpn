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

## Status (dated historical evidence; current nodes unverified)

| Claim | Evidence | Class |
|---|---|---|
| Shorts and ordinary videos play in the native iPhone YouTube app through Hiddify with this fix | Owner confirmed "now it works" on 2026-10-11 after a guided test (Shorts incl. `sAcElROnYIE`, consecutive Shorts, one normal video). Protocol, network, session length and seeking were **not** recorded separately. | **USER-REPORTED (native iPhone, older runtime)** |
| Shorts play in Chrome (iPhone emulation) through the real VLESS+REALITY tunnel with the fix, 6/6 consecutive | `deploy/lib/youtube_shorts_probe.py`, 2026-10-11 | Browser verified |
| Google QUIC flows leave the exit through the paired node, none through the exit's own IP | tcpdump on the exit NIC: 0 UDP packets to the test Google IPs, 1718 packets to the paired node | Server verified |
| Both REALITY and Hysteria2 on the old hand-patched v1.1.0 runtime | Later maintainer-reported entry in DEVICE_ACCEPTANCE_TESTS.md says both worked on 2026-10-11; not the exact rc.5/candidate build | **User reported** |
| Exact-candidate native app, cellular, 15-minute sustained session, post-reboot | not tested for the new standalone policy | **PENDING** |

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

Consequences measured on those IPv4-refused nodes: QUIC/UDP, IPv6,
REALITY/XUDP, UFW, MTU and server-to-Google path changes did not fix that case (server-to-Google QUIC handshakes
12/12 at ~35 ms; every Initial answered within 250 ms; no ICMP errors;
UFW dropped 3 Google packets in 15 minutes). The old "CDN-edge latency /
better peering" theory (§16) is wrong: the Selectel node works because
YouTube accepts its IP, and the latency gap no longer reproduces.

On the separately reported 2026-10-12 standalone VPS, controlled browser
measurements were literal IPv4 6/6 versus literal IPv6 0/6. This is **reported
evidence**, not a reproduction here. Its IPv6 was reachable to ordinary
Google HTTPS, and sampled phone sockets did not identify a failed Short.
See [per-family record](YOUTUBE_IPV6_DIAGNOSTIC_2026-10-12.md) and
[exit policy](EXIT_IPV6_POLICY.md).

## The fix and where it ran (2026-10-11)

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
For a configured hairpin, the whole `GOOGLE_EGRESS_DOMAINS` set (`crates/compat-config/src/model.rs`)
goes through the hairpin. Do not narrow it, "optimise" it to `googlevideo.com`
only, or replace it with IP lists. Auth, player API, static assets and media
each live on a different domain; a hole in any of them breaks Shorts.
*Enforced by:* `exit_with_hairpin_configured_adds_one_outbound_and_sniff_plus_route_rule`.

**INV-2 — Match the remedy to the refused egress address/family.**
Transport, MTU, QUIC-blocking and profile tweaks did not rescue the old
IPv4-refused nodes. A separately accepted IPv4 and refused IPv6 is a distinct
case, not ruled out by those experiments. The plain-exit default remains
`allow`; scoped proxy rejection is an explicit mechanism, not guaranteed
IPv4 retry. Do not disable host IPv6 as a default. Hiddify discards imported
`route.rules`; no imported client rule is claimed as a fix.

**INV-3 — The accepted-IP hairpin is server-side and client-transparent.** A user imports the
normal subscription into Hiddify and nothing else. Never add a client switch,
a second profile or a per-user opt-in as the way Shorts work.

**INV-4 — Hairpin `sniff` stays first after C-16/probe isolation.** iOS TUN clients resolve DNS themselves and
send bare IPs; only a sniffed SNI (TLS or QUIC) lets the domain rule match.
Per-inbound `sniff` is removed in sing-box 1.13, so it must be the
`{"action":"sniff"}` route rule.
*Enforced by:* `exit_hairpin_rejects_literal_ipv6_google_destinations_without_dropping`.

**INV-5 — Keep hairpin literal-IPv6 protection and `no_drop: true`.**
The paired node's IPv4-only path historically stalled literal IPv6. Preserve
its default `sniff -> reject -> route` exactly. On a plain exit the explicit
`reject_google` policy sniffs original literal IPv6 TLS/QUIC on port 443
(100 ms bound) then rejects the shared Google domain set; `allow` stays the
default. `reject_all` is diagnostic and breaks IPv6-only literals. An explicit
hairpin `allow` is refused, preventing accidental removal of protection.
The pinned core matches original literal family, not resolved A/AAAA answers.
Sniff fills domain metadata; resolve does not translate literal addresses.
Do not substitute `ip_cidr: ::/0`. ECH/absent SNI may bypass scoped rejection.
On VLESS/Hysteria2 proxy inbounds rejection closes the flow; it does not prove
iPhone-visible ICMP or native retry. `no_drop` prevents escalation to drops.
*Enforced by:* `exit_hairpin_rejects_literal_ipv6_google_destinations_without_dropping`,
`plain_exit_ipv6_policy_is_explicit_and_scoped`, and exact doctor rule validation.

**INV-6 — Fail closed, never fall back to the refused IP.** Exactly one route
for the domain set; no `selector`/`urltest` that offers `direct` as an
alternative; `route.final = direct` only for non-Google traffic. If the paired
node is down, YouTube must fail visibly rather than silently exit from the
refused IP and look "half working".
*Enforced by:* `exit_hairpin_has_exactly_one_route_for_google_domains_and_no_fallback`.

*Also enforced end to end across state transitions (INV-1, 4, 5, 6, 9) by:*
`hairpin_invariants_survive_user_churn_reconcile_repair_and_policy_changes`
(`apps/admin/tests/cli.rs`). It walks one exit through hairpin `set`, user
create/disable, repeated reconcile (`render-config --require-applied`: config
byte-identical, no extra sing-box restart), a repair-style `init`, refused
`egress set allow`, `egress set reject_all`/`clear`, and hairpin `clear`/`set`,
and re-checks the invariants on the config sing-box would load after each step,
against a frozen domain floor that does not follow `GOOGLE_EGRESS_DOMAINS`.
Earlier Shorts fixes were lost through transitions like these, not through the
renderer's single-input unit tests.

**INV-7 — No third-party egress services.** Cloudflare WARP is **forbidden**
(owner, 2026-10-11 — likely a Cloudflare terms issue for a multi-user
service). Do not use it, even to test. No residential/mobile proxy services,
no additional VPS, no new services without the owner's explicit approval.
Only already owner-approved existing targets may be used; current node inventory is unverified.

**INV-8 — The hairpin target is the owner's existing Selectel node.** In the 2026-10-11 deployment the
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
connection, including the hairpin's, so playback stutters. In the 2026-10-11 record, `ru1`
was **stopped and disabled**: with the control plane
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

**INV-13 — Acceptance is per egress address family, not an ASN label.**
Record domain, literal IPv4 and literal IPv6 controls separately. Ordinary
HTTPS connectivity is not Shorts playback; client-to-VPS IPv6 is distinct
from VPS-to-destination IPv6 and from an ISP leak. A browser or local
explicit retry test cannot establish native iOS recovery. The standalone
policy remains BLOCKED_ON_DEVICE_EVIDENCE for full YouTube remediation.
*Enforced by:* `plain_exit_ipv6_policy_is_explicit_and_scoped`,
`real_core_literal_family_and_proxy_recovery_boundaries`, and the
[PENDING native checklist](NATIVE_IPHONE_IPV6_ACCEPTANCE.md).

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

Write the key as `--relay-reality-public-key=<public.key>`: a base64url key can start with `-`, which older
`vpn-admin` builds read as a flag (I-61).

Roll back: `vpn-admin google-egress-hairpin clear` on the exit (every user is
affected immediately; Shorts return to failing). Pre-change backups of
2026-10-11 are in `/root/arcana-backup-20261011-*` on both nodes.

## Known gaps — do not paper over

- The v1.1.0 hand patch/stopped timer was a **2026-10-11 historical state**.
  rc.5 later rendered the rule natively; the 2026-10-12 diagnostic reports a
  plain reinstall. Today's state is unverified. Never infer timer/service
  state from this record. Exact-candidate native acceptance remains PENDING.
- If YouTube starts refusing the Selectel IP too, the fix fails with no
  automatic fallback. Detect it with the probe; the remedy is an owner
  decision (INV-7/INV-8), not a code change.
- The hairpin is a single point of failure for all Google/YouTube/Gmail-API
  traffic of every user on the exit.
