# TikTok investigation (2026-08-20)

Baseline going in, restated from the task that opened this investigation
(not re-derived, not re-litigated):

- Real Russian users connect successfully.
- General browsing works.
- YouTube works, including native-app video playback.
- singbox-vpn currently outperforms Outline in real-world testing.
- The one known application-specific failure is TikTok.
- This is deliberately **not** treated as "VPN blocked in Russia" — the
  working YouTube/browsing baseline is evidence against that framing,
  not for it.

Per `docs/CLIENT_PROTOCOL_BEHAVIOR.md` and `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`,
this project already holds a deliberate boundary: the generated
subscription expresses no DNS, routing, TUN, or MTU opinion it cannot
verify a client actually honors. Nothing in this document proposes
crossing that boundary without real-device evidence. No production
config (REALITY keys/destination, DNS, MTU, congestion control, firewall,
ports, Hysteria2 parameters) is touched by this pass — see "What was
NOT done" at the end.

## 1. What already exists in this repo (context, not re-derived)

- `crates/compat-config/src/render.rs`: subscription renderer. Already
  ships `?compat=tcp-only` (`CompatibilityMode::TcpOnly`) — drops
  Hysteria2, sets `"network": "tcp"` on the VLESS+REALITY outbound. Built
  for a *different* symptom (Safari plays YouTube, the native YouTube
  iOS app does not) but structurally relevant here too — see hypothesis A.
- `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`: investigated and **rejected**
  a `route.rules` UDP/443-reject rule for the same YouTube-app symptom,
  because Hiddify's actual handling of imported `route.rules` cannot be
  verified from this environment. That reasoning applies identically to
  any TikTok-specific route rule — nothing here proposes one.
- `docs/CLIENT_PROTOCOL_BEHAVIOR.md`: the authoritative statement of what
  the generated config controls (outbounds only) vs. what is entirely
  client-owned (DNS, IPv4/IPv6 routing, TUN, MTU, kill switch). TikTok
  inherits all the same unknowns YouTube did before real-device testing
  narrowed them.
- `deploy/lib/vpn-investigate.sh youtube`: server-side DNS/TCP/QUIC
  reachability probe against YouTube/Google root domains, split
  IPv4/IPv6. This pass adds an equivalent `tiktok` subcommand (see
  below) — same shape, same honesty conventions (labeled OK/WARN/INFO,
  explicit "does not verify the app/client" disclaimer).
- No TikTok-specific code, config, domain, or test existed anywhere in
  this repository before this pass (`rg -i tiktok` was empty).
- Nothing in `crates/`, `deploy/`, or `docs/` special-cases any
  application by domain/SNI — routing is generic (`route.final` → the
  manual selector), so there is no existing rule that could be
  "accidentally" misrouting TikTok specifically (see hypothesis D).

## 2. Hypothesis matrix

Each hypothesis is classified SUPPORTED / DISPROVED / UNKNOWN with the
evidence available **today**, before any real-device TikTok-specific
test has been run. Nothing here is a config change — see the ranked
list and proposed first experiment at the end.

### A. QUIC / HTTP/3 / UDP preference

**Claim**: TikTok's app (and possibly its web client) aggressively
prefers QUIC/HTTP3 over UDP/443 for API and/or media traffic, and
something in the UDP path (Hysteria2 itself, or UDP relay through the
REALITY outbound) fails or degrades where TCP does not.

Status: **UNKNOWN**.

- TikTok is known publicly to use QUIC/HTTP3 for parts of its traffic on
  some platforms (consistent with broad industry adoption of HTTP/3 for
  video-heavy mobile apps), but this project has no verified, current
  (2026) primary-source confirmation of exactly which TikTok endpoints
  (control-plane vs. CDN) use QUIC vs. TCP, or whether TikTok's client
  falls back cleanly to TCP when QUIC fails vs. hard-failing. This is
  exactly the kind of claim `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`
  already refused to assert about YouTube without evidence — same rule
  applies here.
- The existing `?compat=tcp-only` profile (built for a different
  symptom) is a ready-made, zero-new-code way to test this: if TikTok
  starts working under `tcp-only` and still fails under the normal
  profile, that is real evidence UDP/QUIC is implicated. If it fails
  identically under both, QUIC/UDP is disproved as the sole cause.
