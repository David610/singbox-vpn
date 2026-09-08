# YouTube native-app failure — root cause and fix (2026-09-08)

**Status: mechanism PROVEN from upstream source; fix IMPLEMENTED and
opt-in; real-device acceptance NOT YET RECORDED.**

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
`only_quic_reject_emits_route_rules`.

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
