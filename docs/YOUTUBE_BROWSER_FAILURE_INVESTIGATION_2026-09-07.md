# YouTube browser+app failure — investigation (2026-09-07)

**Status: investigation only. No code change shipped. No live device or VPS
access was available in this session — see §4.** Every claim below is
labeled FACT, INFERENCE, or HYPOTHESIS. Do not read anything here as a
confirmed fix.

## 1. Executive status

The symptom escalated: `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` (2026-08-20)
investigated Safari-works/native-app-fails. That symptom is no longer the
whole picture — YouTube now reportedly fails in the browser too. This pass
could not reproduce or measure the failure directly (no VPS/device access —
§4), so it re-reads the historical evidence, re-runs the reasoning the task
requires, checks this repository's own code/history for a regression, and
adds fresh external research about Russian network conditions in 2026. It
ends with a ranked hypothesis table and an exact runbook for whoever has
device/VPS access — it does not claim the incident is resolved, and it does
not ship a speculative fix.

**Confidence this report's factual claims (repo state, diff, external
sources) are correct: 8/10.** Confidence in the ROOT CAUSE ranking below:
**3/10** — it is reasoned from historical docs plus fresh research, not from
a single new packet captured or command run against the real system this
session. Treat the ranking as a prioritized worklist, not a verdict.

## 2. Exact current symptom (as reported, not yet independently measured)

> YouTube fails in the browser AND (per prior history) the native app, on
> the affected client/network. Ordinary VPN function — VLESS+REALITY
> connects, Hysteria2 connects, "ordinary browsing" works — is reported
> intact.

**This report could not fill in the Phase-0 breakdown the task requires**
(home feed / thumbnails / search / metadata / comments / video bytes /
playback / audio / seeking, DNS vs. TCP vs. TLS vs. media-CDN-only failure)
because that requires the actual affected device and network, which this
session does not have. **This is the single most valuable thing to capture
next** — see §10's runbook, item 1.

## 3. What changed from the historical (native-app-only) symptom

| Historical hypothesis (`YOUTUBE_NATIVE_APP_INVESTIGATION.md`) | Given "browser also fails now" |
|---|---|
| Application-level QUIC/UDP behavior specific to the YouTube **app** (§3.1) | **LESS LIKELY as the sole/primary cause.** A browser doing ordinary HTTPS video playback is not the same QUIC-preferring native-app code path this hypothesis was built around. It could still be a *contributing* factor for the app specifically, but it cannot by itself explain a browser failure. |
| UDP relay bug in Hiddify's bundled sing-box core (§3.2) | **LESS LIKELY as sole cause**, same reasoning — a relay bug that only affects UDP-relayed application traffic doesn't explain plain browser HTTPS. |
| VPS outbound UDP failure (egress to Google) (§3.3) | **UNCHANGED / still plausible**, but now insufficient alone — it would explain the app's QUIC path failing, but ordinary browser YouTube playback commonly falls back to or partly uses HTTP/2 (TCP) already, so pure UDP egress failure is a weaker sole explanation for total browser failure than it was for the native-app-only symptom. |
| Hiddify TUN/routing behavior specific to the app (§3.4) | **LESS LIKELY as sole cause** — same reasoning as QUIC/relay: this was invoked to explain an app-vs-Safari *difference*; it doesn't explain both failing identically. |
| IPv6 routing / AAAA-first behavior (§3.6) | **UNCHANGED, still open** — could affect browser and app equally, was never ruled in or out. |
| DNS resolution differences (§3.7) | **MORE LIKELY, now the top candidate — see §5.2.** A DNS-level failure (wrong/empty answer, or a leak to a resolver that can no longer answer for YouTube) affects every client on the device equally — browser and native app alike — which is exactly the shape of the new symptom. |
| Exit-IP/ASN/Google-peering (§3.8) | **MORE LIKELY.** An IP/ASN-level block or degradation at the exit also affects every client equally, same reasoning as DNS. This was already ranked #1 in the historical decision tree for a reason — the new symptom is *more* consistent with it, not less. |
| Vision flow changing VLESS's TLS-layer behavior (§9.5a) | **UNCHANGED / unlikely to be primary** — sing-box's Vision does not intercept UDP/443 (source-verified in the prior investigation); nothing about Vision would newly break a browser that wasn't broken before, absent a change to Vision itself. |
| Stale session/app state (§3.10) | **UNCHANGED** — controlled for by the existing reset procedure (§9.7 there), not eliminated by this pass. |
| Russian censorship/DPI targeting VLESS/REALITY itself (not separately ranked before) | **RECONSIDERED — see §5.3.** New research this pass found reports of TSPU throttling/blocking port-443 VLESS+REALITY specifically in 2026. This is evidence **against** it being the *whole* explanation here, though: if TSPU were dropping the tunnel itself, ordinary browsing and the VPN connection would also fail, which is reported as still working. Kept on the list as a partial/compounding factor, not the leading hypothesis. |

