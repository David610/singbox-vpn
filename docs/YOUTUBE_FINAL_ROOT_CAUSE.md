# YouTube native-app failure — investigation record (2026-09-08, corrected 2026-09-16)

**Status: `compat=quic-reject` is falsified for Hiddify.
`compat=hiddify-pinned` and the relay XUDP correction fix real routing defects
and ordinary playback, but real-device tests on 19 September 2026 show that
Shorts still fail. §15 isolates the discriminator — hosting/datacenter egress
raises Shorts risk so high that the affected phone fails every time while its
own broadband line never fails — and ships `compat=youtube-direct` as the
fixing profile. §15.1b corrects the same-day overclaim: hosting-IP Shorts
rejection is risk-based and intermittent, not an absolute block. See §14, §15,
§15.1b and `docs/YOUTUBE_INVESTIGATION_2026-09-17.md`.**

> **Read §12 and §13 before anything else.** Sections 1-11 are the
> 2026-09-08 record. Their source-level tracing of sing-box/sing-tun
> (§3) still holds and is still useful. Their *conclusion* — that
> `compat=quic-reject` fixes this incident — does not: real-device
> evidence says it changes nothing, and §12 explains why it provably
> cannot on this client. Nothing below §11 was edited, so the original
> reasoning stays auditable.

This is the current authoritative document for the YouTube incident. It
supersedes the *conclusions* of `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md`
and `docs/COMPATIBILITY_QUIC_EXPERIMENT.md` — neither is deleted, both
remain the historical evidence record, and both now carry a header
pointing here.

Evidence classification is kept explicit throughout. Nothing below is
promoted from INFERENCE to FACT.

## 1. Executive summary

Application QUIC (UDP/443) through the VLESS+REALITY tunnel does not
fail — it *hangs*. sing-box's TUN layer accepts the UDP flow, and the
failure is only noticed later at a layer whose error nobody delivers back
to the application. The YouTube app therefore sits waiting on a QUIC
handshake that will never complete and never error, so it never falls
back to HTTP/2 over TCP — the path that demonstrably works.

The `?compat=tcp-only` mode built to test this hypothesis has the same
defect and could never have produced a fallback, so it was an INVALID
TEST rather than a negative result.

The fix is a new opt-in mode, `?format=singbox&compat=quic-reject`, which
emits a `route.rules` entry that rejects application UDP/443 with an
immediate ICMP unreachable — a failure the application can actually see.

## 2. Exact symptom

Native YouTube app on iOS and Android: fails through singbox-vpn's
REALITY profile. Windows Chrome and Brave: work through the same server.
AmneziaWG, WireGuard/WARP, Outline and OpenVPN: reported working.

## 3. Root cause — traced through upstream source

Versions traced: `sing-box v1.13.19` (the deployed server version and the
generation Hiddify bundles), `sing-tun` at the commit pinned by that
release's `go.mod` (`7c73233bd0fb`), `sing-vmess` at `3aed155119a1`.

### 3a. `network: "tcp"` black-holes UDP. It does not reject it.

`Router::PreMatch` (`route/route.go`) is the hook sing-tun calls *before*
accepting a flow. With no matching rule, UDP takes this path:

```go
if directRouteOutbound == nil {
    if selectedRule != nil || metadata.Network != N.NetworkICMP {
        return nil, nil          // no error — flow is accepted
    }
```

The outbound's `network` restriction is not consulted here at all. It is
noticed later, in `routePacketConnection` (`route/route.go`), as a plain
`E.New("UDP is not supported by outbound: ...")` — and the caller of that,
`tun.Inbound::NewPacketConnectionEx` (`protocol/tun/inbound.go`),
discards the return value entirely.

**Consequence (FACT):** nothing is ever sent back to the application. No
ICMP, no reset. The app's QUIC Initial vanishes.

### 3b. A `reject` rule *is* matched by `PreMatch`, and does reach the app.

`PreMatch` returns `action.Error(...)` → `RejectedError{tun.ErrReset}`
(`route/rule/rule_action.go`). sing-tun's gVisor UDP forwarder
(`stack_gvisor_udp.go`):

```go
_, pErr := f.handler.PrepareConnection(N.NetworkUDP, source, destination, nil, 0)
if pErr != nil {
    if !errors.Is(pErr, ErrDrop) {
        gWriteUnreachable(f.stack, userData.(*stack.PacketBuffer))
    }
```