- **Do not assume blocking UDP is the fix.** If `tcp-only` makes TikTok
  *worse* or has no effect, that disproves this hypothesis and must be
  recorded, not discarded.

### B. DNS

**Claim**: TikTok domains resolve differently or fail to resolve
correctly for the Russian client, inside vs. outside the VPN.

Status: **UNKNOWN** (server-side vantage only, so far).

- Server-side (`vpn-investigate.sh tiktok`, added by this pass) can
  confirm whether TikTok's root domains resolve and answer from the
  VPS's own vantage — that is necessary but not sufficient, since (per
  `docs/CLIENT_PROTOCOL_BEHAVIOR.md`) the subscription has no `dns`
  block: DNS resolution while connected is **entirely the client app's
  own behavior**, unverified and unverifiable from the server.
- No evidence yet either way that TikTok's real DNS answers differ
  Russia-side vs. VPS-side (this would show up as, e.g., TikTok's CDN
  handing out unreachable-from-Russia IPs even through the tunnel — a
  CDN steering/anycast quirk, not a singbox-vpn defect).
- **Do not hard-code additional TikTok domains speculatively.** The
  `tiktok` diagnostic intentionally uses only 4 well-known root domains
  (`www.tiktok.com`, `tiktok.com`, `tiktokcdn.com`, `tiktokv.com`) — real
  app/API traffic uses many rotating regional subdomains
  (`api16-normal-c-useast1a.tiktokv.com`,
  `v16-webapp-prime.tiktok.com`, `p16-sign-va.tiktokcdn.com`, and
  similar) that cannot be enumerated correctly without a real packet
  capture from an actual TikTok client session — guessing them risks
  testing domains TikTok doesn't even use for the failing traffic.

### C. IPv6

**Claim**: the client receives AAAA records for TikTok domains and
attempts a broken IPv6 path, while IPv4-only YouTube/browsing succeeds.

Status: **UNKNOWN**, but a priori **less likely than for most
hypotheses** given the existing evidence:

- `docs/CLIENT_PROTOCOL_BEHAVIOR.md` already documents that this
  deployment's IPv4/IPv6 posture is untested end-to-end on a real
  device for *any* application, TikTok included.
