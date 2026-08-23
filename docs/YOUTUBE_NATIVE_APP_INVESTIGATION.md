# YouTube native-app failure vs. Safari success vs. WARP success (2026-08-20)

> **REMOVAL NOTICE (provisioning-contract pass).** References below to
> the `format=xray` A/B mechanism describe a **removed** feature; it
> changed only a share-link label. §9.5a's own finding — that Hiddify
> selects Xray-core solely via an explicit `core=xray`/`xvless://`
> import syntax this project never emitted — is why. The Vision-off
> experiment (§9.5) is unaffected and remains available, now also as
> `?diagnostic=vision-off` on the versioned provisioning contract. See
> `docs/PROVISIONING_CONTRACT.md`.

Investigation only — no fix implemented yet. Per the task that opened
this pass: "do not optimize until the packet path is understood," and
"do not claim success until the native application works on a real
device." Nothing here proposes changing REALITY keys/SNI, DNS, MTU,
BBR/sysctls, Hysteria2 parameters, firewall rules, or the server
transport. Exactly one experiment is proposed to run next (section 9).

**Evidence classification:** the Safari/native-app/WARP behavior below is a
USER-REPORTED historical observation, not DEVICE-VERIFIED. It has no complete
dated record containing device, OS, client version, network, transport, and
commit. All resulting root-cause explanations are HYPOTHESES. Current
acceptance status is recorded only in `docs/DEVICE_ACCEPTANCE_TESTS.md`.

## The question

> Why does Safari play YouTube through singbox-vpn, the native YouTube
> iOS app does not, and Cloudflare WARP on the same phone makes the
> native app work?

TikTok is a second, separately-diagnosed native-app failure — see
`docs/TIKTOK_INVESTIGATION.md`. This document does not force TikTok's
cause onto YouTube's or vice versa; §10 below states explicitly what
methodology carries over and what doesn't.

## 1. What the repository already contains (read before re-deriving anything)

- `crates/compat-config/src/render.rs` / `services/subscription/src/lib.rs`:
  the subscription renderer and its HTTP service. Confirmed by reading
  both, not assumed:
  - `?compat=tcp-only` (`CompatibilityMode::TcpOnly`) is real, already
    shipped, already tested (`render.rs`'s `tcp_only_mode_*` tests,
    `services/subscription/src/lib.rs`'s
    `compat_tcp_only_with_singbox_format_returns_200_and_tcp_only_json`).
    It does exactly two things: drops Hysteria2 from the offered
    outbounds, and sets `"network": "tcp"` on the VLESS+REALITY
    outbound. Nothing else. `format=uri`/`format=hiddify`/`format=xray`
    reject `compat=tcp-only` with 400 rather than silently ignoring it.
  - The generated subscription **never** contains `route.rules`, a `dns`
    block, `inbounds`, MTU, mux, or fragment settings — enforced by
    `client_subscription_never_emits_route_rules` and
    `client_subscription_profile_shape_is_explicit_and_minimal` in
    `render.rs`'s test suite. This is a **repository-level guarantee**,
    not a design note that could have drifted.
- `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`: investigated a client-side
  `route.rules` UDP/443-reject rule for this exact symptom and
  **deliberately did not ship it**, because (as of that investigation)
  this project could not verify Hiddify iOS actually preserves an
  imported `route.rules` array. That reasoning is re-examined, not
  re-asserted, in §6 below — new information changes part of it.
- `docs/CLIENT_PROTOCOL_BEHAVIOR.md`: the authoritative "what singbox-vpn
  controls vs. what the client controls" table. Restated precisely where
  relevant below, not re-derived.
- `docs/clients/HIDDIFY_IOS.md`: already documents this exact symptom
  ("Native YouTube app fails while Safari plays YouTube fine") and
  already tells the user how to test `tcp-only` — but explicitly labels
  it **"not a proven fix... has not yet been confirmed against a real
  device."** No device data exists yet in this repository establishing
  whether `tcp-only` actually fixes the symptom. This is the single most
  important gap this investigation exists to close.
- `docs/FAILURE_CLASSIFICATION.md`: describes `crates/network-state` /
  `crates/failure-classifier`'s `FailureCategory` state machine. **Not
  applicable here** — that machinery belongs to the native
  `client-daemon`/`transport-native` stack (`docs/ARCHITECTURE.md`),
  which Hiddify/sing-box clients never touch. Hiddify's failure
  handling is entirely internal to Hiddify/sing-box-core, invisible to
  this repository. Flagging this explicitly so it isn't mistaken for
  relevant machinery later.
- `docs/RUSSIA_PRODUCTION_INVESTIGATION.md`: a *different* investigation
  (REALITY handshake failures reported by real Russian clients,
  `processed invalid connection`) — relevant only insofar as it already
  established the `format=xray` A/B mechanism (§12 below reuses it) and
  the general evidence-labeling convention (FACT/INFERENCE/UNKNOWN) this
  document also follows. It is not the same failure and its findings are
  not assumed to transfer.
- `rg` across the repo for `tcp-only|udp|quic|http3|443|youtube|tiktok|
  dns|ipv6|route|reject|sniff|xray|hiddify|warp` confirms: **no code
  anywhere references WARP**, no code path does per-application routing
  by domain/SNI, and the only QUIC/UDP-aware mechanism outside sing-box
  itself is `tcp-only`.

**What the repository controls vs. what Hiddify/iOS controls** (restating
`docs/CLIENT_PROTOCOL_BEHAVIOR.md`, not re-deriving it): singbox-vpn
generates `outbounds` + `route.final` only. DNS, full-device tunnel
capture, IPv4/IPv6 route preference, MTU, kill-switch behavior, and
which sing-box core build actually executes the config are **100%
Hiddify + iOS**, invisible to and unverifiable from this server. Every
hypothesis below that lives in that territory is marked UNKNOWN, not
DISPROVED, because the repository has no way to observe it directly —
only a real-device test can.

## 2. Architecture: application traffic path per transport