**Bottom line of this section**: the shift from "app-only" to "app+browser"
does not point at the same place the original investigation was heading
(QUIC/UDP-relay/Vision/client-core). It points toward something that would
affect **every client on the device identically** — which narrows to DNS,
IP/ASN/exit path, or a YouTube/Google-side response to the exit IP. This
reasoning is an **INFERENCE** from the shape of the symptom, not a measured
fact — it still requires the Phase-0/Phase-1 device tests in §10 to confirm.

## 4. Environment — what this session could and could not do

**FACT.** This session runs in an isolated cloud container with access only
to this GitHub repository (`David610/singbox-vpn`). It has:

- No SSH access to the production VPS.
- No access to the iPhone 12 mini / Hiddify app used for acceptance.
- No access to the affected network (presumed, not confirmed, to be a
  Russian ISP — see §4.1).
- No live sing-box process, no packet capture capability against real
  traffic.

Consequently, **none of Phases 0, 2–15 in the task's own plan could be
executed here.** What this session *could* do, and did:

1. Read every historical doc named in the task, plus the code paths named
   (`crates/compat-config/src/{render,server,model}.rs`,
   `deploy/lib/vpn-investigate.sh`, `services/subscription`, `apps/admin`).
2. Diff the accepted release commit against current `main` (§7.1) — a real,
   checkable fact this session *can* establish.
3. Run fresh web research (§5) — allowed and required by the task.
4. Write the runbook in §10 for whoever has live access to actually execute.

### 4.1 Unstated assumption flagged, not guessed past

The task's phases (Russia-specific DPI, TSPU, Roskomnadzor, "Moscow
ingress") strongly imply the affected client is on a Russian network, but
the task's PRIMARY OBJECTIVE section never says so explicitly, and neither
does the stable-release acceptance record
(`docs/release-acceptance/v1.0.0.md`), which lists no region for the
device test network. **I am treating "affected network is Russian" as a
plausible, contextually-supported INFERENCE, not a confirmed fact.**
Confidence: 6/10. If the affected network is NOT Russian, most of §5's
research (which is Russia-specific) does not apply, and the investigation
should restart from §3.8 (exit-IP/ASN) and DNS in general, without the
Russia-specific framing. **Whoever runs §10's runbook should record the
actual network/country/ISP as the very first field** — this is exactly the
kind of input this report should not guess past, per the requirement to
open unclear inputs rather than assume them.

## 5. External 2026 research (fresh, this session, dated close to today)

Every claim below is labeled and sourced. None of this is primary-source
(sing-box/Hiddify docs, OONI raw data) — it is journalism and community
technical writeups, exactly the kind of source the task says may inform but
not replace packet-level evidence. Treat confidence accordingly.

### 5.1 YouTube is now nationally blocked in Russia at the DNS level (FACT, well-corroborated)