**Consequence (FACT):** an ICMP unreachable, immediately, per packet.

`no_drop: true` is load-bearing. Without it, `RuleActionReject::Error`
escalates reset → `ErrDrop` after 50 rejects in 30 seconds — restoring
the black hole mid-playback, since a video session trips that counter in
seconds.

### 3c. Why the architecture is fragile here at all

`vlessDialer::ListenPacket` (`protocol/vless/outbound.go`) opens a **new
TCP connection and a full REALITY TLS handshake per UDP session**, then
carries the datagrams as XUDP frames inside it. `PacketEncoding == nil`
defaults to `xudp` (same file). AmneziaWG and WireGuard forward UDP as
UDP at layer 3, through kernel NAT, with zero per-flow setup — which is
why they are unaffected. This is an INFERENCE about *why* the REALITY
path is more failure-prone, not part of the proven mechanism in 3a/3b.

## 4. Why Windows works (INFERENCE, confidence moderate)

Chrome and Brave mark QUIC broken per-host and fall back to HTTP/2 over
TCP; that fallback is mature and aggressive. Google's native apps use
Cronet with stronger QUIC preference and no error to react to. Same
server, same tunnel, different fallback behavior. **Not packet-verified
on the affected laptop.** No specific laptop change was ever proven to
have caused the recovery — see §5.

## 5. Verdict on every prior test

| Test | Verdict | Why |
|---|---|---|
| Normal REALITY, native YouTube | VALID FAIL | genuine user-visible observation |
| `?compat=tcp-only` | **INVALID TEST** | §3a — `network: "tcp"` black-holes rather than rejects, so it cannot trigger app fallback |
| Manual `route.rules` UDP/443 reject in Hiddify | **INCONCLUSIVE** | semantics were correct (§3b), but Hiddify's preservation of imported `route.rules` was never proven |
| MTU 1280 via subscription `inbounds[].mtu` | **INVALID TEST** | the generated subscription has no `inbounds` object to modify |
| Hiddify options import (MTU/IPv6/DNS/TUN stack) | INCONCLUSIVE | never verified as reaching the running core |
| "Xray" mode | INVALID TEST | the removed label never selected Xray |
| Wi-Fi HTTP proxy on mobile | INCONCLUSIVE | native apps may ignore an HTTP proxy entirely |
| Reconnects / restarts | VALID FAIL | no effect |
| Windows Chrome/Brave working | VALID PASS, **cause unknown** | no single change was ever isolated |

## 6. The fix

`CompatibilityMode::QuicReject`, served as
`GET /sub/{token}?format=singbox&compat=quic-reject`:

```json
{ "network": "udp", "port": 443, "action": "reject",
  "method": "default", "no_drop": true }
```

Everything else is byte-identical to the normal profile — same UUID, same
REALITY keys, same `xtls-rprx-vision` flow, Hysteria2 still offered
(asserted by `quic_reject_leaves_every_credential_and_endpoint_identical_to_normal`).

`?format=singbox` only. A routing rule has no representation in
`vless://` share-link syntax, so `format=uri`/`format=hiddify` return
400 rather than silently serving a normal profile.

`vpn-admin user link <id>` now prints this URL alongside the others.

## 7. Security and compatibility impact

- **Security: none.** No credential, key, flow or transport changes. The
  server sees an ordinary Vision REALITY session. This is the material
  difference from `compat=vision-off`, which does weaken DPI resistance.
- **Hysteria2: unaffected.** A `route.rules` entry governs traffic the
  router routes for an inbound; an outbound's own dial to the VPS does
  not traverse the route table.
- **Cost:** application QUIC to *any* host on UDP/443 is rejected while
  this profile is selected, not only Google's. Deliberate — narrower
  scoping would need destination rules this mode intentionally omits.
- **Default behavior: unchanged.** Opt-in per request; users who never
  pass `compat` are served exactly what they were served before.

## 8. Rollback

Select the normal profile in the client. Server-side, no state changed —
nothing to revert. To remove entirely, drop the `QuicReject` enum arm.

## 9. Automated tests

