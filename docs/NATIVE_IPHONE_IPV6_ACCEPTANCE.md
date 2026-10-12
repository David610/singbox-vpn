# Exact-candidate native iPhone IPv6 / Shorts acceptance

Status: **PENDING — BLOCKED_ON_DEVICE_EVIDENCE**. No native iPhone was available
to this coding task. Local core assertions/browser emulation cannot fill this
record. Keep older hand-patched v1.1.0 reports separate from exact RC evidence.

Record only non-secret build/device details: exact candidate tag and SHA,
sing-box version, role and policy, iPhone model, iOS, YouTube, Hiddify and
active core versions. Record Wi-Fi/cellular and selected transport without
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
3. Play `sAcElROnYIE` and at least five consecutive Shorts, then a long video
   for at least a minute. Test seeking, 15 minutes sustained playback, a
   disconnect/reconnect, client reboot, and an authorized server reboot.
4. Capture enough timing/family evidence to distinguish TCP vs destination
   QUIC, literal vs domain, native IPv6 vs mapped inbound peers and actual
   IPv4 retry. A sampler cannot correlate a particular failed Short. Test
   absent SNI/ECH/fragmented cases separately if observed; do not invent them.
5. Verify ordinary non-Google IPv6/IPv6-only behavior, client-ISP IPv6 leaks,
   DNS leaks and TLS renewal/listener state. An IPv6 VPS exit is not itself
   an ISP leak; A-only server DNS does not force destination IPv4.
6. Independently regression-test the existing accepted-IP hairpin, preserving
   its target, dedicated credential, probe confinement and fail-closed route.
   Do not re-enable its intentionally disabled agent or change owner decisions.

| Gate | REALITY Wi-Fi | REALITY cellular | Hysteria2 Wi-Fi | Hysteria2 cellular |
|---|---|---|---|---|
| Exact candidate/build/profile recorded | PENDING | PENDING | PENDING | PENDING |
| Shorts + long video + seeking | PENDING | PENDING | PENDING | PENDING |
| 15-minute session | PENDING | PENDING | PENDING | PENDING |
| Actual retry/family correlation | PENDING | PENDING | PENDING | PENDING |
| Reconnect/client reboot/server reboot | PENDING | PENDING | PENDING | PENDING |
| DNS/ISP IPv6 leak captures | PENDING | PENDING | PENDING | PENDING |

Only actual successful exact-candidate device evidence permits
YOUTUBE_SHORTS_FIXED_ON_IPHONE. RELEASE_READY additionally requires full exact
RC CI, fresh disposable lifecycle, v1.2.0 -> candidate update/rollback and TLS
renewal evidence. A release or production deploy needs separate owner approval.