```
VLESS+REALITY (the profile that matters most — REALITY is TCP):

  iPhone app (YouTube)
        │ generates its own TCP and/or UDP/443 sockets
        ▼
  Hiddify TUN (NEPacketTunnelProvider, iOS) ── captures traffic per
        │                                       Hiddify's own route/DNS
        │                                       policy — NOT expressed
        │                                       by singbox-vpn's config
        ▼
  Hiddify's bundled sing-box (or Xray?) core
        │ VLESS outbound: TCP/443 to the VPS, TLS+REALITY handshake
        │ App UDP is normally relayed AS UDP-OVER-THE-SAME-TCP-CONNECTION
        │ (sing-box's standard VLESS UDP relay, sometimes called xudp) —
        │ REALITY's own transport is TCP; that does NOT mean application
        │ UDP inside it is dropped by default. It means app UDP is
        │ multiplexed onto the same TCP stream, not sent as a separate
        │ UDP/443 client→VPS flow.
        ▼
  VPS: sing-box VLESS inbound (REALITY)
        │ de-relays the app's UDP payload
        ▼
  VPS's own OS network stack
        │ opens a REAL UDP/443 socket to Google/YouTube's real IP
        ▼
  Internet (Google/YouTube CDN)

Hysteria2 (UDP/443 end-to-end by design — not this investigation's
first target per the task's own instruction to use REALITY first,
listed for completeness):

  iPhone app → Hiddify TUN → sing-box Hysteria2 outbound (QUIC/UDP/443
  to VPS) → VPS Hysteria2 inbound → VPS UDP/443 to Internet

?compat=tcp-only (REALITY only, "network": "tcp" on the VLESS outbound):

  iPhone app → Hiddify TUN → sing-box VLESS outbound, UDP relay DISABLED
  for this outbound → ??? (exactly what happens to an app's UDP/443
  attempt here is the open question in §6 — this is not "UDP becomes
  TCP", it is "UDP relay through this one outbound is refused")

WARP (Cloudflare's own WireGuard-based client — no singbox-vpn code
involved at all):

  iPhone app → Cloudflare's own NEPacketTunnelProvider (WireGuard) →
  Cloudflare's own DNS (typically 1.1.1.1 through the tunnel) →
  Cloudflare's own edge → peering path to Google, NOT this VPS's
  provider/ASN/route at all.
```

The REALITY path above is the key structural fact this investigation
turns on: **application UDP inside VLESS+REALITY is not automatically
"blocked" just because REALITY's own transport is TCP.** Whether it
actually gets relayed, and whether the relayed packets actually reach
Google, are separate, measurable questions — not something safe to
assume either way. This is exactly why §9's Phase-1 experiment is
ranked first.

## 3. Root-cause tree with current evidence

Every hypothesis is SUPPORTED / DISPROVED / UNKNOWN as of today — no
device-side measurement has been run yet for this specific YouTube-app/
WARP comparison, so most of this section is UNKNOWN. That is the honest
starting point, not a gap to paper over.

### 3.1 Application QUIC/UDP behavior (YouTube app prefers/requires QUIC)

Status: **UNKNOWN**. Plausible, industry-consistent (YouTube's app is
known generally to use QUIC/HTTP3 for parts of its traffic), but this
repository has no verified measurement of whether the *specific* failing
connections are QUIC attempts, whether YouTube's app falls back to TCP
cleanly when QUIC fails, or whether it hard-fails instead. `docs/
COMPATIBILITY_QUIC_EXPERIMENT.md` already treated this as a hypothesis,
not a proven cause — correctly. This document does not elevate it either.

### 3.2 UDP relay failure inside sing-box's VLESS/REALITY implementation

Status: **UNKNOWN**. Distinct from 3.1: even if the app does attempt
QUIC, the failure could be that Hiddify's bundled sing-box core fails to
relay it correctly (a client-core bug), not that QUIC itself is
unreachable. §12 (client isolation) is the only way to separate these.

### 3.3 VPS outbound UDP failure (egress to Google, not the tunnel itself)

Status: **UNKNOWN**, and currently **untested** — this is precisely
what §9's Phase-1 experiment measures. Even a perfectly healthy REALITY
tunnel and a perfectly healthy sing-box relay could still fail here if
the VPS's provider filters or degrades outbound UDP/443, which "the
tunnel works, browsing works" does not rule out (ordinary HTTPS
browsing is TCP; it exercises a completely different code/network path
than relayed application UDP).

### 3.4 Hiddify TUN capture/routing behavior specific to the YouTube app

Status: **UNKNOWN**. `docs/clients/HIDDIFY_IOS.md`'s "Four different
claims" table already establishes that "connected" in Hiddify's UI does
not prove full-device capture. A native app could plausibly use
different endpoints, connection pooling, or Multipath/QUIC-connection-
migration behavior than Safari that interacts differently with
Hiddify's TUN — untested.

### 3.5 Hiddify's own sing-box-core build/version/bug

