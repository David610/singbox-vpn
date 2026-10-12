# Exact-candidate native iPhone acceptance (YouTube Shorts, TikTok, ordinary use)

This document is the **procedure and evidence template**. Per-release status lives
only in `DEVICE_ACCEPTANCE_TESTS.md` and `release-acceptance/<tag>.md`; until those
record a PASS for an exact RC, the result is **BLOCKED_ON_DEVICE_EVIDENCE**. Local
core assertions and browser emulation cannot fill this record. Keep older
hand-patched or earlier-release reports separate from exact RC evidence.

The test client is **Hiddify** (iOS) with its embedded core — the client the owner
actually uses — importing the normal HTTPS subscription and, separately, a direct
link from `vpn user links`. No website purchase, Tamara build or custom client is
required. Run a **smoke first** (REALITY on Wi-Fi, ~15 minutes: `sAcElROnYIE` and 3
more Shorts, one long video with a seek, TikTok cold start/feed/3 videos, one
TikTok web video, one ordinary site) to find a major failure before the full matrix.

Record only non-secret build/device details: exact candidate tag and SHA,
sing-box version, role and policy, iPhone model, iOS, YouTube, TikTok, Hiddify and
active core versions, and the import method. Record Wi-Fi/cellular and selected transport without
subscriber tokens, UUIDs, passwords, private certificates or identifiable
per-user IP capture dumps. Keep sensitive raw captures locally restricted;
attach redacted aggregate evidence only.

For each of REALITY and Hysteria2, separately on Wi-Fi and cellular:

1. Confirm the exact candidate serves the normal profile, profile title
   **Arcana VPN**, VPN/TUN enabled and selected transport. Record the domain/
   literal IPv4/IPv6 browser control separately from device outcomes.
2. Establish baseline `allow` on the IPv4-good/IPv6-refused standalone exit,
   then test explicit `reject_google`; measure actual native retry/recovery,
   not just a server close or Google generate_204 response. If scoped sniff
   fails, diagnostic reject_all needs separate owner approval for its IPv6-only
   breakage. Restore the prior policy after the test. No host IPv6 shutdown.
3. Play `sAcElROnYIE` and at least ten consecutive Shorts, then a long video
   for at least a minute. Test seeking, 15 minutes sustained playback, a
   disconnect/reconnect, idle/resume, client reboot, and an authorized server
   reboot. After `reject_google` closes a literal-IPv6 flow, confirm playback
   actually recovers; a correct rejection alone is not a pass.
4. Capture enough timing/family evidence to distinguish TCP vs destination
   QUIC, literal vs domain, native IPv6 vs mapped inbound peers and actual
   IPv4 retry. A sampler cannot correlate a particular failed Short. Test
   absent SNI/ECH/fragmented cases separately if observed; do not invent them.
5. Verify ordinary non-Google IPv6/IPv6-only behavior, client-ISP IPv6 leaks,
   DNS leaks and TLS renewal/listener state. An IPv6 VPS exit is not itself
   an ISP leak; A-only server DNS does not force destination IPv4.
6. **TikTok** (app and web), same four cells plus a VPN-off baseline: cold start,
   feed metadata, thumbnails, video start, at least ten consecutive videos, 15
   minutes of scrolling/playback, comments and profile views, applicable account
   access, and TikTok web playback. On a failure record the failing operation and
   hosts from restricted diagnostics and separate DNS, routing, TCP, destination
   QUIC/UDP, address family, MTU and service/exit restrictions (ledger "TikTok
   diagnostic procedure"). Do not assume the Google policy applies, invent domain
   lists, block UDP globally or disable host IPv6; diagnosis alone is not a pass.
7. Independently regression-test the existing accepted-IP hairpin, preserving
   its target, dedicated credential, probe confinement and fail-closed route.
   Do not re-enable its intentionally disabled agent or change owner decisions.

| Gate | REALITY Wi-Fi | REALITY cellular | Hysteria2 Wi-Fi | Hysteria2 cellular |
|---|---|---|---|---|
| Exact candidate/build/profile recorded | PENDING | PENDING | PENDING | PENDING |
| Shorts + long video + seeking | PENDING | PENDING | PENDING | PENDING |
| TikTok app (feed, thumbnails, 10 videos, 15 min, comments/profile) | PENDING | PENDING | PENDING | PENDING |
| TikTok web playback | PENDING | PENDING | PENDING | PENDING |
| Subscription refresh + direct link import | PENDING | PENDING | PENDING | PENDING |
| 15-minute session | PENDING | PENDING | PENDING | PENDING |
| Actual retry/family correlation | PENDING | PENDING | PENDING | PENDING |
| Reconnect/client reboot/server reboot | PENDING | PENDING | PENDING | PENDING |
| DNS/ISP IPv6 leak captures | PENDING | PENDING | PENDING | PENDING |

Only actual successful exact-candidate device evidence permits
YOUTUBE_SHORTS_FIXED_ON_IPHONE (and the equivalent TikTok statement). RELEASE_READY additionally requires full exact
RC CI, fresh disposable lifecycle, v1.2.0 -> candidate update/rollback and TLS
renewal evidence. A release or production deploy needs separate owner approval.
