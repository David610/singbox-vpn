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

## Round 2 (dev channel, before release): server hardening candidate

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
| Hysteria2 with a real client over a network | n/a | **BLOCKED on UDP/443**: no datagram from the test PC reached the server's UDP/443 during a full client run (unfiltered capture; the first capture in this round filtered on the wrong source address and was replaced); UDP/4443 from the same PC arrives. Proven on loopback (doctor); on a real network see Round 3 | `rc2-udp-path-and-hysteria2-external.txt` |
| Production updater, injected failure, rollback against a published release | n/a | **superseded by Round 3**: `update.sh --version` refuses a dev-channel install by design, so it was run in Round 3 against the published rc.2 | Round 3 |
| Lifecycle gate (`lifecycle-acceptance.sh`) | n/a | **NOT RUN**: AlmaLinux 9 only; this host is Ubuntu | |
| Shell test suite | n/a | 71/71 pass under WSL (3 need root); Windows checkout fails on CRLF (I-27) | |
| Rust: `release_identity` tests, clippy on `admin` | n/a | 19 tests pass; clippy `-D warnings` clean | |
| Device (iPhone) evidence | NOT_TESTED | NOT_TESTED | |

Honest limits: the candidate was installed through the dev channel from a source tree, so a release
build's `vpn` and `vpn-admin` (one copied file) is covered by unit tests and by the fallback, not by a
VPS run. The second host (AlmaLinux) and the owner's production exit were not touched. Two fresh
installs were done (baseline, candidate) on a host with no Let's Encrypt issuance for this name in the
previous 7 days (crt.sh); both completed.

## Round 3: qualification of the published v1.3.1-rc.2 (prerelease)

**Decision: every server-side check that can run without a physical device passed on the published
artifacts, with one open gap (I-31) and two things that could not be verified (UDP/443 on a real network,
any device). This is a release candidate, not production-ready, and no stable release is implied.** The
candidate is ready for physical iPhone testing, provided the host is a fresh install or has had
`install.sh` re-run (see I-31).

### Identity

