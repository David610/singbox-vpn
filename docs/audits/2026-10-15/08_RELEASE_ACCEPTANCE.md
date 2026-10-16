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

## Round 2: server hardening candidate for v1.3.1-rc.2

**Decision: the four server defects are fixed and verified on a real VPS on the branch; this is NOT a
release acceptance.** It is not merged, not tagged, and has no physical-device evidence. The
production-updater rollback proof and the Hysteria2 path over a real network are still open (below).

| Item | Value |
|---|---|
| Branch | `fix/server-hardening-rc2` on `main` 0b038a0; tested at **f070601** (earlier: 10ad1b0) |
| Fixes | I-15 9ca903e, I-18 0c7118a, I-16 a4e8f16 + 10ad1b0, I-17 f070601 |
| Version | `Cargo.toml` stays `1.3.1`; `v1.3.1-rc.2` would be a tag only |
| Host | 91.244.71.165, Ubuntu 24.04.5, **disposable** (owner confirmed 2026-10-16); sing-box 1.14.1 pinned |
| How deployed | public bootstrap, dev channel, **pinned to the commit SHA**; source built on the host. Checksum-**unverified** branch source, not a release artifact |
| Baseline | unmodified `main` 0b038a0, fresh install on the same host, same domain |
| Client | real sing-box 1.14.1 for Windows (SHA-256 checked against GitHub's published digest), from a PC on the public internet |

| Gate | Baseline `main` | Candidate | Evidence (`test-results/`) |
|---|---|---|---|
| I-15 own addresses recorded on a domain install | not recorded; doctor WARN | `public_ipv4`/`public_ipv6` recorded, doctor OK | `rc2-doctor-baseline-origin-main.txt`, `rc2-doctor-candidate-f070601-fresh.txt` |
| I-15 tunnel user reaches the node's sshd/nginx | **reachable** (`SSH-2.0-Ope...`, nginx 404) | **refused** (rc 52 / rc 35); internet still 204 | `rc2-tunnel-reach-own-address-{baseline,candidate}.txt` |
| I-16 request for older `main` over a newer tree | built the **newer** tree, reported complete, manifest `main` | n/a: fixed installers swap the tree; manifest records commit and tree digest | `rc2-i16-baseline-reproduction.txt`, `rc2-doctor-candidate-f070601-fresh.txt` |
| I-16 pinned SHA delivers that commit | not possible (SHA treated as a branch name) | archive commit read and compared; refusal path tested | `deploy/lib/tests/test-bootstrap-source-identity.sh` (WSL) |
| I-16 crash between the swap's two renames | n/a | recovered; tree digest identical to before | `rc2-source-swap-crash-safety-candidate.txt` |
| I-16 failed install after a swap | n/a | state as designed, services active; clean re-run converges to a byte-identical tree/binaries/config | same |
| I-17 healthy source build | WARN "release identity INCONSISTENT" | OK, 3 executables match, also after reboot | doctor files above, `rc2-doctor-candidate-after-reboot.txt` |
| I-18 wrong-hostname Hysteria2 certificate | doctor exit **0**, FAIL=0 | exit **1**, `Hysteria2 self-test FAILED: ... not accepted for <host>`; REALITY still OK | `rc2-doctor-{baseline,candidate}-wrong-hysteria2-cert.txt` |
| Broken config detected | not run | drifted on-disk config: L4 FAIL; truncated file: L2 FAIL; both exit 1; restored | `rc2-broken-config-detection-candidate.txt` |
| REALITY, real client, public internet | n/a | PASS: first byte 0.7 s, egress 91.244.71.165, Wikipedia 200, TTFB 0.24 s | `rc2-client-proto-candidate-upgrade.json` |
| Subscription and revocation | n/a | PASS: rotate-vless kills the old profile and not the new; disable, expire and remove each refuse the client and 404 the subscription; enable/clear-expiry restore it | `rc2-revocation-lifecycle-candidate.txt` |
| Reboot | n/a | PASS: services, ufw (8 rules) and egress table (9 lines) identical; doctor 41 OK / 0 FAIL / 0 WARN; the same pre-reboot profile still connects; own-address isolation holds | `rc2-reboot-candidate.txt` |
| Hysteria2 with a real client over a network | n/a | **BLOCKED**: 0 UDP/443 packets from the test PC reached the server (tcpdump); the same filtering as I-21. Proven only on loopback (doctor) | `rc2-udp443-capture-during-hy2-attempt.txt` |
| Production updater, injected failure, rollback against a published release | n/a | **NOT RUN**: `update.sh --version` refuses a dev-channel install by design; needs a published rc.2 and an rc.1 install | see `07` |
| Lifecycle gate (`lifecycle-acceptance.sh`) | n/a | **NOT RUN**: AlmaLinux 9 only; this host is Ubuntu | |
| Shell test suite | n/a | 71/71 pass under WSL (3 need root); Windows checkout fails on CRLF (I-27) | |
| Rust: `release_identity` tests, clippy on `admin` | n/a | 19 tests pass; clippy `-D warnings` clean | |
| Device (iPhone) evidence | NOT_TESTED | NOT_TESTED | |

Honest limits: the candidate was installed through the dev channel from a source tree, so a release
build's `vpn` and `vpn-admin` (one copied file) is covered by unit tests and by the fallback, not by a
VPS run. The second host (AlmaLinux) and the owner's production exit were not touched. Two fresh
installs were done (baseline, candidate) on a host with no Let's Encrypt issuance for this name in the
previous 7 days (crt.sh); both completed.

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