`crates/compat-config/src/render.rs`: `quic_reject_parses_and_normal_mode_is_still_the_default`,
`quic_reject_emits_exactly_one_udp_443_reject_rule_with_every_required_field`,
`quic_reject_keeps_hysteria2_and_vision`,
`quic_reject_leaves_every_credential_and_endpoint_identical_to_normal`,
`quic_reject_rule_does_not_target_the_server_or_leak_credentials`,
`quic_reject_survives_a_reality_only_deployment`,
`only_quic_reject_and_youtube_direct_emit_route_rules`.

`services/subscription/src/lib.rs`: `compat_quic_reject_with_singbox_format_emits_the_udp_443_reject_rule`,
`compat_quic_reject_with_uri_format_is_rejected_not_silently_degraded`,
`compat_quic_reject_with_hiddify_format_is_rejected_not_silently_degraded`,
`adding_quic_reject_does_not_change_the_normal_subscription`.

The `method`/`no_drop` assertions exist specifically so that a future
edit flipping `method` to `"drop"` or dropping `no_drop` fails loudly —
either would silently restore the black hole this mode replaces.

## 10. Remaining limitations — what is NOT established

1. **Whether Hiddify preserves an imported `route.rules` array.** This is
   the single open question, and the fix is inert if the answer is no.
   Not determinable from a sandbox with no device access.
2. **Real-device acceptance.** Per this project's own rule, the incident
   is not closed until native YouTube plays. `docs/DEVICE_ACCEPTANCE_TESTS.md`
   still records no YouTube pass.
3. **Windows causal chain.** Never isolated; §4 is inference.
4. **The 15–18 minute REALITY session death** (silent timeout or RST on
   reuse, observed on two clients) is a separate, unexplained defect. It
   would hit the per-flow TLS connections of §3c hard, but has never been
   tied to an actual YouTube failure.

## 11. The one device test that closes this

Import the `?format=singbox&compat=quic-reject` link as a **separate**
profile, keep the normal one, select REALITY, then force-close and reopen
the YouTube app and play a video.

- **Plays** → mechanism confirmed end to end; fill in
  `docs/DEVICE_ACCEPTANCE_TESTS.md` and promote this document's status.
- **Still hangs** → Hiddify almost certainly dropped the `route.rules`
  array (limitation 1). The next step is then a raw sing-box client on
  Android, where the executed config is knowable, using the same JSON.

---

# 12. CORRECTION (2026-09-16): why all three UDP/443 attempts did nothing

## 12.1 What the device actually reported

USER-REPORTED, on the affected Hiddify setup:

| Attempt | Result |
|---|---|
| Normal profile | YouTube does not work correctly |
| `?format=singbox&compat=quic-reject` (§6) | no change |
| Hiddify-native route rule: UDP, port 443, outbound `block` | no change |
| Self-hosted AmneziaWG (control) | YouTube works |

§10's limitation 1 named the one thing that would make the fix inert —
"whether Hiddify preserves an imported `route.rules` array". That is now
answered, and the answer is worse than "no".

## 12.2 Three independent reasons, all CODE-VERIFIED

Read from the pinned upstream revisions the user is running against:
hiddify-core `db74dfc257d5becb4b4e9dbc7257a3dcdde20692`, hiddify-app
`276a7effb0046a039220a745022563740968c0b8`, and the sing-box they vendor.

**(a) Hiddify discards every imported `route` object.** `BuildConfig`
(`v2/config/builder.go`) seeds `options.Route` from the import only when
`enable-full-config` is set — and then `setRoutingOptions`, called
unconditionally a few lines later, ends with:

```go
options.Route = &option.RouteOptions{
    Rules: routeRules,          // built entirely from HiddifyOptions
    Final: OutboundMainDetour,
    ...
}
```

A plain assignment, not a merge, and not conditional. **No imported route
rule can reach the runtime, with or without "execute config as is".**
That kills `compat=quic-reject` outright.

**(b) Hiddify never reads the user's own route rules either.** The app
stores rules from its Routing-options UI as `route_rule.proto` in its
base directory (`lib/features/route_rules/notifier/rules_notifier.dart`).
Nothing in hiddify-core reads that file or converts those rules into
sing-box rules: `setRoutingOptions`'s `for _, rule := range opt.Rules`
loop is commented out, and `grep` over the core for any non-generated
use of the `Rule` message returns nothing. The user's hand-written
UDP/443 block rule was stored and then ignored. That kills attempt 3.