- However: YouTube already works, including native playback, and
  Google's own infrastructure is heavily dual-stack — if a broken IPv6
  path on this deployment were going to break something, YouTube would
  be a more likely victim than TikTok, not less. This does not disprove
  the hypothesis (TikTok's CDN could have different IPv6 rollout/steering
  than Google's), but it means IPv6 should not be assumed as the
  default explanation just because it's a classic app-specific failure
  mode.
- The `tiktok` diagnostic reports IPv4/IPv6 reachability separately, and
  `vpn-investigate.sh mtu`/manual `dig AAAA` against the TikTok CDN
  domains can establish whether they publish AAAA records at all before
  any device-side test is needed.

### D. Routing / sniffing misclassification

**Claim**: TikTok traffic is accidentally routed DIRECT, REJECT, to the
wrong outbound, or over an unsupported transport, while YouTube is
correctly proxied.

Status: **DISPROVED at the config-generation level** (structural,
verified by reading the code, not by a device test):

- `render_singbox_client_subscription*` (`crates/compat-config/src/
  render.rs`) emits no `route.rules` at all — enforced by the
  `client_subscription_never_emits_route_rules` test. There is no
  domain/IP/protocol rule anywhere in the generated config that could
  single out TikTok (or any app) for different treatment. `route.final`
  is always the manual `select` outbound, which contains only the
  REALITY/Hysteria2 endpoints, `auto`, and `direct` — TikTok traffic has
  no code path to be silently misrouted to `direct`/reject by anything
  this project generates.
- **Residual UNKNOWN**: whether Hiddify's own default sniffing/routing
  (entirely client-owned, see `docs/CLIENT_PROTOCOL_BEHAVIOR.md`) does
  something TikTok-specific regardless of what the subscription
  contains. Not verifiable without a real Hiddify install + packet
  capture on the device.

### E. Control-plane vs. CDN/media split

**Claim**: TikTok's API/control-plane traffic succeeds while its
video/image CDN traffic fails (or vice versa) — a different, more
specific failure than "TikTok cannot connect at all."

Status: **UNKNOWN — this is the single most important thing the
Russian tester's report needs to distinguish**, and no data exists yet
to classify it either way. This is why the reproduction procedure below
(section 3) explicitly separates "feed metadata loads" from "video
thumbnails load" from "video actually plays."

### F. MTU / fragmentation

**Claim**: large packets stall (video media) while small ones (API
responses) succeed.

Status: **UNKNOWN, and correctly out of scope until evidence points
here** — per the task's explicit instruction and this project's own
established convention (`docs/CLIENT_PROTOCOL_BEHAVIOR.md`: "No
MTU/fragmentation override is set... upstream sing-box's own defaults
are used unless a reproducible failure demonstrates otherwise"). YouTube
video playback already works, which is at least as demanding a
sustained-large-transfer case as TikTok video — this weighs against MTU
as the explanation, but does not rule it out (different CDN, different
packet-size profile, different congestion behavior are all possible).
`vpn-investigate.sh mtu <tiktok-cdn-host>` exists and is reversible/
read-only if this needs checking later; nothing was run speculatively.

### G. TikTok's own regional policy (independent of transport)

**Claim**: TikTok itself restricts or degrades service for Russia-based
traffic (by account region, device region, or — most relevant here —
**exit-IP geolocation/ASN**), independent of whether the VPN protocol
works correctly.

Status: **SUPPORTED as a real, documented factor — but not yet isolated
from a network-layer cause.** This is not the same as "assume TikTok
blocking means Russian DPI blocked the VPN" (explicitly disallowed by
the task) — it is the opposite direction: TikTok itself, not Russian
censorship infrastructure, may be the actor imposing the restriction,
and a VPN's job in that case is to look non-Russian to TikTok, which is
a datacenter-exit-IP/ASN question, not a REALITY/sing-box question.

Evidence (2026-08, web search — see Sources below; treat as supporting,
non-primary evidence per the task's own sourcing rules, since TikTok
does not publish a primary policy document at this granularity):

- TikTok suspended livestreaming, new content posting, and monetization
  for Russia-based accounts/traffic starting March 2022 (in response to
  Russian legislation), while continuing to serve **viewing** of
  existing content to Russian users. As of the most recent (2026)
  reporting available to this session, that split — viewing works,
  posting/live/monetization does not — is still the commonly reported
  state, i.e. this is a **years-old, TikTok-imposed, still-current
  restriction, not a new or VPN-related phenomenon.**
- Separately and more relevant to "does the VPN itself work": community
  reporting (non-primary, used only as supporting evidence per the
  task's sourcing rules) consistently describes TikTok detecting and
  blocking based on the **VPN exit IP's datacenter/ASN reputation**,
  not just its geolocation — i.e. TikTok can distinguish "residential/
  mobile Netherlands IP" from "datacenter Netherlands IP" and treat them
  differently, independent of REALITY/Hysteria2/any protocol-level
  property.

This means TWO distinct, non-exclusive things could be happening, and
the reproduction procedure below is designed to tell them apart:

1. **TikTok viewing works from any non-Russian residential/mobile IP,
   but this VPS's specific IP/ASN is flagged as a datacenter/proxy
   exit** → the fix is an exit-IP/provider decision, not a sing-box
   config change. singbox-vpn cannot "fix" this by changing REALITY
   parameters, DNS, or routing — see the task's own explicit instruction
   not to invent a configuration fix for an IP-reputation problem.
2. **TikTok viewing is restricted for Russia-associated
   accounts/devices/SIMs regardless of exit IP** (e.g. account flagged
   at signup from a Russian number) → no VPN, including a perfectly
   working one, changes this. This is not a singbox-vpn problem at all.

Neither of these is proven yet. Both outrank most transport-layer
hypotheses in prior probability precisely because TikTok's Russia
posture is a known, long-standing, actively-maintained policy — unlike
YouTube, which has no equivalent self-imposed restriction and has
already been confirmed working end-to-end. **This is exactly why
hypothesis G is ranked first below, and why the first experiment
proposed is a comparison test, not a config change.**

## 3. Comparison table (YouTube vs. TikTok) — what is known vs. unknown today

| Characteristic | YouTube | TikTok |
|---|---|---|
| DNS resolution | Unverified end-to-end on real device, but works in practice (video plays) | Unverified — no data yet |
| IPv4 | Works (video plays) | Unverified |
| IPv6 | Unverified, works in practice | Unverified |
| TCP/443 (REALITY transport) | Works | Unverified whether TikTok even reaches this far |
| UDP/443 (Hysteria2 / app QUIC) | Works (native app plays video — implies either QUIC isn't required, or it works, or the app falls back cleanly) | Unverified |
| Own-service regional policy | None known — Google does not self-restrict YouTube for Russia | **TikTok self-restricts posting/live/monetization for Russia since March 2022; viewing is (reportedly) still generally served, but exit-IP/ASN reputation can independently affect VPN users** — see hypothesis G |
| Exit-IP/ASN sensitivity | No evidence Google CDN differentiates by ASN for this deployment | Plausible per community reporting; unverified against this specific VPS |
| Works in browser | Not directly tested per the task's framing (native app confirmed) | Unknown — task explicitly asks to test separately from the app |
| Works in native app | **Confirmed working, including playback** | **Confirmed failing** (this is the entire premise of this investigation) |

The single largest confirmed difference is not a protocol property at
all — it's that **TikTok, unlike YouTube, has a known, self-imposed,
Russia-specific service policy that predates and is independent of this
VPN.** That does not prove hypothesis G is the whole story (a
transport-layer issue could coexist with it, or could be the real cause
while the policy is a red herring), but it means the reproduction
procedure must be able to distinguish "TikTok policy" from "network
path" before any protocol-level experiment is worth running.

## 4. Reproduction procedure for the Russian tester

This is what is actually needed before ranking can move past "UNKNOWN."
Nothing below requires installing anything new — it uses the existing
Reality/Hysteria2/tcp-only profiles already served by this deployment's
subscription endpoint.

### 4.1 What to test

Run each of the following **separately** and record the result
precisely — do not summarize as "TikTok doesn't work":

1. TikTok app — cold start, does it open at all?
2. TikTok app — does the "For You" feed load (thumbnails visible)?
3. TikTok app — does tapping a video actually start playback, or does
   it spin/stall/error?
4. TikTok app — profile images, comments, login — load or not?
5. TikTok web (`www.tiktok.com` in a mobile or desktop browser) — same
   breakdown as above (feed loads? thumbnails? playback?).
6. YouTube app (control — already known-working, confirm still true
   during the same session).
7. YouTube web (control).
8. General HTTPS browsing (control — a couple of ordinary sites).

For each, record: **opens/does not open, feed metadata loads, thumbnails
load, video starts, video stalls after starting, comments load, login
works, exact error message if any, and whether it fails immediately or
times out.** "TikTok API works but the video CDN fails" is a
categorically different result than "TikTok cannot connect at all" —
treat them as different findings, not the same failure.

### 4.2 Where to test

Run the full 4.1 list under each of:

- Mobile network, VPN off (baseline — establishes what TikTok already
  does for this SIM/number without any VPN involved, which bears
  directly on hypothesis G's "account/SIM region" variant).
- Mobile network, VPN on.
- Wi-Fi, VPN off.
- Wi-Fi, VPN on.

### 4.3 Which VPN profile

Repeat 4.1–4.2 for each profile already available from this deployment,
never combining more than one variable at a time:

- **VLESS+REALITY** (the default/production profile — test this first,
  it's what matters most).
- **Hysteria2** (manually selected in the client's selector, not
  `auto`).
- **`?compat=tcp-only`** (already-shipped, already-tested-for-a-
  different-symptom profile — re-importing the subscription with
  `?compat=tcp-only` appended gives a real answer to hypothesis A
  without any new code: if TikTok starts working here, UDP/QUIC is
  implicated; if it doesn't, that's evidence against hypothesis A, not
  for it).

### 4.4 The single highest-value additional data point

If at all possible, **repeat step 4.1 (TikTok app, mobile network) using
a different, non-datacenter VPN product** (a well-known commercial
consumer VPN, or Outline on the same VPS if convenient) from the same
device/SIM/network. This is not about validating a competitor — it is
the one test that can directly separate hypothesis G's two variants
from everything else:

- TikTok fails through **both** singbox-vpn and another VPN on a
  similar exit → points at TikTok's own Russia/account policy, not this
  project's transport.
- TikTok fails through singbox-vpn **but works** through a consumer VPN
  exiting from a similar or different country → points at this VPS's
  specific IP/ASN reputation, not the REALITY/Hysteria2 protocol itself
  — the fix, if any, is a hosting/IP decision, not a sing-box config
  change.
- TikTok works through singbox-vpn already in some of the above
  combinations but not others → narrows to whichever transport/network
  combination differs, which is exactly what 4.1–4.3 is designed to
  surface.

## 5. Hypothesis ranking

Ranked by prior probability given everything established above —
**not** by ease of fixing, and explicitly not defaulting to the example
ranking the task description warned against reusing without evidence:

1. **TikTok's own Russia service policy / exit-IP-ASN reputation (G)** — HIGH.
   The only hypothesis with independent, existing, non-speculative
   evidence (TikTok's documented March-2022-onward Russia restrictions;
   community-reported ASN-based VPN detection) that is unrelated to
   anything this project's config could cause or fix.
2. **Control-plane vs. CDN/media split, cause unknown (E)** — MEDIUM,
   pending data. Not yet known whether TikTok fails at DNS, at the API
   layer, or only at video playback — this determines which of the
   other hypotheses are even reachable, so it must be established by
   the reproduction procedure before any other hypothesis can be
   meaningfully tested.
3. **QUIC/UDP application-level behavior (A)** — MEDIUM. Directly
   testable today with zero new code via the existing `?compat=tcp-only`
   profile; genuinely plausible for a video-heavy app; YouTube's
   working native playback is weak evidence against QUIC/UDP being
   broken in general on this deployment, but does not rule out TikTok's
   own QUIC usage or fallback behavior differing from YouTube's.
4. **DNS (B)** — MEDIUM-LOW. No evidence yet either way; plausible in
   principle (CDN steering) but no specific signal pointing here over
   any other client-owned unknown.
5. **Routing/sniffing misclassification (D)** — LOW. Disproved at the
   config-generation level (no `route.rules`, no domain-specific
   handling exists anywhere in this project's generated config);
   residual risk is entirely inside Hiddify's own client behavior,
   unverifiable without a device-side capture.
6. **IPv6 (C)** — LOW. Plausible in the abstract, weakly disfavored by
   YouTube already working end-to-end over whatever this deployment's
   real IPv4/IPv6 posture is.
7. **MTU/fragmentation (F)** — LOW. Explicitly out of scope per the
   task until a reproducible large-transfer-specific symptom is
   observed; YouTube's working sustained video transfer is evidence
   against a blanket MTU problem on this path.
8. **REALITY handshake issue** — LOWEST. REALITY already works for
   YouTube, browsing, and the existing Russian user base on the exact
   same VLESS+REALITY transport — there is no mechanism by which
   REALITY's handshake would succeed for one application's traffic and
   fail for another's; this is ruled out unless the reproduction
   procedure produces REALITY-layer errors specific to TikTok's
   connections (checkable server-side via `vpn-investigate.sh client
   <ip>` if/when a specific tester IP is known).

### 5.1 2026-08-20 addendum: cross-reference to the YouTube investigation

`docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` (a separate, independently-run
investigation into the native YouTube-app symptom) reached Berlin-side
findings worth cross-referencing here, without collapsing TikTok's
distinct diagnosis into YouTube's:

- **INFERENCE, not FACT**: to whatever extent Berlin testing establishes
  that this deployment's VLESS+REALITY/Hysteria2/Hiddify transport stack
  behaves cleanly end-to-end for YouTube there, that is evidence
  *against* a generic sing-box/Hiddify transport bug as TikTok's
  explanation too — it does not prove TikTok's cause, but it further
  weakens hypothesis A (3, above) relative to hypothesis G (1, above),
  which already outranks it. This does not change the ranking order
  above; it reinforces the direction it already pointed, and should not
  be read as new evidence that lets hypothesis A be promoted or
  dismissed outright — TikTok's own reproduction procedure (§4) is still
  what settles it, not an inference borrowed from YouTube.
- **FACT** (see the YouTube document's §9.5a/§13.3, source-verified
  against sing-box v1.13.19 and Xray-core): sing-box's `xtls-rprx-vision`
  does not reproduce Xray-core's UDP/443-interception behavior. If
  hypothesis A (QUIC/UDP) is ever tested for TikTok via `?compat=tcp-only`
  or a Vision-related profile, do not explain the result with "Vision
  blocks QUIC" unless the exact active core for that test is confirmed —
  the same caveat the YouTube document now applies to its own testing.
- Russia-specific network filtering and TikTok's own service policy
  (hypothesis G) remain non-exclusive and possibly interacting, per this
  document's own §"G" analysis — nothing above changes that.

## 6. Proposed first experiment

**Exactly one experiment, as required.** It is a data-collection pass,
not a config change — per the task's own rule 2 ("do not modify
anything until the failure is classified") and rule 12 ("proposed first
experiment" must precede implementation, and this phase has zero
TikTok-specific real-device evidence to act on yet.

- **Why**: hypothesis G is ranked highest but is not yet distinguished
  from a network-layer cause, and hypothesis E (control-plane vs. CDN
  split) gates every other hypothesis. Both require the Russian
  tester's structured report (section 4) before any code or config
  change would be evidence-based rather than speculative — exactly what
  the task prohibits ("do not invent a configuration fix when the
  evidence says the VPN itself is not the cause").
- **Files/config affected**: none in production. This pass added:
  - `deploy/lib/vpn-investigate.sh` (`tiktok` subcommand) — read-only
    server-side DNS/TCP/QUIC diagnostic, same shape and safety
    guarantees as the existing `youtube` subcommand. Run it once on the
    VPS (`sudo vpn-investigate.sh tiktok`) to rule out a gross
    server-side DNS/TCP/QUIC block to TikTok's root domains before the
    device test, and to have a baseline to compare against if a later
    experiment is warranted.
  - `deploy/lib/tests/test-vpn-investigate.sh` — extended to cover the
    new subcommand's `--help` text, matching the existing convention.
  - This document.
- **Expected result**: the `tiktok` diagnostic either shows the VPS's
  own network path to TikTok's root domains is clean (expected, given
  general browsing already works) or reveals a gross server-side block
  — the latter would be a surprising, high-value finding that
  reprioritizes everything above. Absent that, the real signal comes
  from the Russian tester completing section 4's procedure.
- **Rollback**: `git revert` the commit, or simply do not run the new
  `tiktok` subcommand — it is inert, read-only, and not wired into any
  install/update/service path.
- **Russian client test required**: yes — section 4's full procedure,
  including the profile-by-profile breakdown (REALITY / Hysteria2 /
  tcp-only) and, if at all feasible, the cross-VPN comparison in 4.4.
  This experiment does not claim TikTok works or is fixed — it only
  establishes the diagnostic baseline and the exact reproduction
  procedure needed to interpret whatever the tester reports next.

## 7. What was NOT done (explicitly)

Per the task's constraints, none of the following were touched, and none
should be touched until the section 4 reproduction data comes back:

- No MTU, BBR, congestion-control, or sysctl change.
- No REALITY key, destination, or port change.
- No DNS provider, resolver, or global DNS behavior change.
- No firewall rule change.
- No Hysteria2 parameter change.
- No `route.rules` added to any subscription profile (would repeat the
  exact mistake `docs/COMPATIBILITY_QUIC_EXPERIMENT.md` already
  rejected for a similar symptom, for the same unverifiable-client-
  behavior reasons).
- No new TikTok-specific compatibility mode, selector, or profile —
  `?compat=tcp-only` already exists and is reused as-is for testing
  hypothesis A; nothing new was added to `crates/compat-config`.
- No claim that TikTok is "fixed" — the task's own success criteria
  (native app plays multiple videos, sequential scrolling, cold start,
  reconnect, mobile + Wi-Fi) requires real-device evidence this phase
  does not yet have.

## Sources

- [Is TikTok Banned in Russia? 2026 Status & Unlock](https://bearvpn.com/blog/is-tiktok-banned-in-russia/) — supporting evidence only, not treated as authoritative; corroborates TikTok's own March-2022-onward Russia service restrictions (posting/live/monetization suspended, viewing generally still served) as still current as of 2026.
- [How to bypass TikTok block in 2026 – VPN, proxies, antidetect](https://browser.vision/blog/how-to-bypass-tiktok-blocking) — supporting/community evidence for exit-IP/datacenter-ASN-based detection as a factor independent of geolocation alone.
- `sing-box.sagernet.org/configuration/route/rule/` and `.../rule_action/` — cited for the `route.rules`/`action: reject` syntax referenced in hypothesis A's discussion of why a routing-rule approach is not proposed (same reasoning as `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`, not re-verified independently in this pass since no such rule is being shipped).
