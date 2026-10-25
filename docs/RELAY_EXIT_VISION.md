# Relayed exit route without XTLS Vision (`vless_flow = "none"`)

Status: opt-in, **not default**. Added after `v1.3.1-rc.6` (needs a new candidate; rc.6 does not contain it).
Real-hardware validation of the fix itself is still open (see "What is and is not verified").

## Problem observed (2026-10-25, live hosts)

Chain: client -> relay (ru1, VLESS+REALITY+Vision) -> exit (Evolus, VLESS+REALITY+Vision) -> Internet,
built from `?format=singbox&compat=hiddify-pinned` (the exit outbound has `detour` = the relay hop).

| Observation | Result |
|---|---|
| curl, HTTP/1.1, basic connectivity, bulk non-Google transfers | worked |
| Chromium HTTP/2 to `m.youtube.com` | hung, 6/6 runs; client log `tls: received record with version 28d8 when expecting version 303` |
| Chromium with HTTP/2 disabled | worked |
| Vision off on the first (relay) hop only | still hung (3/3) |
| Vision off on both hops | worked (3/3) |
| Vision off on the nested exit hop only, relay hop keeps Vision | worked (3/3) |

The same browser and machine loaded the page normally through fin1's single-hop path.

## What is and is not verified

- Verified on live hosts: the A/B above (browser, repeated runs, experiments reverted afterwards).
- **Not reproduced in isolation.** `deploy/lib/lab/nested_vision_lab.sh` builds relay, exit and client with the pinned
  sing-box 1.14.1 on loopback and drives 16 parallel HTTP/2 streams (curl and headless Chromium); a four-node variant
  adds a Google hairpin hop, production-like `sniff` rules, injected latency and re-segmentation, and loads the real
  YouTube page. Every variant, including Vision/Vision, passed. So the lab proves the Vision-less shape works; it does
  not reproduce the failure, and **the root cause is unclassified** (generated configuration, upstream sing-box, or an
  interaction with the live network path or the Google hairpin/spoof hop). Do not call it an upstream defect.
- Not verified: the fix on the live chain after this change, native iPhone/Android behaviour, Hiddify app UI.

## Mechanism

sing-box's VLESS server compares the client's flow with the flow configured for that user exactly; a mismatch is
rejected. Therefore the client and server must change together:

1. Relay `deployment.toml`: the relayed route declares `vless_flow = "none"` and its **own** credential
   (no `credential_ref`; a shared credential would force the direct route to the same flow, and validation refuses it).
2. Exit: create a user dedicated to this route and run `vpn-admin user vision-off-experiment <id>` on the exit.
   That user's inbound entry renders `"flow": ""`; every other user keeps `xtls-rprx-vision`.
3. Relay: `vpn-admin user peer set <relay-user-id> <via-route-id> --credential-stdin` with that exit user's UUID.

The relay's own first hop is untouched (Vision on). The relay forwards opaque TCP to the declared exit and never
learns the exit flow, so its server document is byte-identical with or without the setting.

## Guard rails (validated in `deployment.rs`, tested in `relay_role_policy.rs`)

- `vless_flow = "none"` only for `vless_reality` and only on a relayed route (`path` other than `direct`).
- A route sharing a credential through `credential_ref` must have the same `vless_flow` as the route that owns it.
- Default (`vision`) serializes and renders exactly as before; the endpoints fingerprint is unchanged.
- `vpn-admin doctor` on a relay reports each Vision-less route as an INFO line (the exit side cannot be checked from the relay);
  `user peer set` prints a reminder when the route is Vision-less.

## Security consequences

- Vision exists to hide the TLS-in-TLS pattern of a proxied TLS session. Without it on the exit hop, that pattern is
  visible to an observer of the **relay -> exit leg** (the Russia-to-Europe leg), unless something else hides it. The
  client -> relay leg keeps Vision. This is more fingerprintable than production and has no network-side evidence
  either way; it worked on the live leg on 2026-10-25, which does not predict how long it keeps working.
- The credential is dedicated: revoking, rotating or disabling it on the exit (or removing the relay user) stops
  the route without touching other users. A Vision-less credential must never be given to an ordinary user.
- No fallback is introduced: if client and server flows disagree the connection fails; there is no direct route in
  the relayed profile.

## Operations

Rotation: rotate the exit credential (`vpn-admin user rotate-vless <exit user>` keeps the flow setting), then
`user peer rotate` on the relay. Revocation: disable the exit user; the relay route then fails closed.
Reconciliation, `render-config` and `update.sh --repair` re-render both sides from the stored settings
(`deployment.toml` and `users.json`), so the behaviour persists; confirm after an upgrade with the doctor INFO line
and a Chromium HTTP/2 request through the chain.
