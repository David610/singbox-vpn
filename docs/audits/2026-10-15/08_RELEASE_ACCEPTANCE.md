# 08 - Release acceptance

## Decision

**NOT READY to declare any of this production-qualified.** The server-side defects that could be
reproduced are fixed on branches and verified on a real VPS, but there is no physical-device
evidence for any customer client, the stable release is behind `main`, and two findings (own-address
isolation in the installer, Hysteria2 coverage in health checks) are still open. Recommended path:
merge reviewed branches, cut a corrected stable (v1.3.1), run the VPS acceptance workflow on that
exact tag, then do the iPhone tests below. Do not rely on `v1.3.0` for Hysteria2 caps or burst
user changes.

## Candidate identity

| Item | Value |
|---|---|
| Deployed build | `singbox-vpn:fix/shadowrocket-base64-subscription-link` at **c1c0a7f** (main 56193f9 + a54a2e9 + c1c0a7f), dev channel, built on the VPS by the installer |
| Deployed binaries contain | `subscription_url_shadowrocket` and the `reset-failed` change (checked in the installed `vpn` and `vpn-admin`) |
| Later commits, **not deployed** | 63ad42c (doctor own-address fix), 40d2f4d (docs). Verified by unit tests and CI, not on the VPS |
| Manual config applied on the VPS after install | `public_ipv4`/`public_ipv6` in `deployment.toml` (the doctor's own advice) |
| sing-box | 1.14.1 pinned |
| Provenance | dev channel = checksum-**unverified** branch source; not a release artifact. The doctor correctly warns `release identity INCONSISTENT` |
| vpn-web | PR #83 at 4970a82, **not deployed anywhere** (no staging environment was available; production Pages is not a disposable resource). Verified by unit/endpoint tests and a real-node loopback test of the link shape |

## Gates

| Gate | Result |
|---|---|
| Stable v1.3.0: install, hashes, doctor | PASS |
| v1.3.0 -> v1.3.1-rc.1 update, injected-failure rollback, reboot, uninstall | PASS |
| Candidate burst of 12 restarts, no outage | PASS |
| Candidate Shadowrocket Base64 link correct and same credential | PASS (server) |
| vpn-web Hysteria2 link works on a real node | PASS (loopback variants); endpoint tests PASS |
| vpn-web full suite, tsc, eslint | PASS (before the last two commits; CI green on 4970a82) |
| singbox-vpn CI | green on c1c0a7f; 40d2f4d in progress when written |
| Hysteria2 / QUIC / UDP with a real client | BLOCKED (test network filters UDP/443) |
| YouTube Shorts | FAIL for this exit IP (not a protocol defect); no hairpin credentials |
| IPv6 leak, DNS leak, kill switch | NOT_TESTED |
| Hiddify iOS / Shadowrocket / INCY on a real iPhone | NOT_TESTED |
| Installer records own public address (S-1) | FAIL (open) |
| Interrupted install recovery; disk/memory/CPU pressure | NOT_TESTED |

## Answers to the completion questions

**A. Hiddify.** The missing file is a client bug (purgeable cache vs persistent database, no
existence check); the core-start failure is not reproduced and no server cause was found: Hiddify's own
current core and v4.0.0 both accept every server representation, and the current core runs them.
Fixed: nothing server-side is needed. Still needs a device: the iOS app on current stable, with its log.

**B. Shadowrocket.** Plain-text bodies are documented as rejected. Formats now exposed: Base64 link
(singbox-vpn) and `shadowrocket`+`encoding=base64` (vpn-web). Corrected vpn-web Hysteria2 links are usable.
Not yet imported on a real client.

**C. INCY.** Not diagnosed. The only evidence is a third-party bug report about VLESS-REALITY-only
subscriptions and the fact that both repos declare INCY VLESS-only. The vpn-web `xray` format is not a valid
Xray config.

**D. VPS.** Now running the dev build c1c0a7f (not a published artifact). The stable v1.3.0 matched its
published artifact exactly. Survives reboot, uninstall/reinstall and an injected update failure.

**E. Networking.** REALITY, DNS and IPv4 work; IPv6 egress works; QUIC/UDP/Hysteria2 and leaks are not
verified; streaming fails for this IP.

**F. Reliability.** One user's change can interrupt everyone (7 of 10 operations) and, before the fix,
could cause a multi-minute outage by itself. The outage is fixed; the interruption is inherent to sing-box and
documented.

**G. Consistency.** The vpn-web external renderer disagreed with the node (SNI, obfuscation): fixed. Docs
disagreed on the archived client: fixed in the PR. The doctor disagreed with its own advice: fixed.
Remaining: installer vs doctor on own-address isolation; Hysteria2 absent from health checks.

**H. Performance.** No regression measured: tunnel adds ~47 ms TTFB; server capacity > 1 Gbit/s on loopback at
<25 % CPU. Upload/UDP/long-session untested.

**I. Remaining risks.** See `issues.json` (open and blocked entries). Missing evidence: any physical-device
run, a network with working UDP/443, the second node for the YouTube hairpin.

## iPhone test procedure (needs you)

Use a throwaway user: `vpn user create --name iphone-test --json` on the node. Photograph or screenshot each
numbered result; do not send the URLs or tokens.

1. Hiddify **4.1.1** (App Store/TestFlight stable). Add the `subscription_url`. Screenshot the profile list.
   Tap Connect. Screenshot the result and Settings -> Logs (last ~30 lines, with any token blanked).
2. Open Safari to a "what is my IP" page: expect 91.244.71.165. Open the YouTube *app*: play 3 normal videos and 10 Shorts
   (expect Shorts to fail for this IP; note that it is the IP, see `05`).
3. Delete the profile in Hiddify, force-quit, re-add the link. Note whether "no such file" ever reappears and whether
   it follows a reboot, an app update, or a low-storage warning.
4. Shadowrocket: add `subscription_url_shadowrocket`. Screenshot the import result (servers listed or an error).
   Connect and repeat step 2's IP check. Then add the plain `subscription_url` and note whether it is rejected.
5. INCY: add `subscription_url`; screenshot the provider (server count). Then add the Base64 link; then a link containing only the
   Hysteria2 line (take it from `vpn user links <id>`). Note which show servers.
6. Switch Wi-Fi to cellular while connected; note reconnect time. Repeat once on an IPv6-only/NAT64 cellular network if available.
7. For Hysteria2 on a phone: select it explicitly in Hiddify and note connect time and whether browsing works.

Delete the throwaway user afterwards (`vpn user remove <id>`).