Multiple independent, reputable outlets report that on **2026-02-10**,
Roskomnadzor removed the YouTube domain from Russia's National Domain Name
System (NSDI) under the "sovereign Runet" law; WhatsApp followed a day
later, and at least 13 resources total (including Facebook, Instagram, Tor,
BBC, DW, RFE/RL) have been removed as of that action. The mechanism
reported: devices relying on Russia's national DNS infrastructure simply
stop receiving an IP address for `youtube.com` and related domains — it is
described as a DNS-layer removal, not (only) an IP block.

- [Blocking of YouTube in Russia — Wikipedia](https://en.wikipedia.org/wiki/Blocking_of_YouTube_in_Russia)
- [Russia Escalates Internet Censorship Removing YouTube and WhatsApp From National Domain System — UNITED24 Media](https://united24media.com/latest-news/russia-escalates-internet-censorship-removing-youtube-and-whatsapp-from-national-domain-system-15835)
- [Russia uses new website blocking method... — The Insider](https://theins.press/en/news/289338)
- [Russia removes WhatsApp, Facebook, YouTube... — Asia-Plus](https://www.asiaplustj.info/en/news/world/20260212/russia-removes-whatsapp-facebook-youtube-and-major-news-sites-from-national-dns-servers)

**Why this matters here (INFERENCE, confidence 6/10):** this repository's
own code makes DNS **entirely the client's responsibility** — confirmed by
reading `crates/compat-config/src/render.rs` (no `dns` block is ever
emitted; `client_subscription_has_no_dns_block_and_no_inbounds` enforces
this) and restated in `docs/CLIENT_PROTOCOL_BEHAVIOR.md`. If Hiddify, for
this user's specific build/settings, resolves `youtube.com` via the
device's local/ISP resolver instead of routing the DNS query through the
tunnel, the query hits Russia's NSDI, finds nothing, and **every client on
the device — browser and app alike — fails identically**, while every other
(non-blocklisted) domain, and the VLESS/Hysteria2 tunnel itself (which
connects by IP/hostname the operator controls, not through NSDI), keeps
working. This is the one hypothesis in §3's table that cleanly explains
"browser now also fails" without requiring anything in this repository to
have changed — consistent with §7.1's finding that nothing did.

**This is NOT proven.** It requires confirming, on the actual device: (a)
does `youtube.com` fail to resolve at all while connected, and (b) is that
resolution actually leaking outside the tunnel. Neither is testable from
this session.

### 5.2 TSPU reportedly now targets port-443 VLESS/REALITY specifically (HYPOTHESIS, weak sourcing, partially contradicted by the reported symptom)

Community/vendor-blog sources (not primary, not independently verified by
this session) report that by mid-2026, after broader TSPU (Roskomnadzor's
deep packet inspection system) rollout, self-hosted VLESS+REALITY on port
443 is "instantly dropped or throttled to zero" on some Russian access
networks, while the same traffic on a high port (47000+) reportedly fares
much better; separately, Roskomnadzor is reported to have begun blocking
VLESS as a protocol (among SOCKS5, L2TP) starting around December 2025.

- ["Clumsy Hands" or a New Level of DPI? — Habr](https://habr.com/en/articles/990236/)
- [Russia's TSPU System — iplogs.com](https://iplogs.com/blog/russia-tspu-how-it-blocks-vpns)
- [Russia's TSPU to 2030 — TGV](https://tgvpn.io/en/tspu-dpi-russia-2030-analysis.html)
- [Russia Begins Blocking VLESS VPN Protocol — Mezha](https://mezha.net/eng/bukvy/russia-begins-blocking-vless-vpn-protocol-increasing-internet-restrictions/)

**Why this is ranked below DNS here (INFERENCE, confidence 5/10):** if TSPU
were dropping/throttling this deployment's port-443 VLESS+REALITY tunnel
itself, the symptom would be **the whole tunnel failing or degrading**, not
"YouTube specifically fails while REALITY, Hysteria2, and ordinary browsing
work." The user's own report says the tunnel and ordinary browsing work.
That is evidence *against* a blanket port-443-VLESS block being the
dominant cause of *this specific* incident — though it remains a real,
separately-worth-tracking risk for this deployment's overall Russia
posture (this pass does not resolve or dismiss it; see
`docs/RUSSIA_PRODUCTION_INVESTIGATION.md` for that separate, ongoing
question), and it is a plausible **partial contributor** if TSPU applies
finer-grained, destination-aware throttling (e.g., specifically degrading
sessions whose traffic pattern matches Google/YouTube CDN IPs) rather than
a blanket port-443 rule. That finer-grained variant cannot be confirmed or
ruled out without a packet capture during a failing attempt (§10, item 5).

### 5.3 sing-box 1.13.x / Hiddify — no new relevant defect found (INFERENCE, confidence 4/10 — limited by this session's lack of web/issue-tracker depth)

No sing-box 1.13.x changelog entry or open issue specific to VLESS/REALITY/
DNS/QUIC regressions was found that post-dates the pinned `1.13.19` and
would explain a new browser-wide YouTube failure. This is a weaker,
absence-of-evidence finding — this session's research depth here was
limited (general web search only, no direct GitHub issue-tracker crawl of
`SagerNet/sing-box` or `hiddify/hiddify-app`). **Do not treat this as
"ruled out"** — it is UNTESTED, not DISPROVED. If whoever has live access
finds Hiddify auto-updated its app or its bundled sing-box-core version
recently, that is a new variable this pass did not have visibility into and
should be recorded explicitly (§10, item 2).

### 5.4 AWS/datacenter IP treatment by YouTube — real phenomenon, but a poor fit here (INFERENCE, confidence 3/10 as the explanation for THIS incident)

It is a well-documented, ongoing (not a 2026-specific new) phenomenon that
YouTube/Google treats requests from cloud/datacenter IP ranges (AWS, GCP,
Azure) with more suspicion — CAPTCHA/`LOGINREQUIRED` responses for
automated tools, primarily. This is real but does not obviously explain
"video playback (a normal browser session, not automated scraping) fails
outright" — the documented pattern is bot-detection friction on
programmatic access, not blocking ordinary interactive playback for a
residential-looking browser session tunneled through a VPS IP (this is,
after all, exactly what any commercial VPN's exit IP already looks like to
Google, and VPN users routinely watch YouTube through cloud-hosted exits).
Kept on the list per the task's own instruction to check exit-IP/ASN, but
ranked low for explaining *this* incident specifically, pending the
same-VPS-vs-WARP control in §10.

- [The Datacenter IP Block — ansaribilal.com](https://ansaribilal.com/blog/ytagent-datacenter-ip-block-youtube-ai-agents-2026/)
- [yt-dlp issue #9015 — AWS Lambda IP denied](https://github.com/yt-dlp/yt-dlp/issues/9015)

## 6. Experiments performed this session

Only checks executable without live VPS/device access:

| # | Hypothesis | Check | Result |
|---|---|---|---|
| 1 | A code regression between the accepted-working RC and the current stable release caused this | `git log`/`git diff --stat 74b7a06..HEAD` | **FACT**: the only change is one documentation commit (`docs/release-acceptance/v1.0.0.md`, 18 lines, recording acceptance). Zero code changed. See §7.1. |
| 2 | This repo's firewall scripts restrict outbound/egress traffic from the VPS | `grep` for OUTPUT/FORWARD/egress rules in `deploy/almalinux/firewall.sh` | **FACT**: the script only manages inbound zone/port rules (SSH-safe activation, REALITY/Hysteria2 ports). No OUTPUT/FORWARD/egress rule exists anywhere in this repo's firewall code. If outbound UDP/443 to Google is blocked, it is not this repository's doing — it would be the OS's own default policy (typically ACCEPT), or an AWS Security Group/NACL, neither of which this repo manages or can introspect. |
| 3 | The repository already has YouTube-specific server-side diagnostics | Read `deploy/lib/vpn-investigate.sh` | **FACT**: `youtube`, `youtube_dns`, `youtube_tcp`, `youtube_quic` (and the TikTok equivalents) already exist and do exactly what Phase 3/9 of the task asks — DNS A/AAAA, TCP+TLS connect (IPv4 and IPv6 separately), and an honest HTTP/3 probe that reports "tooling gap" rather than a false result when curl lacks HTTP/3. No new server-side reachability tooling is needed; §10 reuses this directly. |
| 4 | A `dns`-block compatibility mode (mirroring `compat=tcp-only`/`compat=vision-off`) could be shipped now to test/fix the DNS hypothesis | Read `render.rs`'s existing compat-mode mechanism in full | **Considered and rejected for this pass — see §9 and §13.** The exact same objection that killed the `route.rules` idea in `docs/COMPATIBILITY_QUIC_EXPERIMENT.md` (cannot verify Hiddify actually imports/honors a field this project cannot observe it applying) applies at least as strongly to an injected `dns` block. Shipping it without that verification would repeat a mistake this repository's own history explicitly warns against, and the task itself directs testing a **client-side, zero-code-change** DNS override first (Phase 7). |

## 7. Raw factual observations

### 7.1 No code regression between accepted RC and current stable

```
74b7a06 (accepted RC, v1.0.0-rc.9) ──▶ d84131c (current HEAD, v1.0.0)
  docs/release-acceptance/v1.0.0.md | 18 ++++++++++++++++++
  1 file changed, 18 insertions(+)
```

**FACT.** Whatever is causing the current failure, it is not a code change
in `crates/`, `services/`, `apps/`, or `deploy/` between the last
known-good state and now — those are byte-identical.

### 7.2 The acceptance record never actually tested YouTube

**FACT**, read directly from `docs/release-acceptance/v1.0.0.md`: the
device acceptance evidence lists REALITY/Hysteria2 connect, handover,
sustained transfer, reconnect, idle/resume, revocation, token rotation, and
refresh/re-import as PASS. **YouTube is not mentioned anywhere in that
record.** This matters for calibration: "YouTube worked before" per the
task's framing is the user's own recollection, not something the formal
acceptance record verified. That does not make the user's report wrong —
it means there is no dated, device-verified baseline to diff against,
which is exactly why `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` and
`docs/RUSSIA_PRODUCTION_INVESTIGATION.md` both independently arrived at
"UNVERIFIED" for this exact application, repeatedly, over the past month.

### 7.3 Acceptance was recorded today (2026-09-07) — same day as this task

**FACT.** This is worth flagging plainly rather than silently noting: the
release this incident is against was accepted *today*. If the YouTube
failure was present at the moment of acceptance too, "previously worked"
may describe an earlier release or an earlier point in time, not
specifically v1.0.0/rc.9 — another reason §4.1's "record what network/date
the failure was first observed on" matters more than it might otherwise.

## 8. Root-cause ranking

Ranked by fit to the CURRENT symptom (browser + app both fail; tunnel,
REALITY, Hysteria2, ordinary browsing all reported healthy), using only
what this session could establish. **This is a prioritized worklist for
the runbook in §10, not a verdict** — see the confidence column.

| Rank | Hypothesis | Evidence for | Evidence against | Confidence it's the (a) cause | Next action |
|---|---|---|---|---|---|
| 1 | DNS: YouTube domain unresolvable via a non-tunneled/leaking resolver, compounded by Russia's Feb-2026 NSDI removal of youtube.com | Explains browser+app failing identically; explains ordinary sites/tunnel unaffected; this repo confirmed-by-code never controls client DNS; well-corroborated external report of the NSDI removal | Not yet measured on the real device; Safari reportedly worked as recently as the Aug-2026 investigation, after the NSDI change — so whatever changed since must be on the DNS-path/client side, not the NSDI action itself | 3/10 (plausible, unmeasured) | §10 items 1, 3, 6 |
| 2 | Exit IP/ASN/Google-CDN peering (this specific AWS IP/subnet treated differently for Google/YouTube specifically) | Explains browser+app failing identically while other sites work; ranked #1 in the prior Russia investigation's own decision tree for unrelated reasons | AWS-hosted VPN exits routinely serve YouTube elsewhere; no evidence yet that THIS IP is degraded specifically for Google | 2/10 | §10 items 4, 7, 8 |
| 3 | Finer-grained (destination-aware, not blanket-port) Russian DPI/TSPU interference specific to Google/YouTube-bound traffic through this tunnel | Matches the general 2026 TSPU-escalation research direction | Reported tunnel/ordinary-browsing health argues against a blanket VLESS/port-443 block; no packet-level evidence either way for a narrower rule | 2/10 | §10 item 5 |
| 4 | VPS outbound UDP/443 egress to Google specifically degraded (provider/AWS Security Group) | This repo's own firewall code doesn't restrict egress (§6.2) — a real outbound problem would have to be AWS-side, consistent with "not this repo's code" | No evidence yet either way | 1/10 | §10 item 4 |
| 5 | Application QUIC/UDP-relay/Vision/client-core (the ORIGINAL native-app hypotheses) | Still could be a compounding factor for the app specifically | Does not explain the browser also failing; this was the leading theory for the OLD symptom, not the new one | 1/10 as sole/primary cause of the CURRENT symptom | Keep `compat=tcp-only`/`compat=vision-off` as already-shipped diagnostics for the app-specific slice, but do not lead with these |
| 6 | IPv6 routing/AAAA-first behavior | Never ruled out | Would need to explain browser+app identically, no more evidence for it now than before | 1/10 | §10 item 9 |
| 7 | Stale session/cached state | Controlled for by reset procedure | Would not typically produce a persistent, reproducible failure across a browser AND an app | <1/10 | Rule out first via reset procedure before trusting any other result |
| 8 | Code regression in this repository | Checked directly | **DISPROVED** — zero code diff between accepted-working RC and current HEAD (§7.1) | 0/10 | None — closed |

## 9. Root cause

**UNKNOWN. Confidence in any single answer: low (see §8).** This session
cannot respect the task's own instruction to determine a root cause from
evidence, because the evidence that would decide between ranks 1–3 above
does not exist yet and cannot be produced without VPS/device access this
session does not have. Saying otherwise would be exactly the kind of
guess the task explicitly forbids ("do not say 'could be anything' — rank
them," which §8 does; it does not say "pick one without evidence").

## 10. Immediate fix

**None shipped.** There is no code bug to fix (§7.1), and the leading
hypotheses (§8, ranks 1–3) are all outside this repository's code — they
are client-DNS-configuration, exit-IP/provider, or live-network questions
that require the runbook below, not a patch. Shipping a speculative code
change (e.g., a DNS compat mode) without being able to verify it against
the real device would repeat the exact mistake `docs/
COMPATIBILITY_QUIC_EXPERIMENT.md` already documents and rejects for a
structurally identical reason (§6, row 4; §13).

**What to actually do next — exact runbook, ordered by information gain,
using tools that already exist:**

1. **Fill in the Phase-0 breakdown for real**, on the affected device,
   right now, before anything else: browser YouTube — homepage /
   thumbnails / search / video metadata / comments / playback start /
   sustained playback / seeking, each PASS/FAIL, with the reset procedure
   from `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.7. Also record the
   network (Wi-Fi/cellular), country/ISP if known, and exact timestamp
   (UTC). This alone will falsify or support several rows in §8 (e.g., a
   total-failure "server never responds" result looks very different from
   "homepage loads, thumbnails load, playback specifically stalls").
2. **Record exact client versions**: Hiddify app version/build (Settings →
   About), and whether it auto-updated recently; iOS version. Compare
   against what was used for `docs/release-acceptance/v1.0.0.md`
   (iOS 26.6.1, Hiddify "4.0.0 dev"/"4.0" App Store) — if either changed
   since that record, note it as a new variable.
3. **Cheapest, zero-code-change test for hypothesis #1 (DNS)**: in
   Hiddify's own app settings, look for a DNS/"Remote DNS" override (many
   sing-box-based clients expose one) and point it at a public resolver
   (e.g. `1.1.1.1` or `8.8.8.8`) instead of whatever it defaults to, or
   enable Hiddify's own "Bypass LAN"/"Fake IP"/DNS-through-tunnel option if
   present. Reconnect, force-quit the browser, retest. If this alone
   fixes browser YouTube, hypothesis #1 (DNS) is strongly confirmed and
   the fix is a **Hiddify client-side setting**, documented for users —
   not a server change. This is the task's own Phase-7 instruction,
   restated: test client DNS before touching product code.
4. **Server-side reachability** (already-built tooling, run as root on the
   VPS): `deploy/lib/vpn-investigate.sh youtube` — does exactly Phase 3's
   DNS/TCP/TLS/QUIC checks against `youtube.com`, `googlevideo.com`,
   `youtubei.googleapis.com`, `ytimg.com`, `ggpht.com`, dialed directly
   from the VPS (not through the tunnel). A FAIL here narrows straight to
   §8 rank 2 or 4 (exit IP/provider/egress) without needing the phone at
   all.
5. **Correlated packet capture during a real failing attempt**:
   `deploy/lib/vpn-investigate.sh udp-egress-capture "$CLIENT_IP" /root/youtube_udp_test.pcap 60`
   immediately followed by `udp-egress-verdict` on the resulting pcap,
   exactly as documented in `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.1
   — this is the existing tool built for exactly this question and
   distinguishes "no UDP left the VPS" from "UDP left but nothing came
   back" from "bidirectional UDP already works, failure is elsewhere."
   `deploy/lib/vpn-investigate.sh client <observed-IP>` adds host-wide
   REALITY accept/reject counts correlated to the same window.
6. **DNS leak check specifically**: from the connected phone, a DNS-leak
   test page/app (e.g. dnsleaktest.com) while connected via REALITY,
   compared against the same test with the VPN off — `docs/
   CLIENT_PROTOCOL_BEHAVIOR.md` already flags this as a real test that has
   never been run. This directly answers whether DNS is leaking outside
   the tunnel.
7. **Same-VPS, different-tunnel control** (§8 rank 2/3): stand up a
   temporary WireGuard or Outline listener on the SAME VPS (same exit
   IP/ASN), no changes to the production sing-box config, and retest
   YouTube through it. Same-VPS-works-but-singbox-vpn-fails points at
   sing-box/Hiddify; same-VPS-also-fails-but-WARP-works points at the exit
   IP/ASN/provider itself.
8. **WARP as a control**, recording the full table `docs/
   YOUTUBE_NATIVE_APP_INVESTIGATION.md` §5 already specifies (public
   IPv4/IPv6, DNS resolver, full Phase-0 breakdown) — not just "WARP
   works," which by itself proves nothing about which of WARP's several
   simultaneous differences (exit IP, DNS, IPv6, MTU) matters.
9. **Force IPv4-only** on the client (Hiddify setting, if exposed) as one
   variable, per `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.3 — only
   after 1–8 above, since it's ranked lower in §8.

Every one of these is read-only or additive (temporary WireGuard/Outline
on the side, a client-side settings toggle) — nothing here touches REALITY
keys, Hysteria2 parameters, firewall rules, or DNS providers server-side,
consistent with the task's "do not randomly tune the server" rule.

## 11. Verification

Not applicable yet — no fix was made. Once runbook item 3 or another
experiment identifies a real cause, verification must use the full
acceptance bar the task specifies in "TEST SUCCESS CRITERIA": browser AND
app, homepage/search/thumbnails/playback/seeking/multiple videos, plus
ordinary sites, REALITY, Hysteria2, reconnect, and >=10 minutes sustained
playback, on the real affected device — not a single successful page load.

## 12. Rollback

Not applicable — nothing was changed in production or in this repository's
runtime behavior. If runbook item 3 (a Hiddify client-side DNS override)
is tested, rollback is reverting that one Hiddify setting — it lives
entirely on the phone, not on the server, and this repo's generated
subscription is unaffected either way.

## 13. Repository changes

**None**, deliberately. This document is the only addition. A candidate
change was considered and explicitly rejected — see §6 row 4: an
opt-in `?compat=`-style subscription mode that would inject a sing-box
`dns` block forcing DNS through the tunnel to a resolver of the operator's
choosing, mirroring `compat=tcp-only`/`compat=vision-off`. It was not
implemented because:

1. Exactly like the `route.rules` mechanism `docs/
   COMPATIBILITY_QUIC_EXPERIMENT.md` investigated and declined to ship,
   this project has no way from this environment to verify Hiddify's
   subscription importer actually preserves/honors an injected `dns`
   block from a raw JSON subscription rather than discarding or
   overriding it with its own template defaults — the same unverifiable-
   client-behavior objection that document already raised for a
   structurally similar field.
2. The task's own Phase 7 explicitly directs testing a **client-side**
   DNS change first, before any product change: "If a controlled Hiddify
   tunneled-DNS/DoH change fixes the browser immediately, document that
   evidence before product changes." Runbook item 3 (§10) is that test,
   and it costs nothing to try — no reason to build and ship unverified
   server code ahead of it.
3. Shipping code that cannot be verified against the real symptom would
   violate the task's own "no speculative hardening" and "one variable at
   a time" rules as much as changing REALITY/Hysteria2 parameters would.

If runbook items 3 and 6 (§10) together confirm DNS leakage is the
mechanism AND Hiddify's own settings cannot reliably fix it client-side,
that is the point at which building this compat mode would become
justified — with the same explicit EXPERIMENTAL labeling, per-mode test
coverage, and documentation this repository already uses for
`tcp-only`/`vision-off`.

## 14. Remaining unknowns

Everything in §8 whose confidence is below "confirmed." Explicitly, in
priority order: whether DNS actually leaks on this device (§10.6); whether
YouTube resolves at all while connected (§10.1/§10.3); whether this
specific VPS/IP is treated differently by Google than a generic residential
IP (§10.7/§10.8); whether a same-VPS non-sing-box tunnel reproduces the
failure (§10.7); the affected network's actual country/ISP (§4.1); whether
Hiddify or iOS updated since the last time this reportedly worked (§10.2);
and whether TSPU is applying any destination-aware throttling to this
tunnel's Google-bound traffic specifically (§10.5).

## 15. Recommendation

Run §10 items 1–3 first — they require no server access, no code change,
and the lowest-cost one (item 3, a Hiddify DNS-override toggle) has a
realistic chance of resolving this outright within minutes if hypothesis
#1 is correct. If it does not, items 4–8 (all already-built or
zero-risk-additive tooling) will separate exit-IP/ASN causes from
DPI/TSPU causes from a genuine sing-box/Hiddify relay bug. Do not skip
straight to a server-side product change (a new compat mode, a new egress
path, an IP rotation) before at least items 1, 3, 4, and 6 produce a
result — every one of those is cheaper and faster than the alternative,
and several of the higher-effort options in the original task (a
Tailscale multi-hop egress feature, a second VPS/provider) are explicitly
gated by the task's own text on first proving the exit-IP hypothesis, which
has not happened yet.