Status: **UNKNOWN**. `docs/clients/HIDDIFY_IOS.md` already flags
Hiddify's iOS release pipeline has open, confirmed problems
(hiddify/hiddify-app#2317) as of 2026-08, and that Hiddify's bundled
sing-box core is "confirmed several minor versions behind this
deployment's server-pinned version" (referenced from `docs/
COMPATIBILITY_QUIC_EXPERIMENT.md`/`docs/TELEGRAM_RESILIENCE_PLAN.md`).
Neither of these is proven to cause THIS symptom — they are documented
risk factors that raise the prior probability of a client-core
explanation, nothing more.

### 3.6 IPv6 routing / AAAA-first behavior

Status: **UNKNOWN**. No AAAA/A comparison has been captured yet for the
actual destinations the YouTube app contacts during a failure (as
opposed to guessed Google domains). §9 does not test this first because
Safari already succeeds over whatever this deployment's real IPv4/IPv6
posture is — weak evidence against IPv6 as the sole explanation, not
proof against it (native app and Safari could resolve/route
differently). Ranked below QUIC/UDP-relay/egress in §8.

### 3.7 DNS resolution/behavior differences (app vs. Safari vs. WARP)

Status: **UNKNOWN**. `docs/CLIENT_PROTOCOL_BEHAVIOR.md` already states
the generated config has no `dns` block — DNS is entirely Hiddify's
call. WARP is known to typically force its own DNS (1.1.1.1-family)
through its own tunnel — a real, measurable difference from whatever
Hiddify does by default. Untested against the actual phone.

### 3.8 Exit-IP / ASN / Google CDN peering differences

Status: **UNKNOWN**, but genuinely plausible and cleanly separable from
everything else — WARP changes the exit IP/ASN entirely (Cloudflare's
own network, direct peering with Google in most regions) versus this
VPS's hosting provider. This is exactly what §9's later same-VPS
WireGuard/Outline control (Phase 6, §9.6) is designed to isolate from
transport-layer causes. Not testable from this session (no live VPS
access) — documented as the next operator-run experiment.

### 3.9 Split tunneling / route coverage gaps

Status: **UNKNOWN**. Whether every YouTube-app destination is actually
captured by Hiddify's TUN during a failing attempt is unverified — a
native app may hit different endpoints (dedicated video-serving pools)
than Safari. `docs/clients/HIDDIFY_IOS.md` already documents this as an
open unknown for the general case; nothing new establishes it for
YouTube specifically.

### 3.10 Stale application connection/session state

Status: **UNKNOWN going in, but directly controlled for.** §9's reset
procedure (force-quit YouTube, reconnect VPN, fresh launch) exists
specifically so this variable cannot masquerade as any of the others.
Any result gathered without following that procedure must be discarded,
not partially trusted.

### 3.11 MTU / PMTUD

Status: **Deliberately out of scope for the first experiment**, per the
task's own instruction and this project's established convention
(`docs/CLIENT_PROTOCOL_BEHAVIOR.md`: no MTU override "unless a
reproducible failure demonstrates otherwise"). Only prioritized if §9's
results show a large-transfer-specific stall pattern (thumbnails/API
load, video starts then stalls) rather than an immediate failure.

## 4. Application-behavior breakdown (not just "works/fails")

To be filled in by the real-device test — see the row-filling template
added to `docs/DEVICE_ACCEPTANCE_TESTS.md`'s YouTube-specific record.
The dimensions that matter, restated from the task and now made
explicit as required fields, not optional notes:

**YouTube**: app launches / home feed loads / thumbnails load /
account-avatar loads / comments load / video metadata loads /
advertisement starts / video starts / video buffers indefinitely /
audio starts / seeking works / next-video works.

**TikTok** (for the follow-up pass, see `docs/TIKTOK_INVESTIGATION.md`
§4 for its own already-published procedure): app launches / feed
metadata loads / profile images load / comments load / video thumbnail
loads / video bytes load / scrolling to next video works / login works.

A failure where APIs/metadata succeed but only media-CDN bytes fail is
a different finding than total connectivity failure — both YouTube's
and TikTok's write-ups must never collapse this distinction into
"doesn't work."

## 5. WARP as a scientific control — what to actually record

WARP working does not by itself prove any specific cause among §3's
UNKNOWNs — it only proves *something* about singbox-vpn's path differs
in a way that matters to the YouTube app. The variables WARP changes,
all simultaneously, that must be recorded and reasoned about
individually rather than credited to "WARP" as a whole:

| Variable | How to check on each VPN | What it isolates |
|---|---|---|
| Public IPv4 | `curl -4 ifconfig.me` (or any what's-my-ip page) with each VPN connected | Exit-IP/ASN hypothesis (§3.8) |
| Public IPv6 | Same, IPv6-specific checker; also whether the field is even populated | IPv6 hypothesis (§3.6) |
| DNS resolver in use | An on-device DNS-leak-test page/app while each VPN is connected | DNS hypothesis (§3.7) |
| IPv6 availability | Does the phone have real IPv6 connectivity at all before connecting? | Baseline for §3.6 |
| YouTube app result | Full §4 breakdown, not pass/fail | The actual target symptom |
| Safari YouTube result | Full §4 breakdown | Control — previously USER-REPORTED PASS; reproduce it |
| TikTok result | `docs/TIKTOK_INVESTIGATION.md` §4 breakdown | Whether WARP also fixes TikTok (tells us if the two failures share a cause) |

Record this full table for **both** singbox-vpn (REALITY, manually
selected) and WARP, same phone, same network, same reset procedure
(§9.7 below). Whichever rows differ between the two runs are the actual
candidate explanations — rows that are identical on both VPNs are
eliminated as explanations for why one works and the other doesn't.

## 6. Critical re-examination of `?compat=tcp-only`

### 6.1 What it verifiably does today

Confirmed by reading `crates/compat-config/src/render.rs` directly (not
inferred from the docstring): sets `"network": "tcp"` on the VLESS
outbound JSON, and omits Hysteria2 from `outbounds` entirely. That is
the complete, total effect on the generated config. No `route.rules`,
no `packet_encoding`, no DNS, no MTU — the test suite locks this in.

### 6.2 The open question: what happens to an app's UDP/443 packet under `network: tcp`?

This is genuinely unresolved by reading sing-box's docs alone and
requires either sing-box source inspection or a real-device/real-client
test — not asserted here without that evidence. The candidate outcomes,
which are **not equivalent** and matter for whether YouTube's app can
retry over TCP quickly or hangs:

- The VLESS outbound simply has no UDP relay capability while
  `network: tcp` is set — an application UDP packet arriving at
  sing-box's TUN inbound with `route.final: select` pointing at this
  outbound would need to go *somewhere*. If sing-box's TUN/route logic
  requires an outbound capable of carrying UDP and none exists, the
  most likely behavioral classes are: (a) the connection attempt is
  refused immediately at the sing-box client layer (fast, clean
  failure — good for the app's own TCP-fallback logic if it has one),
  or (b) it silently blackholes until the OS/app's own timeout fires
  (slow, and looks identical to "network is just broken" from the
  app's perspective). **Which of these actually happens is unverified
  from this environment** — this project does not have a running
  sing-box client + TUN to observe it directly, and no primary-source
  sing-box documentation was found in this pass stating VLESS-outbound-
  level UDP-refusal timing/error-code behavior specifically (as opposed
  to route-rule-level `reject`, which IS documented — see §6.3).
- **This is a real gap** `docs/COMPATIBILITY_QUIC_EXPERIMENT.md` did not
  close either — it justified shipping `tcp-only` on the grounds that
  "there is no UDP outbound left to fall back to," which is correct
  about the *config*, but does not establish what failure signal that
  produces for the app. This document does not claim the earlier
  decision was wrong; it identifies precisely what was left unverified.

### 6.3 What IS verified (2026-08, primary source): sing-box `route.rules` reject semantics

From `sing-box.sagernet.org/configuration/route/rule_action/`
(confirmed current as of this pass): a `{"action": "reject", ...}` rule
has a `method` field —

- `method: "default"` (the default if unspecified) — for TCP: replies
  with a TCP RST; **for UDP: replies with ICMP port-unreachable.** This
  is a fast, explicit failure signal.
- `method: "drop"` — silently drops the packet(s). This looks like a
  dead network to the application until its own timeout fires — a much
  worse experience even if the underlying "don't let this reach the
  VPS/Internet" goal is the same.
- A documented safety feature: if `no_drop` is not set, sing-box
  auto-escalates `default` to `drop` after 50 triggers in 30 seconds
  (abuse/flood protection) — meaning a `route.rules` UDP/443-reject
  rule's behavior could silently change from "fast ICMP unreachable" to
  "silent drop" under load, which is itself a confound worth knowing
  about before trusting a single manual test as representative.

This distinction (`method: default` vs. `drop`) is exactly the "explicit
QUIC failure that make the app retry over TCP" vs. "UDP just vanishes"
split the task asks about in §6/§7. It only applies to a `route.rules`
rule, though — **not** to `tcp-only`'s `network: "tcp"` outbound
property, whose failure-signal behavior remains the open question in
§6.2.

### 6.4 Re-examining why `route.rules` was rejected — partially outdated

`docs/COMPATIBILITY_QUIC_EXPERIMENT.md`'s core objection was: this
project cannot verify Hiddify iOS preserves an imported `route.rules`
array, because no real Hiddify install was available to check. A 2026-08
web search (supporting evidence, not a primary-source guarantee) found
that **Hiddify Next (a current Hiddify build line) now ships an in-app
Route Rules editor UI** (hiddifynext.app's routing-rules guide). This
does not resolve the original objection — an in-app rule editor is not
the same as "imports and executes a `route.rules` array pasted into a
raw JSON config unmodified" — but it is new information suggesting
Hiddify's routing-rule handling may be more mature than it was when
`COMPATIBILITY_QUIC_EXPERIMENT.md` was written, and worth re-checking
against the actual installed build before treating the original
rejection as still fully current. **This document does not reverse that
decision** — it flags it for re-verification as part of §9's Phase-2/
§12's client-isolation work, using the ACTUAL Hiddify app/build on the
affected phone, not a general claim about "Hiddify" as a name.

### 6.5 The A/B/C test the task specifies

```
A — normal REALITY (no compat parameter)
B — ?compat=tcp-only (already shipped)
C — REALITY + a manual, operator-pasted route.rules UDP/443 reject rule
    (method: default, for the fast/explicit-failure signal) — ONLY IF
    the installed Hiddify build's raw-config/custom-JSON import path can
    actually be confirmed to accept and execute it; if it cannot, C is
    not runnable on that device, full stop, not approximated.
```

C must never be auto-generated or shipped from `services/subscription`
without exactly the same verification bar `COMPATIBILITY_QUIC_EXPERIMENT.md`
already set (§6.4 explains why that bar is being re-checked, not
lowered) — it stays a manual, single-device, clearly-labeled experiment
per that document's "Safe alternative" section, unchanged.

Interpretation, exactly as the task specifies:

- A fails, B fails, C works → `tcp-only`'s `network: tcp` approach is
  insufficient; an explicit, fast UDP failure signal (not just "no UDP
  outbound") is what actually lets the app recover. Strong candidate
  fix: ship the reject-rule approach instead of/alongside `tcp-only`,
  contingent on resolving §6.4's Hiddify-preservation question for real.
- A fails, B works, C works → UDP/QUIC strongly implicated in general;
  `tcp-only` is already sufficient — no further QUIC-fallback work
  needed, `docs/clients/HIDDIFY_IOS.md`'s existing guidance can be
  upgraded from "unproven" to "confirmed" once documented.
- A fails, B fails, C fails → QUIC/UDP is not the explanation (or not
  the whole explanation). **Stop adjusting QUIC/UDP handling** and move
  to §9's other phases (egress, IPv6, DNS, client isolation, exit-IP).

## 7. Server-side outbound UDP audit (read-only, nothing changed)

Not run in this pass (no live VPS in this session) — documented here as
exactly what the operator must run, mirroring the task's own ordering:

```bash
# Firewall/security posture — read-only, no rule added or removed:
sudo firewall-cmd --list-all      # if firewalld
sudo nft list ruleset             # if nftables
sudo iptables -S                  # if iptables
# Plus whatever the hosting provider's own security-group/firewall
# console shows for outbound UDP/443 — this is NOT visible from the VPS
# itself if the provider filters upstream of the host's own NIC.

# curl's actual HTTP/3 support — verify before trusting any --http3 result:
curl -V | grep -i http3           # do not assume; check the compiled feature list
```

`deploy/lib/vpn-investigate.sh youtube` (pre-existing) already performs
exactly this "does curl on this host have real HTTP/3 support" check
before attempting any QUIC test (`youtube_quic()`'s `curl --version |
grep -qi HTTP3` guard) and honestly reports "not tested — tooling gap"
rather than a false PASS/FAIL when it doesn't. Re-run it as part of §9's
Phase-1 experiment; no new server-side diagnostic was needed for this
question since the existing tool already asks it correctly. If the
installed curl lacks HTTP/3, do not install a random package to force
one — document the gap and rely on the packet-capture-based Phase-1
experiment instead (§9), which does not need a QUIC-capable client at
all, only tcpdump.

This audit determines "is sing-box/the VLESS relay broken" vs. "this
VPS/provider simply cannot exchange QUIC with the Internet reliably" —
two very different root causes with very different fixes (§11).

## 8. Decision tree

```
YouTube app fails
│
├── Phase 1 (§9.1): does application UDP/443 leave the VPS during a
│   failing attempt (correlated by timestamp with the client's REALITY
│   tunnel traffic)?
│   │
│   ├── NO UDP/443 egress observed at all
│   │   ├── §6.5 test C (explicit reject) changes the app's behavior
│   │   │      → QUIC-fallback/client-routing problem (§3.1/§3.4/§3.9)
│   │   └── test C does not change it
│   │          → inspect TUN/sing-box-core/DNS/IPv6 (§3.2/§3.5/§3.6/§3.7)
│   │
│   └── UDP/443 egress observed
│       ├── no replies come back
│       │      → VPS/provider/firewall/upstream UDP problem (§3.3)
│       └── replies come back (bidirectional)
│              → inspect relay/TUN/session/MTU behavior (§3.2/§3.4/§3.11)
│
├── Force-IPv4 (§9.3) fixes it → broken IPv6 path (§3.6) — fix THAT,
│   not by keeping tcp-only as a permanent workaround
│
├── Controlled/tunneled DNS (§9.4) fixes it → Hiddify DNS config (§3.7)
│
├── A verified alternate client (raw sing-box / confirmed Xray-core)
│   works with the SAME REALITY credentials → Hiddify-layer bug (§3.4/§3.5)
│
├── Xray-core works, sing-box-core fails → sing-box-client/core interop
│   bug specifically (§3.5), not a server or protocol problem
│
├── Same-VPS WireGuard/Outline (§9.6) ALSO fails while WARP succeeds
│   → exit-IP/ASN/provider/Google-peering path (§3.8) — a hosting
│   decision, not a sing-box config change
│
└── Same-VPS WireGuard/Outline WORKS while singbox-vpn fails
       → exit-IP is largely eliminated; shift fully to Hiddify/sing-box/
       TUN/UDP-relay/DNS/IPv6 (§3.2/§3.4/§3.5/§3.6/§3.7)
```

## 9. Recommended experiment order (information gain first)

Exactly one experiment (§9.1/Phase 1) is proposed to run **now**; the
rest are documented in order so the next session/operator does not have
to re-derive prioritization, but they are **not** authorized to all run
in one sitting — one variable at a time, evidence recorded between each,
per the task's own rule.

### 9.1 — PROPOSED FIRST EXPERIMENT: does application UDP/443 leave the VPS? (Phase 1)

**Why**: this is the single highest-information-gain measurement listed
in the task, splits the largest branch of the decision tree (§8), and
requires no new hypothesis about Hiddify/iOS internals this project
cannot observe — it only needs packets already crossing the VPS's own
NIC.

**Files/config affected**: none in production. This pass added
`deploy/lib/vpn-investigate.sh udp-egress-capture` (read-only,
bounded-duration `tcpdump`, same safety envelope as the pre-existing
`capture`/`summarize` commands — no service, firewall, or route is
touched) plus its `--help`/validation-test coverage. A later pass added
`deploy/lib/vpn-investigate.sh udp-egress-verdict INPUT.pcap CLIENT_IP`:
`summarize` alone reports raw packet metadata and left the A/B/C/D read
to a human eyeballing a timeline — `udp-egress-verdict` reads the same
pcap and CLIENT_IP, correlates the REALITY TCP/443 tunnel window against
host-wide UDP/443 direction (outbound vs. inbound), and prints one
FACT/INFERENCE/UNKNOWN-labeled verdict, all timestamps UTC (to
correlate with `journalctl -u sing-box` output captured separately).
Also read-only, no capture of its own, no service/firewall/route touched.

**Exact server commands**:

```bash
# 1. Identify the phone's current client IP (from Hiddify's connected
#    state or `ss -tn state established '( dport = :443 )'` on the VPS
#    while the phone is connected via REALITY).
CLIENT_IP=<phone's observed source IP for the REALITY tunnel>

# 2. Start the bounded capture BEFORE opening YouTube (60s is enough
#    for the 10s test window below plus setup slack; extend if needed,
#    max 300s):
sudo deploy/lib/vpn-investigate.sh udp-egress-capture "$CLIENT_IP" \
  /root/youtube_udp_test.pcap 60
```

**Exact iPhone steps** (run the §9.7 reset procedure immediately before
this, not just "make sure it's connected"):

```
1. Disconnect Hiddify VPN entirely.
2. Force-close the YouTube app (swipe up/away, not just background it).
3. Reconnect Hiddify, REALITY transport manually selected (not auto).
4. Confirm public IP == the VPS's IP (any what's-my-ip check).
5. Wait ~5s for the tunnel to settle.
6. (Operator starts the capture command above right before step 7.)
7. Launch YouTube fresh (cold start, not resumed from background).
8. Tap one specific video previously confirmed to fail.
9. Wait exactly 10 seconds from the tap.
10. Force-close YouTube.
(Capture auto-stops at 60s, or Ctrl-C the operator's session early.)
```

**What to save**: only the resulting `.pcap` (packet metadata — no
payload beyond the first 160 bytes/packet, already enforced by the
`-s 160` snap length shared with the pre-existing `capture` command)
and the output of:

```bash
deploy/lib/vpn-investigate.sh summarize /root/youtube_udp_test.pcap
deploy/lib/vpn-investigate.sh udp-egress-verdict /root/youtube_udp_test.pcap "$CLIENT_IP"
```

**What must be redacted before sharing this anywhere**: nothing in the
pcap or `summarize` output is a secret by construction (no REALITY
private key, VLESS UUID, or Hysteria2 password ever appears — those
never touch the wire in cleartext and this capture only ever records
metadata), but the phone's public/carrier IP address and the exact
video ID/URL tapped are still meaningful identifying/behavioral data —
redact or generalize both before sharing the `summarize` output or pcap
outside the operator/admin doing the diagnosis.

**PASS/interpretation** — there is no single "PASS", only which of §8's
branches the result lands in:

- No UDP/443 packets appear anywhere in the capture during the 10s
  window → Case A (§8's "NO UDP/443 egress"): the app either never
  attempted QUIC, or Hiddify/sing-box never relayed it. Proceed to
  §6.5's A/B/C test to distinguish those.
- UDP/443 packets leave the VPS toward an external IP but nothing comes
  back → Case B: VPS/provider egress problem (§3.3/§7).
- Bidirectional UDP/443 traffic exists during the window → Case C: the
  path exists; the failure is downstream (relay/TUN/session/MTU, §3.2/
  §3.4/§3.11).
- TCP/443 tunnel traffic to `$CLIENT_IP` is present (proving the phone
  was actively using the tunnel) but genuinely no UDP/443 appears
  anywhere and playback still fails → Case D: QUIC is not sufficient to
  explain the failure by itself; do not keep treating it as the sole
  cause.

Cases A and D above share the exact same server-side observation — zero
UDP/443 packets, tunnel active — because the difference between them
("client attempted QUIC but it was never relayed" vs. "client never
attempted QUIC at all") lives entirely on the client device, which this
server cannot see. `udp-egress-verdict` therefore always reports this
combined observation as one labeled case, **"Case A/D"**, and says so
explicitly rather than guessing which one applies — do not read a
zero-UDP result as proving Case A specifically; §6.5's A/B/C reject-rule
test is what actually separates client-routing causes from a healthy
tunnel that simply carries no QUIC.

**FAIL interpretation**: if `$CLIENT_IP` cannot be identified, or the
capture shows no REALITY tunnel traffic at all during the window, the
experiment did not actually run — this is a procedural failure (wrong
IP, phone not connected, reset procedure skipped), not a network
finding. Redo it, don't record a null result as evidence.

**Rollback**: nothing to roll back — `udp-egress-capture` is read-only,
bounded, and not wired into any install/update/service path; delete the
`.pcap` file when done (it contains a phone's real IP address and CDN
destination IPs — treat it as no more sensitive than a normal traffic
log, but no reason to retain it past the diagnosis).

### 9.2 — Phase 2: QUIC fallback semantics (§6.5's A/B/C)

Only after 9.1 establishes which branch of §8 applies — if 9.1 shows
UDP genuinely never leaves the VPS (Case A), this is the next highest-
value test. If 9.1 shows bidirectional UDP already works (Case C), skip
straight to relay/TUN/session investigation instead — running §6.5
against a already-working UDP path would not be informative.

### 9.3 — Phase 3: IP family (force IPv4)

Test whether restricting Hiddify's own client-side network preference to
IPv4-only changes the app's behavior — mechanism entirely inside
Hiddify's settings, not this repository's config (§3.6). Record actual
observed AAAA/A answers for the real destinations seen in 9.1's capture,
not guessed Google domains.

### 9.4 — Phase 4: controlled/tunneled DNS

Compare DNS resolver/leak behavior across singbox-vpn / WARP / VPN-off
(§5's table). If iOS has no way to isolate "WARP's DNS + singbox-vpn's
transport," state that limitation plainly rather than forcing a test
that isn't possible on the platform.

### 9.5 — Phase 5: client implementation isolation (§12)

Compare Hiddify's bundled sing-box core against a verified alternate
client on the SAME REALITY credentials. `format=xray`
(`services/subscription`, already shipped for the Russia REALITY
investigation) exists as a labeled A/B share-link, but — exactly as the
task warns — **the label alone does not prove Hiddify's engine
selection actually changed**; this must be confirmed by checking
Hiddify's own UI/settings for which core is actually executing before
trusting any A/B result from it.

### 9.5a — Phase 5b: XTLS Vision on vs. off (`compat=vision-off`)

A second client-layer isolation that changes only ONE variable: whether
VLESS requests the `xtls-rprx-vision` flow. Vision alters VLESS's
TLS-layer behavior (its padding/splitting, and it carries relayed UDP
only as XUDP), so it is a plausible — and until now untestable —
suspect. This is materially different from `compat=tcp-only`, which
removes UDP relay entirely; `compat=vision-off` keeps every transport
and every other field exactly as production.

Shipped as of this pass (previously the "implementation gap" this
section called out), opt-in and additive:

```
tester link:  https://<host>:<port>/sub/<token>?format=hiddify&compat=vision-off
              (?format=singbox&compat=vision-off also supported;
               ?format=xray&compat=vision-off is rejected — two A/B
               variables in one link would be unattributable)
server side:  vpn-admin user vision-off-experiment <user-id>
                            [--off to revert]
```

**Finding — the server side is NOT neutral here.** sing-box's VLESS
server compares the client's requested flow with the flow configured for
that user and rejects any difference:

```go
// sing-vmess vless/service.go (sing-box v1.13.19 pins
// sing-vmess v0.2.8-0.20250909125414-3aed155119a1)
userFlow := s.userFlow[user]
if request.Flow == FlowVision && request.Command == vmess.NetworkUDP {
    return E.New(FlowVision, " flow does not support UDP")
} else if request.Flow != userFlow {
    return E.New("flow mismatch: expected ", flowName(userFlow),
                 ", but got ", flowName(request.Flow))
}
```

There is no "accept either flow" setting, and the inbound's user map is
keyed by UUID (one flow per entry), so the same UUID cannot be offered
both ways at once. The experiment is therefore per-user and mutually
exclusive: while a user is opted in, their inbound entry renders with an
empty flow and ONLY the Vision-off link works for them; every other user
keeps `xtls-rprx-vision` untouched. Nothing else changes — same VPS,
same exit IP, same REALITY keys/SNI/short ID, same ports, same routing,
same DNS, Hysteria2 unchanged.

Cost, to state plainly: Vision is what conceals the TLS-in-TLS pattern
of a proxied TLS session, so a Vision-off profile is more
fingerprintable to DPI. This is a time-boxed diagnostic and must never
become a default; revert with `--off` when the run is finished.

**Do not assume "Vision blocks QUIC" going into this experiment — that
claim is core-specific, and the wrong core for sing-box.** Source-level
research (Xray-core `proxy/vless/outbound/outbound.go`, main branch;
sing-vmess `vless/client.go`/`vless/service.go` at the exact commit
sing-box v1.13.19 pins) found:

- **Xray-core's `xtls-rprx-vision` intercepts and rejects application
  UDP/443 by design** — `"XTLS rejected UDP/443 traffic"`, documented at
  https://xtls.github.io/en/config/outbounds/vless.html, specifically to
  force QUIC-preferring apps onto TCP. Xray-core's own docs describe a
  *separate* flow value, `xtls-rprx-vision-udp443`, for when that
  interception is NOT wanted.
- **sing-box's `xtls-rprx-vision` does not reproduce this.** No
  port-443/QUIC special case exists anywhere in sing-box's/sing-vmess's
  VLESS outbound or Vision implementation as of the pinned version — a
  full-tree search of the `v1.13.19` checkout for any UDP/443-rejection
  logic in the Vision path found nothing. sing-box's Vision is
  functionally closer to Xray's `-udp443` variant than to plain
  `xtls-rprx-vision`.
- sing-box's VLESS server does reject Vision combined with the *raw*
  UDP relay command (`vmess.NetworkUDP`) — see the `service.go` snippet
  above — but allows Vision combined with `CommandMux` (XUDP, sing-box's
  default packet encoding, which this project's generated config always
  uses since it never sets `packet_encoding`). So even on sing-box-core,
  UDP relay through a Vision-flow VLESS outbound is not blocked by
  default.
- Hiddify iOS defaults to its sing-box-based core; Xray-core is opt-in
  only, via an explicit `core=xray`/`xvless://` selection
  (hiddify-app v2.0.4+ release notes) that this project's generated
  links never set.

**Consequence for interpretation**: if Vision-off (this experiment)
fixes the YouTube-app symptom on a normal, sing-box-core Hiddify
install, the mechanism is *not* "Vision was blocking QUIC" — that
mechanism doesn't exist in sing-box. It would have to be something else
Vision changes (its TLS-layer padding/splitting, or its XUDP-only
handling of relayed UDP) — investigate which, don't default to the
Xray-specific explanation. Conversely, if the *actual* core in use turns
out to be Xray (verify per §9.5, don't assume), the Xray UDP/443
interception mechanism becomes directly applicable and is the more
likely explanation. Which core is actually active is therefore one of
the highest-value facts to establish before drawing any conclusion from
this experiment's result — see §13.3 for why this matters again for
Russia-side testing specifically.

### 9.6 — Phase 6: same-VPS control protocol (WireGuard/Outline)

The highest-value remaining control after 9.1: deploy a temporary
WireGuard or Outline endpoint on the SAME VPS (same exit IP/ASN),
without touching singbox-vpn's production config, and compare YouTube-
app behavior through it against both singbox-vpn and WARP. This
directly separates §3.8 (exit-IP/ASN) from every Hiddify/sing-box-layer
hypothesis — see §8's decision-tree branches for both outcomes.

### 9.7 — Reset procedure (mandatory for every phase above, not optional)

```
1. Disconnect VPN.
2. Fully force-close YouTube (and TikTok, if testing that too).
3. Connect the specific profile/transport under test.
4. Verify public IP changed to the expected exit.
5. Wait ~5s for the tunnel to stabilize.
6. Start capture/logging if this phase uses it.
7. Launch the app fresh (cold start).
8. Test the same video/content each time.
9. Record the full §4 breakdown, not pass/fail.
10. Force-close the app before the next experiment.
```

Add an airplane-mode cycle or device restart as an explicit extra
variable only if results are inconsistent across repeats of the SAME
configuration — never mix a "warm" comparison VPN session against a
freshly reset one.

## 10. TikTok — same methodology, not the same conclusion

TikTok's own investigation and reproduction procedure already exist in
full (`docs/TIKTOK_INVESTIGATION.md`) and were written independently of
this document. What carries over from this pass: the §9.7 reset
procedure, the "control-plane vs. media-CDN" distinction (§4 here / §2E
there), and — if §9.1-9.2 here identify QUIC/UDP-relay as YouTube's
actual cause — that `docs/TIKTOK_INVESTIGATION.md`'s hypothesis A (QUIC/
UDP for TikTok) becomes worth testing with the same `?compat=tcp-only`
A/B, since it's already a candidate there too. What does **not** carry
over automatically: `docs/TIKTOK_INVESTIGATION.md` already identifies a
YouTube does not share — TikTok's own Russia service policy /
exit-IP-ASN reputation — as its top-ranked hypothesis, which has no
YouTube equivalent (Google does not self-restrict YouTube for Russia).
Do not force one explanation onto both; run YouTube's Phase 1 first
(highest information gain, cleanest control via WARP), then decide
whether the same root cause plausibly explains TikTok or whether its
own investigation's hypothesis G dominates instead.

## 11. Proposed fixes IF each hypothesis is proven — not implemented yet

Exactly as the task requires: what the correct fix would be, contingent
on which hypothesis 9.1 onward actually proves. None of these are
implemented by this pass.

```
root cause: application QUIC never gets a fast/explicit failure signal
  (§6.5 shows C fixes it, B does not)
possible fix: ship an opt-in, verified route.rules UDP/443 reject
  profile (method: default) instead of/alongside tcp-only — contingent
  on re-confirming Hiddify's raw-config route.rules preservation on the
  actual affected build (§6.4), not assumed from tcp-only's existence

root cause: VPS/provider UDP egress is degraded or filtered (§9.1 Case B)
fix: a provider/network-path decision (different security-group rule,
  different provider, or accept degraded UDP and lean on tcp-only) —
  NOT a sing-box/Hiddify config change

root cause: broken/inconsistent IPv6 path specific to Google's CDN
fix: correct IPv6 tunneling end-to-end, or a deliberate, disclosed
  IPv4-only compatibility mode — NOT keeping tcp-only as an unrelated
  workaround for an IPv6 problem

root cause: Hiddify's bundled sing-box-core has a relay/TUN bug
  (§9.5 shows raw sing-box or Xray-core works, Hiddify does not)
fix: a Hiddify-side client/core issue to report upstream, or switching
  the recommended client for affected users — NOT a server networking
  change

root cause: exit-IP/ASN/Google-peering path specific to this VPS/provider
  (§9.6 shows same-VPS WireGuard/Outline also fails while WARP succeeds)
fix: an alternate exit IP/provider decision — NOT a protocol/transport
  tweak; changing REALITY/Hysteria2 parameters cannot fix an IP-
  reputation or peering problem
```

## 12. What was NOT done (explicitly)

No REALITY key/SNI/destination change. No DNS provider or global DNS
change. No MTU/BBR/sysctl change. No firewall rule change. No Hysteria2
parameter change. No new `route.rules` shipped in any generated
subscription (`client_subscription_never_emits_route_rules` still holds
— unchanged). No claim that `tcp-only` is proven to fix or not fix the
YouTube-app symptom — that remains exactly as unproven as `docs/
clients/HIDDIFY_IOS.md` already, correctly, says it is. The only change
in this pass is a read-only diagnostic tool (`udp-egress-capture`,
later joined by `udp-egress-verdict`, which reads an existing pcap and
prints a labeled A/B/C/D verdict instead of leaving that read to the
operator) and this document.

## 13. Russia verification protocol — confirm the Berlin fix, don't re-litigate it

This section exists for the moment a Berlin experiment (§9.1–§9.5a)
identifies which mode fixes the native YouTube app there. Once that
happens, the Russia-side question changes shape: it is no longer "what
causes this," it is **"does the Berlin-proven fix continue to work under
Russian network interference?"** Treat these as different questions with
different failure modes, and do not silently collapse them.

### 13.1 Isolate the variable Russia actually introduces

A Russia test result is only interpretable if everything except the
network path is held constant against the Berlin run that produced the
fix:

```
same generated subscription   (identical /sub/<token>?... URL/link,
                                identical compat= mode if one was used)
        +
same Hiddify build/core        (exact app version, exact core — see
                                §9.5's "client implementation isolation"
                                and §9.5a's server-side finding that
                                Vision-off requires a matching per-user
                                server opt-in; if the Russia device is a
                                different phone/app install, record that
                                explicitly as an added variable, not a
                                controlled repeat)
        +
same iPhone
        +
same reset procedure (§9.7)
```

If any of those four differ from the Berlin run, a Russia
failure/success cannot be attributed to "the network" — it could equally
be attributed to whichever of those four changed. Record all four
explicitly for every Russia attempt, the same way §5's WARP table
already requires for the Berlin/WARP comparison.

### 13.2 Do not reopen hypotheses Berlin already disproved

If Berlin established that, say, `compat=tcp-only` fixes the symptom
and `compat=vision-off` does not (or vice versa), a Russia test's job is
to confirm that same mode still fixes it under Russian conditions — not
to re-run the full §3 hypothesis tree from a blank slate. Re-opening a
Berlin-disproved hypothesis by default (e.g. re-litigating IPv6 or DNS
in Russia when Berlin already isolated the cause to the VLESS/Vision
path) wastes the highest-value information Berlin already produced and
risks attributing a Russia-specific failure to the wrong mechanism.

The correct framing for a Russia run, once a Berlin fix exists:

- **Berlin-proven fix works in Russia too** → the mechanism generalizes;
  the remaining question is whether it's safe to promote from
  EXPERIMENTAL toward a documented, still-opt-in recommendation (never a
  new default without the censorship-resistance review §9.5a already
  flags for Vision-off specifically).
- **Berlin-proven fix works in Berlin but NOT in Russia** → do not
  conclude the Berlin diagnosis was wrong. This result means Russia adds
  a *second*, independent failure mode on top of whatever Berlin's fix
  addressed — most plausibly DPI/active-probing behavior specific to
  the Russian path (§3.6/§3.7/§9.6-style exit-IP reasoning, or genuine
  network interference that has no Berlin equivalent), not a reason to
  distrust the Berlin measurement. Investigate what's *different* about
  the Russian network path itself (§9.6's same-VPS control, run from
  Russia instead of Berlin, is the next highest-value step here) rather
  than re-deriving §3 from scratch.
- **Normal profile already worked in Russia even though it failed in
  Berlin** → this would indicate the two locations are not failing for
  the same reason at all; do not force a shared explanation (§10's rule
  already applies across YouTube/TikTok, and applies identically across
  Berlin/Russia for the same app).

### 13.3 Do not assume Vision/core behavior transfers without evidence

§9.5a already establishes, from source (sing-box v1.13.19 pinned by
this deployment, `deploy/lib/versions.env`), that sing-box's
`xtls-rprx-vision` does **not** reproduce Xray-core's UDP/443-interception
behavior — that is an Xray-specific mechanism, not a sing-box one. This
holds regardless of which country the client is in; Russia does not
change which core Hiddify executes. **Do not explain a Russia-specific
YouTube result with "Vision blocks QUIC" unless packet evidence (§9.1's
`udp-egress-verdict`, run against the Russia session) or a confirmed
core identification (§9.5) from the *exact* active core in that Russia
session proves it.** If the Russia tester's Hiddify build/core is
unverified, say so explicitly (UNKNOWN) rather than inferring it matches
the Berlin session's core.

### 13.4 TikTok stays a separate track

Per §10, TikTok's own investigation (`docs/TIKTOK_INVESTIGATION.md`) is
not re-opened by a YouTube Russia result. If Berlin's YouTube testing
confirms the transport stack (VLESS+REALITY, Hysteria2, Hiddify's
sing-box core) behaves cleanly end-to-end there, that is evidence
*against* a generic sing-box/Hiddify transport bug as TikTok's
explanation — it does not prove TikTok's cause, but it further weakens
one candidate relative to `docs/TIKTOK_INVESTIGATION.md`'s
already-top-ranked hypothesis G (TikTok's own Russia service policy /
exit-IP-ASN reputation) and its interaction with Russia-specific network
filtering. See that document's §5 ranking, which this section does not
override — only reinforces the direction it already pointed.

## Sources

- [Rule Action - sing-box](https://sing-box.sagernet.org/configuration/route/rule_action/) — `reject` action `method: default` (TCP RST / ICMP port-unreachable) vs. `method: drop` (silent), and the 50-triggers/30s auto-escalation to `drop`. Cited in §6.3.
- [About Routing Rules: Limitations & Alternatives - Hiddify Next](https://hiddifynext.app/en/guides/routing-rules/) — supporting evidence only (not a primary sing-box/Hiddify-core spec) that a current Hiddify build line ships an in-app Route Rules UI. Cited in §6.4 as a reason to re-verify, not reverse, the prior `route.rules` rejection.
- [VLESS - Project X (Xray)](https://xtls.github.io/en/config/outbounds/vless.html) — documents `xtls-rprx-vision` intercepting UDP/443 and the separate `xtls-rprx-vision-udp443` flow that does not. Cited in §9.5a/§13.3.
- [Xray-core `proxy/vless/outbound/outbound.go`](https://github.com/XTLS/Xray-core/blob/main/proxy/vless/outbound/outbound.go) — the `"XTLS rejected UDP/443 traffic"` rejection path. Cited in §9.5a/§13.3.
- [sing-vmess `vless/service.go`/`vless/client.go`](https://github.com/SagerNet/sing-vmess) at the commit sing-box v1.13.19 pins (`v0.2.8-0.20250909125414-3aed155119a1`) — no UDP/443-rejection logic in the Vision path; the `flow != userFlow` mismatch check and the Vision+raw-UDP rejection quoted in §9.5a. Cited in §9.5a/§13.3.
- [hiddify-app releases](https://github.com/hiddify/hiddify-app/releases) — v2.0.4+ notes describing `core=xray`/`xvless://` as an explicit opt-in, sing-box-based core as the default. Cited in §9.5a/§13.3.
