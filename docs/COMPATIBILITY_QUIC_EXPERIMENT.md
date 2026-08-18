# QUIC compatibility profile — feasibility research and decision (P10)

Investigated whether the generated Hiddify/sing-box subscription could
safely offer an optional fourth profile — alongside Reality, Hysteria2,
and Auto — that forces application-level UDP/443 (QUIC/HTTP3) to fail so
QUIC-capable apps fall back to TCP, as a controlled experiment for
isolating whether application-level QUIC is involved in the reported
"Safari plays YouTube, the native YouTube app does not" symptom.

**Decision: not implemented in the subscription renderer.** The
mechanism is real and expressible in current sing-box syntax (verified
below), but this project cannot verify the one precondition that
actually matters — whether Hiddify preserves it — from this development
environment, and shipping it anyway would silently cross a boundary this
project has deliberately held and tested since the compatibility stack
was built. See "Why not" below for the specific, itemized reasons, and
"Safe alternative" for what to do instead.

## What is technically real

sing-box's route-rule syntax genuinely supports exactly this shape.
Current (sing-box >= 1.13) `route.rules` syntax:

```json
{
  "route": {
    "rules": [
      { "network": "udp", "port": 443, "action": "reject" }
    ],
    "final": "select"
  }
}
```

This is sing-box's own documented pattern for making a QUIC-preferring
application's UDP/443 attempt fail outright (not silently drop — reject,
so the application observes a real connection failure and — if it is
well-behaved, as browsers generally are and some native apps are not —
falls back to TCP/443, which would then be carried over whichever
outbound `route.final` points at). Verified against current
`sing-box.sagernet.org/configuration/route/rule/` and
`.../rule_action/` documentation, 2026-08.

Mechanically, a rule like this operates on **application traffic routed
through the client's own TUN inbound** — it decides which outbound tag
handles a given (destination, protocol, port) tuple from apps on the
device. It is a different mechanism from the **outbound transport
connection** sing-box itself makes to reach this deployment's VPS (the
REALITY TCP/443 or Hysteria2 UDP/443 dial). Reason through why that
matters for each transport:

- **REALITY**: the VPN transport connection is TCP/443 to the VPS. A
  `{"network":"udp","port":443,"action":"reject"}` rule would not touch
  it at all — only applications' own outbound UDP/443 attempts. This is
  the transport a "Compatibility" profile would need to use.
- **Hysteria2**: the VPN transport connection to the VPS itself IS
  UDP/443 (that is the entire protocol). A blanket UDP/443-reject rule
  would have to explicitly exclude the deployment's own Hysteria2
  outbound (e.g. by destination IP/port pinned to this VPS specifically,
  not port alone) or it would sever the tunnel's own transport, not just
  application QUIC. This is achievable in principle (a `rule_set`
  keyed by destination rather than a blanket port match), but it raises
  the design's complexity and blast radius further — one more thing that
  would need to be proven correct on a real device before being handed
  to users. This document does not attempt that variant.

## Why not

Every one of these would need to be true before this could ship as
something this project generates and claims to be a working
"Compatibility" mode. None of them can be established from this
environment:

1. **Whether Hiddify's iOS/Android client actually preserves an
   imported subscription's `route.rules` unmodified**, rather than
   discarding, overriding, or re-deriving its own routing from the
   `outbounds` list alone. This is exactly the kind of client-internal
   merge behavior `docs/CLIENT_PROTOCOL_BEHAVIOR.md` already documents
   as unverified for far simpler things (DNS, IPv6 preference) — there
   is no more reason to assume it here, and no real Hiddify install
   available in this environment to test against.
2. **Whether Hiddify's TUN mode is even active** for a given user at the
   moment this rule would need to apply — `route.rules` only affects
   traffic actually captured by a TUN inbound, which is entirely
   Hiddify's own "Service Mode" setting (`docs/clients/HIDDIFY_IOS.md`'s
   "Four different claims" table), not something the subscription can
   guarantee.
3. **Whether Hiddify's bundled sing-box core** (confirmed several minor
   versions behind this deployment's server-pinned version — see
   `docs/TELEGRAM_RESILIENCE_PLAN.md`'s 2026-08-11b addendum) parses
   this exact rule/action syntax identically to the version this was
   verified against.
4. **Blast radius beyond the one target app.** A blanket
   `network=udp, port=443` reject rule affects every application on the
   device that tries QUIC on port 443, not just the YouTube app — every
   other QUIC-capable app (other Google services, some CDNs, some game
   clients) would also be forced to fall back or fail, for the duration
   the profile is selected. That is an acceptable, disclosed cost for a
   deliberately-selected diagnostic mode a user opts into for a few
   minutes; it would not be acceptable as a silent default or as
   something represented as low-risk.
5. **This project's own tested boundary.** `crates/compat-config/src/
   render.rs`'s test suite (`client_subscription_has_no_dns_block_and_
   no_inbounds`, `client_subscription_profile_shape_is_explicit_and_
   minimal`) locks in, on purpose, that the generated subscription never
   expresses a routing opinion the server cannot verify or enforce —
   see `docs/CLIENT_PROTOCOL_BEHAVIOR.md`'s design rationale. Adding a
   `route.rules` array for one profile is a direct, deliberate exception
   to that boundary. Crossing it once, without being able to verify the
   client actually honors it, would produce exactly the kind of false
   confidence ("select Compatibility mode, problem should be gone") this
   whole effort is trying to eliminate, not add another instance of it.

Point 5 is enforced going forward, not just asserted here:
`render.rs`'s test suite now also asserts `route.rules` is absent from
every rendered profile (`client_subscription_never_emits_route_rules`),
so a future change cannot silently add this without deliberately
revisiting this decision.

## Safe alternative (what to do instead, today)

None of the above blocks a **manual, operator-run, clearly-labeled
diagnostic experiment** on a single already-cooperating test device —
only an automatically-generated, silently-trusted "Compatibility"
subscription profile that every user could select. The manual path,
using facilities that already exist:

1. Select the **Reality** endpoint manually (never Hysteria2, per the
   transport-conflict reasoning above).
2. If Hiddify's own UI exposes a per-profile "custom config"/raw JSON
   override (version- and platform-dependent — check the installed
   Hiddify build first), paste the rule above into it by hand, on the
   one test device, for the duration of the experiment only.
3. Retest YouTube-app playback.
4. Revert immediately by removing the custom override and reconnecting
   — this must never be left in place "to see if it helps" (same rule
   this project applies to every other experiment in the investigation
   this section follows from).
5. Record the result using `docs/DEVICE_ACCEPTANCE_TESTS.md`'s
   streaming/YouTube matrix, noting explicitly that "Compatibility mode"
   was a manual client-side override, not a vpn1-issued profile.

This gives the same diagnostic value (does forcing QUIC-preferring apps
onto TCP change the YouTube-app symptom?) without asking every user's
client to silently trust a routing rule this project cannot verify it
actually applies.