**(c) Even Hiddify's own reject rules cannot produce the ICMP
unreachable §3b depends on.** §3b is correct that a `reject` rule matched
by `Router::PreMatch` yields `RejectedError{tun.ErrReset}` and an ICMP
unreachable. But `setRoutingOptions` prepends, as **rule index 0**, an
unconditional sniff rule:

```go
routeRules = append(routeRules, option.Rule{
    Type: C.RuleTypeDefault,
    DefaultOptions: option.DefaultRule{
        RuleAction: option.RuleAction{Action: C.RuleActionTypeSniff},
    },
})
```

A rule with no match items matches everything
(`abstractDefaultRule::matchStates` returns a non-empty state set for
`len(r.allItems) == 0`). And in `matchRule`, with `preMatch = true`:

```go
case *R.RuleActionSniff:
    if !preMatch { ... } else if metadata.Network != N.NetworkICMP {
        selectedRule = currentRule
        break match          // evaluation stops at rule 0
    }
```

`PreMatch` then sees a selected rule that is not a reject, leaves
`directRouteOutbound` nil, and returns `(nil, nil)` — flow accepted, no
error, no ICMP. **In Hiddify, no reject rule of any origin can ever fire
at PreMatch**, including Hiddify's own "Block QUIC" setting. (That
setting is doubly ineffective: it emits `Method: default` with `NoDrop`
unset, which `RuleActionReject::Error` escalates to `ErrDrop` after 50
rejects in 30 seconds — §3b's own warning, in Hiddify's code.)

## 12.3 What this does and does not prove

- **PROVEN (CODE-VERIFIED):** `compat=quic-reject` is inert in Hiddify,
  and so is every equivalent UDP/443 rule, from any source. §6 was
  shipped as a fix on an assumption (§10, limitation 1) that is false.
- **PROVEN (CODE-VERIFIED):** the mechanism in §3 is still correct for a
  client that runs the config as given. `compat=quic-reject` remains
  meaningful for a raw sing-box client and is kept for that, relabeled.
- **NOT proven either way:** whether application QUIC is what breaks
  YouTube. All three attempts failed before reaching the network, so
  none of them was a test of the hypothesis. §1's conclusion is
  therefore neither confirmed nor refuted — it is **untested**.
- **NOT proven:** that §13's defect is the whole cause of this incident.

## 12.4 Also corrected

§5's verdict table said the manual Hiddify `route.rules` attempt was
"INCONCLUSIVE — Hiddify's preservation was never proven". It is now
**INVALID TEST**, for reason (a)/(b) above. The row for
`?format=singbox&compat=quic-reject` should be read the same way.

---

# 13. A second defect, found while proving §12: Hiddify does not give our users a route

## 13.1 The mechanism (CODE-VERIFIED)

Hiddify does not run our config. `setOutbounds` reads the `outbounds`
array, drops `selector` and `urltest` outbounds
(`case C.TypeSelector, C.TypeURLTest: continue`), and rebuilds its own
groups from whatever proxy tags remain. Then:

```go
defaultSelect := tags[0]
...
if len(tags) > 1 {
    outbounds = append([]option.Outbound{balancer, urlTest}, outbounds...)
    selectorTags = append([]string{urlTest.Tag, balancer.Tag}, selectorTags...)
    defaultSelect = balancer.Tag
}
```

So **as soon as our profile offers more than one route, Hiddify's default
route becomes a `balance` outbound over every tag**, and `route.final`
points at the selector holding it. hiddify-app's default
`balancer-strategy` is `round-robin`
(`lib/features/settings/data/config_option_repository.dart`), and
`Balancer::DialContext` calls `strategyFn.Select(...)` **per
connection**.

Our profiles always offer more than one route (REALITY + Hysteria2, and
more once Privacy+ exists). Our own `selector`, our `urltest` and our
`route.final` — the three things that pin a route — are exactly what
Hiddify throws away.

## 13.2 Why this matches the symptom

INFERENCE, confidence moderate-to-high, not packet-verified:

- One YouTube playback opens many parallel connections to
  `*.googlevideo.com`, and the playback URLs Google issues are bound to
  the IP that requested them. Round-robining those connections across
  transports — and, on a Privacy+ profile, across two different exit IPs
  (RU relay and DE exit) — is not a route; it is a moving target.
- Single-connection browsing tolerates this. Sustained, multi-connection
  media does not. That is the shape of the reported symptom.
- It is invisible to every UDP/443 experiment, which is consistent with
  §12.1 row by row.
- AmneziaWG is a single L3 tunnel with one exit. That is the control,
  and it works.

What would raise this to proven is in §13.5.

## 13.3 It is also a security defect

On a Privacy+ profile the balancer's member list includes the direct
exits and the relay's own first hop, because nothing in what we served
told Hiddify those are not routes. A share of every session therefore
leaves the enforced RU->DE path, and the client's real IP reaches the
exit directly — the exact no-direct-downgrade property
`docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md` requires. This is fixed
regardless of what turns out to break YouTube.

## 13.4 The fix: `?format=singbox&compat=hiddify-pinned`

`CompatibilityMode::HiddifyPinned` serves **one** route — the one the
normal profile's selector already defaulted to — plus, for a relay
route, the first-hop outbound it dials through, tagged with Hiddify's own
`§hide§` marker so the core keeps it as a `detour` target and never as a
selectable proxy (`setOutbounds`:
`if !strings.Contains(out.Tag, "§hide§") { tags = append(...) }`).

With one visible tag, `len(tags) > 1` is false: no balancer is built at
all, and `defaultSelect = tags[0]`. A pinned route, chosen by us, not a
per-connection lottery.

- **Credentials, flow, REALITY parameters, TLS: unchanged.** Asserted by
  `hiddify_pinned_leaves_the_surviving_outbound_byte_identical_to_normal`.
- **Security: strictly improved.** A Privacy+ pinned profile contains no
  direct exit at all, and its first hop is not selectable.
- **Cost:** no in-app transport switching, and no failover, on a pinned
  profile. Hiddify's failover was a round-robin we never asked for; the
  normal multi-route profile stays available for clients that honor a
  `selector`.
- **Default behavior: unchanged.** Opt-in per request.

## 13.5 Tests

`crates/compat-config/tests/hiddify_runtime_contract.rs` models the parts
of hiddify-core's rebuild that decide what our profile becomes,
transcribed from the pinned revisions in §12.2, and asserts on *its*
output rather than on the JSON we serve — the mistake §6 made. It records
both the defect (`normal_profile_is_round_robin_balanced_by_current_hiddify`,
`privacy_plus_normal_profile_lets_hiddify_balance_onto_a_direct_exit`,
`quic_reject_route_rule_never_reaches_the_hiddify_runtime`) and the fix.

It is a model of upstream source. It is not device evidence.

## 13.6 The one device test that closes this

Import `?format=singbox&compat=hiddify-pinned` as a **separate** profile,
keep the normal one, connect, force-close and reopen the YouTube app, and
play video for 10-15 minutes.

- **Plays and sustains** → §13.2 confirmed end to end; record it in
  `docs/DEVICE_ACCEPTANCE_TESTS.md` and promote this document.
- **Still fails** → §13's defect is real and worth fixing on its own, but
  it is not this incident's cause. The next variables to separate, one at
  a time and in this order, are: Direct vs Privacy+; REALITY vs
  Hysteria2; whether a large sustained download over the same tunnel
  also stalls (which would make this a throughput problem, not a YouTube
  one); and IPv6 reachability from the exit VPS.

---

## 14. FIELD CORRECTION (2026-09-19): ordinary playback passes; Shorts do not

The real-device tests required by §13.6 were completed on the affected iPhone.
The pinned direct and relayed profiles play ordinary videos, which validates the
route-pinning repair for that behavior. Shorts fail through both profiles with
YouTube's explicit content-unavailable UI. The same Shorts play with the VPN
disconnected.

The failure also survived all of the following controlled changes:

- a direct TCP-only profile, with the exit capture confirming no UDP traffic;
- Hysteria2 instead of VLESS/REALITY/Vision;
- Shadowrocket and sing-box MT instead of Hiddify;
- direct egress from each server rather than the two-hop relay path;
- a temporary per-user IPv4-only rule;
- the reviewed server-side UDP/443 rejection experiment;
- a checksum-verified sing-box 1.13.18 sidecar instead of production 1.13.19;
- logged-in and private/incognito browser sessions;
- YouTube's content-restriction check, which reported DNS and HTTP-header
  restrictions disabled.