| Item | Value |
|---|---|
| Tag | `v1.3.1-rc.2`, annotated, object `bcdb904e5636aa4bd0a8201825234a508e2c8552` |
| Commit | `d64ea56f52e65492cf48f3a9c1324576740edf79` (merge of PR #158; PR head `117a64ae2854a8f3e93152aa60d7fa544a126e72`; parent `main` `0b038a0a09c9f093add9b19bf673b3c29a49c307`) |
| Fix commits inside | I-15 `9ca903e`, I-18 `0c7118a`, I-16 `a4e8f16` + `10ad1b0`, I-17 `f070601`, CI/test follow-ups `a314664`, `117a64a` |
| CI | PR head: 47/47 checks pass, 0 code-scanning alerts. Merge commit: 24/24 check runs, CI and Security workflows success |
| Release | GitHub **prerelease** (not draft; `latest` is still `v1.3.0`), published 2026-10-16T12:03:10Z, release run 38896988771: version/tag/policy gates, the full CI gate, `build`, `runtime-compat` (AlmaLinux 8 and 9 containers), `publish`, `verify-published-bootstrap` all success |
| Version contract | `deploy/lib/check-release-version.sh v1.3.1-rc.2`: PASS (6 packages at 1.3.1). No Cargo change; the prerelease identity is the tag only |

### Artifacts (SHA-256, all five entries of `SHA256SUMS` verified; GitHub's recorded digests equal the local ones)

| Asset | SHA-256 |
|---|---|
| `install.sh` | `9812389200ed52e4e3129d41451c63b6f728bea6761e8dcda71d8cfb5e942120` |
| `SHA256SUMS` | `c7bf83fb38684cfbb22fd54d64bb0667d53895a257fb388b6716cb8da0534e13` |
| `singbox-vpn-src.tar.gz` | `d9c0a066d314e3be8cb7aa0a1eb6d9d66a209d9369e3017e187c78a51f4848ba` |
| `singbox-vpn-x86_64-unknown-linux-gnu.tar.gz` | `0563e6571b4656f473c0e8d9cbf51ecd4b4ac46e28e64bb9a3c279c5307c93d7` |
| `singbox-vpn-sbom.cdx.json` | `280ab5849943ab039ef470de12919bf2adabe8df2ac16efbbc864b7688130d30` |
| `singbox-vpn-licenses.json` | `ab2a63795b66a2306644ae99458dd602f32f4e6f8a42cff93e8acb5940757334` |
| inside the binary archive: `vpn-admin` | `2aa891d520e552c81f997e218c02416d83b83300622fe65acae87a890d980695` |
| inside the binary archive: `subscription` | `19b9678c6ba14ca8367c1892f7751318e648f9a40b66da9ae5dfd85faf01f1af` |

Verified independently of the workflow: attestations for the source archive, the binary archive and the SBOM
(`gh attestation verify`; repository `David610/singbox-vpn`, workflow `release.yml`, ref `refs/tags/v1.3.1-rc.2`,
commit `d64ea56`, trigger `push`); the release source archive equals the tag tree (511 files, 0 git-blob
differences); both binaries report `1.3.1` and need at most `GLIBC_2.28`; the shipped `vpn-admin` contains the
fix markers (`vpn_sha256`, `source_tree_sha256`, Hysteria2 self-test, own-public-address). On the VPS the
installed `vpn-admin`, `vpn` and `vpn-subscription-svc` hash to exactly these values.

### VPS results (91.244.71.165, Ubuntu 24.04.5, disposable; published artifacts only, no dev-channel binaries)

Hostname for these installs: `91-244-71-165.sslip.io`. The project hostname had hit Let's Encrypt's limit of
5 certificates per 168 h (earlier fresh installs, mine included); `crt.sh` had under-reported them (I-36).

| Gate | Result | Evidence (`test-results/`) |
|---|---|---|
| Install v1.3.1-rc.1 with the official bootstrap, pinned | PASS: checksums and attestations verified by bootstrap and installer; doctor 39 OK / 0 FAIL / 0 WARN | (log only) |
| rc.1 state prepared for the upgrade | users `default`, `up-a`, `up-b` (30-day expiry), `up-c` (disabled); real client connected for `up-a`, `up-b` | `rc2-upgrade-rc1-to-rc2-official.txt` |
| Update rc.1 → rc.2 with an injected failure after the switch, **attempt 1** | **FAIL**: "UPDATE FAILED - ROLLBACK ALSO FAILED". All files were restored byte-identically, but `sing-box` stayed in `start-limit-hit` (five restarts in 16 s from my own user changes just before; rc.1 has no fix for I-11) and `vpn-expiry-reconcile` failed. Recovered by `reset-failed`; state identical to before | `rc2-upgrade-attempt1-rollback-failure-rc1-updater.txt` |
| Same, **attempt 2** on a clear restart budget | PASS: "PREVIOUS RELEASE RESTORED AND VERIFIED (v1.3.1-rc.1)", fingerprint identical with no manual recovery, doctor 39/0/0, old profiles connect | `rc2-upgrade-attempt2-injected-failure-rollback-ok.txt` |
| Update rc.1 → rc.2, official command, normal | PASS: checksum + attestation verified, `UPDATE COMPLETE`. Installed binaries equal the published artifacts; manifest `v1.3.1-rc.2` | `rc2-upgrade-rc1-to-rc2-official.txt` |
| Preservation across the update | PASS: REALITY keys and Hysteria2 certificate/key byte-identical; the four accounts keep enabled/disabled/expiry; subscription URLs unchanged; refetched profiles carry identical credentials; real client connects with profiles made on rc.1 | same |
| `vpn doctor` after the update | PASS with 1 WARN: `C-16 no own-public-address deny rule` (**I-31**); identity line OK, both handshake self-tests OK | `rc2-doctor-rc2-published-after-upgrade-and-repair.txt` |
| Tunnel user vs the node's own services after the update | **EXPOSED**: sshd banner readable through the tunnel (update does not backfill the addresses) | `rc2-upgrade-i15-backfill-via-published-install-repair.txt` |
| Remedy: re-run the published rc.2 `install.sh` (in-place repair) | PASS: `public_ipv4`/`public_ipv6` recorded; doctor 41 OK / 0 FAIL / 0 WARN; probe refused (rc 35 / 52) | same |
| Reboot | PASS: all 6 services active; state fingerprint, manifest and key hashes identical; doctor 41/0/0; profiles from rc.1 connect (one single-probe miss, see I-35) | `rc2-reboot-published-rc2.txt` |
| REALITY, full matrix, real client over the internet | PASS: first success 1.4 s, egress `91.244.71.165`, dual-stack probe, HTTPS 200 (TTFB 0.24 s), DNS-through-proxy 200, 3 × 25 MB at 36.7 / 36.3 / 35.4 Mbit/s (this PC's direct rate: 5.4–38.8) | `rc2-client-reality-published-rc2.json` |
| Hysteria2 loopback (doctor self-test) | PASS (handshake with Salamander + auth + certificate); **loopback only** | doctor files |
| **Hysteria2 external, UDP/4443** (not 443) | PASS: real client over the internet, first success 1.39 s, egress IPv4 and IPv6, HTTPS 200 (TTFB 0.10 s), 3 × 25 MB at 13.7 / 14.9 / 24.1 Mbit/s; host reverted afterwards (fingerprint identical) | `rc2-client-hysteria2-udp4443-published-rc2.json`, `rc2-udp-path-and-hysteria2-external.txt` |
| **Hysteria2 external, UDP/443** | **NOT VERIFIED**: UDP/443 from the only available network is filtered upstream of the VPS (0 datagrams reached the server during a full client run; UDP/4443 from the same PC arrives). No other authorized network was available | same |
| Provisioning, rotation, disable, expiry, token rotation, removal | PASS with a real client (9 steps: create, rotate-vless, rotate-hysteria, disable, enable, expire, clear-expiry, rotate-token, remove); disable, expiry and token rotation also 404 the old subscription URL (the 404 after `remove` was observed on the dev-channel build, not re-fetched here) | `rc2-published-provisioning-revocation-burst.txt` |
| Restart burst (I-11) | PASS: 12 back-to-back user creations = 12 `sing-box` starts, 0 start-limit events, doctor 0, existing user still connected | same |
| rc.2's own updater, injected failure straight after a burst (34 starts in 3 min) | PASS: restored and verified, 0 start-limit events, fingerprint identical, no manual recovery | `rc2-published-updater-injected-failure-after-restart-burst.txt` |
| Own-address isolation | PASS on a fresh install and after the install repair; EXPOSED after a plain update (I-31) | see above |

### Not done / outstanding

- **Physical devices (required before any production claim):** the iPhone procedure below, on this exact tag, covering
  Hiddify (REALITY, Hysteria2, subscription refresh, reconnect on Wi-Fi/cellular handover), Shadowrocket and INCY. No
  device evidence exists for rc.2.
- **UDP/443 on a real network:** needs a network that does not filter it (phone hotspot or another host).
- **Open finding I-31:** `update.sh` does not record the node's own addresses. Until it does, hosts updated from rc.1
  need `install.sh` re-run (tested above) or the two keys set by hand. Recommend fixing before a stable release.
- The AlmaLinux lifecycle gate (`lifecycle-acceptance.sh`, `vps-acceptance.yml`) and a certificate-renewal run were not
  executed for rc.2; this host is Ubuntu and the workflow needs a repository SSH-key secret.
- I-33 (cosign downloaded twice with a 300 s cap): a slow link breaks the install; pre-fetching cosign was the workaround.
- Credentials: the VPS root password that was pasted into the conversation has been rotated, SSH-key login is set up, and
  credential-bearing temporary files were removed after the sanitized evidence above was kept.

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