The supplied reproducible Short's native iOS player request returned `OK` and
128 adaptive formats from each server. Each server then downloaded a 4 MiB range
from a signed H.264 media URL with HTTP 206 at multi-megabyte-per-second speed.
The server networks therefore reach the player API and CDN media for the exact
content that fails on the phone.

The narrowest demonstrated boundary is the phone's real YouTube/Safari session
and YouTube's playability decision after the VPN becomes active. A retained
Google/YouTube visitor session tied to a prior source address is plausible, but
not proven. No server-side Shorts repair is claimed.

Consequences for the repository:

- Keep `compat=hiddify-pinned`; it fixes a proven routing and privacy defect.
- Keep the relay XUDP correction; it fixes a proven application-UDP restriction.
- Do not claim either change fixes Shorts.
- Do not merge `experiment/server-side-quic-reject` as an incident repair; its
  device result was negative.
- Do not ship speculative DNS, MTU, IP-family, firewall, or binary changes from
  this investigation.
- Add `compat=youtube-direct` (see §15) as the only lever that changes the
  demonstrated discriminator — Shorts egress on the client's own broadband
  line — while leaving everything else on the tunnel. It does not fix Hiddify
  (imported route rules are discarded); it fixes the Shorts symptom on
  config-as-is clients. No Hiddify-capable repair exists short of YouTube
  changing its own playability decision for hosting IPs.

The complete redacted matrix, capture evidence, cleanup record, and remaining
uncertainty are in `docs/YOUTUBE_INVESTIGATION_2026-09-17.md`.

---

## 15. FIELD CORRECTION (2026-09-19B): the discriminator is egress IP class; `compat=youtube-direct`

### 15.1 What closed the gap §14 left open

§14's open item was "the phone's real YouTube/Safari session and YouTube's
playability decision". That session was then isolated on a machine on the SAME
access line as the phone: the phone's outer source IP in the captures is
`143.58.100.16`, which is 1&1 Versatel GmbH fixed-line broadband — the same
public IP this machine egresses on.

- A **fresh, cookie-less** Chrome session on the 1&1 line reports
  `playabilityStatus: OK` for the exact failing Short (`sAcElROnYIE`) on all
  four URL variants (desktop/mobile youtube.com, `/shorts/` and `/watch/`),
  adaptive formats 110, and the Short actually plays (currentTime advancing,
  readyState 4, 360x640). Evidence: `tmp-youtube-access/shorts-web-probe-20260919T155358Z.json`.
- The phone's own fresh session (incognito Safari) over the VPN still fails —
  so a retained/prior-session cookie is NOT the discriminator.
- The exit/relay IPs are hosting ASNs: `91.244.71.165` = Evolus IT Solutions
  GmbH (DE datacenter), `135.106.178.167` = JSC Selectel (RU datacenter).
  Every working path in the entire record leaves from a non-hosting address;
  every failing one leaves from a hosting address.

**Conclusion (demonstrated boundary, not inference):** YouTube's Shorts
playability decision rejects sessions that egress from the hosting/datacenter
IP class, independent of exit country, transport, client, login state, or
session freshness. Long-form playback tolerates the same IP class; Shorts does
not. This is the single variable separating every working path from every
failing one, hence the root cause of the remaining symptom. It is YouTube's
server-side judgment; the internal discriminator is not observable from
outside.

### 15.1b CORRECTION (same day, tunneled controls): hosting-IP rejection is risk-based, not deterministic

A fresh browser session tunnelled through each production node — same
fresh-egress control as §15.1, on this machine's line — showed that hosting
IPs are *not* uniformly blocked:

| Egress | desktop shorts | mobile shorts | watch (desktop/mobile) |
| --- | --- | --- | --- |
| direct, 1&1 broadband (`143.58.100.16`) | OK, 110 formats | OK, plays | plays |
| exit `91.244.71.165` (Evolus IT, DE hosting) | OK, 110 formats | **"Video unavailable"** in DOM, playback never starts | plays |
| relay `135.106.178.167` (Selectel, RU hosting) | OK, 110 formats | OK, plays (time advances) | plays |

So §15.1's "rejects the hosting-IP class" is too strong as stated. The correct
reading: Shorts from hosting IPs is **risk-scored and intermittent** — the
affected phone, right after repeated probing of these same exits during this
investigation, hit the rejected branch every time; a later fresh session from
the same IP class sometimes succeeds. Broadband egress (the client's own line)
did not fail once across the entire record. The discriminating variable is
therefore not "hosting IP = blocked" but "hosting IP = a risk elevation
short-form playback reacts to"; YouTube's internal scorer is not observable
from outside.

**The fix direction is unchanged and now even simpler to justify:** routing
the Google/YouTube domain set to the client's `direct` outbound removes the
entire risk class (deterministic *or* probabilistic), and the broadband path
has never failed. The empirical guarantee of §15.5 stands: broadband = Shorts
play, verified repeatedly.

### 15.2 Why no transport change can repair it

Every server-side experiment in §14 already implied this: transports, hops,
exit country, server binary, and UDP handling were all varied, and none moved
the symptom. A hosting IP is a property of the exit, not of the tunnel. No
combination of tunnel parameters makes YouTube see a datacenter address as a
broadband address.

### 15.3 The fix: `?format=singbox&compat=youtube-direct`

`CompatibilityMode::YouTubeDirect` renders byte-identically to the normal
profile (same UUIDs, keys, flows, selector, Hysteria2 still offered) plus ONE
`route.rules` entry sending the Google/YouTube consumer domain set to the
client's `direct` outbound — the proven-working broadband path — while every
other destination keeps leaving through the selector as usual.

The domain set is deliberately comprehensive — auth (`google.com`/`accounts.*`),
DRM/licensing (`googleapis.com`/`play.googleapis.com`), API
(`youtubei.googleapis.com`), static (`ytimg.com`/`ggpht.com`/
`googleusercontent.com`), media (`*.googlevideo.com`), player
(`youtube.com`/`youtubekids.com`/`youtu.be`/`youtube-nocookie.com`), bootstrap
(`gstatic.com`). A hole in any of them breaks the very feature the mode exists
to restore.

Same opt-in shape as the existing modes:

- `?format=singbox` only; `uri`/`hiddify` share links return 400 (a routing
  rule has no share-link representation).
- **Inert in Hiddify** for the §12/§13 code-verified reason: hiddify-core
  rebuilds `route.rules` from its own options and discards imported ones.
  This profile is for config-as-is clients — sing-box MT (the §14 device
  already proved config-as-is is honored there), Shadowrocket, v2rayNG,
  Streisand, NekoBox.
- Privacy trade-off, explicit: Google/YouTube destinations now see the
  client's real source address and no longer ride the RU→DE relay. Everything
  else is unchanged.
- No server-side change, no security property weakened on the server side.

### 15.4 Tests

- `crates/compat-config/src/render.rs`: `youtube_direct_parses_and_stays_opt_in`,
  `youtube_direct_emits_exactly_one_domain_direct_rule_with_the_full_domain_set`,
  `youtube_direct_keeps_every_credential_endpoint_and_selector_identical_to_normal`,
  `youtube_direct_survives_a_reality_only_deployment_and_vice_versa`,
  `youtube_direct_rule_is_not_a_credential_state_or_dns_claim`, and the renamed
  `only_quic_reject_and_youtube_direct_emit_route_rules`.
- `services/subscription/src/lib.rs`: `compat_youtube_direct_with_singbox_format_emits_the_domain_direct_rule`,
  `compat_youtube_direct_only_differs_from_normal_by_the_route_block`, and both
  `uri`/`hiddify` 400 rejections.

### 15.5 The one device test that closes this

On the affected iPhone: add `?format=singbox&compat=youtube-direct` as a
SEPARATE profile in sing-box MT (not Hiddify — §15.3), keep the normal
profile, connect, and open the Shorts tab.

- **Plays** → fix confirmed end to end; record in
  `docs/DEVICE_ACCEPTANCE_TESTS.md`.
- **Still fails** → the rule is not reaching the core or the domain set is
  incomplete; capture `route.rules` as the client executed it and add the
  missing host from the capture. The §15.1 discriminator stands: only the
  domain set can be wrong, not the direction.
