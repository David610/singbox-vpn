# Clean-install readiness for the Russian-entry / European-exit design (2026-10-25/26)

Secret-free. Nothing here was applied to ru1, Evolus or fin1 (they are LIVE reference nodes; owner decision 2026-10-25:
keep them intact, keep their backups, do not reinstall). Owner decision of the same date on egress: for the private pilot,
Google/YouTube may exit through Russia while ordinary traffic exits through Europe (the configuration shown to work).

Evidence classes: SOURCE (code/unit tests), REAL-SINGBOX (CI with the pinned sing-box 1.14.1), INSTALLER/LIFECYCLE, FRESH_VPS, DEVICE.
No FRESH_VPS or DEVICE result exists for anything below; a simulated fresh install is not FRESH_VPS_VERIFIED.

## A. Baseline and engineering changes

Baseline: `main` = `e7defcd` (PR #208 merged). Latest published candidate is still `v1.3.1-rc.6` (`35d4a3f`); **it contains none of the changes
below**. A new candidate (rc.7) must be cut from the merged result after review.

| PR | Change | Evidence |
|---|---|---|
| #209 | Real two-hop scenarios S18-S20: production renderers, dedicated Vision-less exit credential carries traffic; client-Vision vs server-none and client-none vs server-Vision are rejected with no byte reaching the target; lab script no longer claims to reproduce the historical failure | REAL-SINGBOX (CI `singbox-validate` ran them; 47/47 checks green) |
| #210 | `vpn-admin tls-spoof set\|clear\|show`; managed capability drop-in for the TLS spoof (create only while enabled, marker-owned, adopt/leave/remove policy, daemon-reload + restart only when needed, doctor FAIL on exits **and relays** when the running sing-box lacks the capabilities, `uninstall.sh` removal) | SOURCE + CLI lifecycle tests (47/47 green) |
| #211 | `vpn-admin pair issue-exit / accept-exit / issue-hairpin / accept-hairpin / status`: protected bundles instead of hand-edited TOML and UUIDs; declare-once route with conflict refusal; rollback on apply failure; secret hygiene; lifecycle test (render idempotence, rotation, disable/enable, backup/restore) | SOURCE + CLI tests driving the real binary; CI pending/green per the PR |
| #212 | Subscription service refuses `format=hiddify\|uri` for a user holding a relayed route (400 `relay_route_needs_pinned_profile`) instead of serving the direct route; `user create` on a relay hands out the pinned link | SOURCE (workspace 1205/0 locally) |
| #213 | `vpn-admin doctor` WARNs on `RUST_LOG=debug\|trace` for vpn-subscription | SOURCE |
| #205 | Russian onboarding made scheme-neutral; relay link = pinned profile; mobile behaviour labelled unverified | docs |

Not a defect fix, still open: the nested-Vision hang is **field-only and unreproducible in isolation** (`docs/RELAY_EXIT_VISION.md`).
`vless_flow = "none"` (#208) is an opt-in control whose live effect after the change is untested.

## B. Reproducible installation procedure

`docs/PAIRING.md` is the minimal ordered procedure and input list. Summary of what is automated and what is not:

| Step | Automated | Owner |
|---|---|---|
| VPSs, DNS A records, provider firewall, SSH key + recovery | no | yes |
| Install exit / install relay from the published candidate (checksum + attestation verified by the installer) | yes (`install.sh`, `--role relay`) | choose inputs |
| Friend accounts on the entry | yes (`user create`) | names |
| Pair route (dedicated exit credential -> relay user) | yes (`pair issue-exit \| pair accept-exit`) | |
| Google/YouTube exception (hairpin + spoof + capability drop-in) | yes (`pair issue-hairpin \| pair accept-hairpin`, `tls-spoof set`) | decide whether to enable |
| Hand out the link | link printed by `user create` | send privately |
| Device acceptance | no | a phone in Russia |

## C. Verification (honest)

- Unit/config: full workspace test runs were green locally on each branch before push (1203-1205 tests); workspace clippy `-D warnings` and `cargo fmt --check` clean; `cargo doc` clean (#211 had a doc error caught by CI and fixed).
- REAL-SINGBOX: S18-S20 passed in CI. The existing 17 two-hop scenarios and interop suites are unchanged and green.
- Installer/lifecycle: CLI tests exercise create/accept/render/rotate/disable/backup/restore against fake sing-box and systemctl; the real `install.sh`/`update.sh` were **not** run by any new automated test.
- Failures seen and kept on record: CI flakes (rc.4 release gate `singbox-validate` hairpin/harness-readiness race; main `groupadd` "Text file busy"; PR #207 hairpin-lab SNI check) all passed on re-run of the failed jobs; CodeQL cleartext-logging on a test assertion (#212, fixed); rustdoc unclosed tag (#211, fixed).
- Not run: the shipped two-hop suite locally in an unprivileged namespace (the harness could not pass even unchanged scenarios there), so S18-S20 first executed in CI.

## D. Configuration-drift closure

| Baseline item | Status |
|---|---|
| Spoof capability drop-in (manual, ru1) | **Fixed in source** (#210). Not applied live. |
| `[google_egress_tls_spoof]` setting | Intentionally configurable (`tls-spoof set`). |
| Peer endpoints sharing one credential | **Fixed in source**: a relayed route owns a dedicated credential; flow agreement validated (#208/#211). |
| Hairpin users + UUID transfer between hosts | **Fixed in source** (#211 bundles). |
| Share link bypassing the entry | **Fixed in source** (#212). |
| `sslip.io` host names | Intentional environment setting; **unresolved** for a clean install (real DNS names are an owner input; baseline D5). |
| Evolus `RUST_LOG=debug` drop-in | Detection added (#213). Live removal is an owner/approved action; **not done**. |
| Evolus `10-hardening.conf` | **Unresolved / UNKNOWN origin**; candidate to fold into the shipped unit after review. |
| `ipv6_literal = reject_google`, `public_ipv6` | Intentionally configurable; installer default already matches. |
| fin1 still rc.3 | Not part of the two-VPS target; unchanged. |
| sshd password + root login, `/root` leftovers and credential-bearing backups | **Owner-controlled; unchanged.** Backups kept at the owner's instruction. |
| Provider resolvers, `ip_forward=0` | Environment facts; no change needed. |
| Mid-lifecycle: drop-in changes during an `update.sh` rollback | **Known gap**: update's rollback restores `deployment.toml` but not the drop-in; the next `render-config` reconciles it. |

## E. Clean-VPS readiness

The code and tooling are **ready to be tested on two freshly provisioned VPSs once #209-#213 are reviewed and merged and a new candidate is cut** (not before). They are
**not** claimed FRIENDS_READY, and nothing has been installed on a fresh host. Known limits that stay in force:

- Ubuntu 22.04/24.04 are "recognized / best-effort": CI covers package resolution in containers only (`os-matrix`), and `lifecycle-acceptance.sh` refuses non-AlmaLinux hosts.
  Real installs of rc.3-rc.6 succeeded on Ubuntu 22.04/24.04 hosts (manual evidence, including a relay install that exposed and fixed the Hysteria2 self-test defect, #207).
  Full Ubuntu support is therefore **not claimed**; AlmaLinux 9 is the only OS with an end-to-end lifecycle harness.
- Hiddify on a phone is unverified; the nested-Vision hang is unexplained; UDP/QUIC through the relay is untested; reachability of the Russian entry from consumer ISPs is untested.

## F. Handoff: fresh-deployment qualification (MISSION 3/4)

Prerequisites (owner): two fresh VPSs (**owner decision needed: the three existing hosts are kept as references and no purchase is currently authorised**);
entry in Russia, exit in Europe; AlmaLinux 9 preferred (supported gate), Ubuntu 24.04 acceptable as best-effort and labelled so; entry >= 1 vCPU/1 GB, exit >= 2 vCPU/2 GB, >= 10 GB disk;
two DNS A records on a real domain (the Cloudflare token the owner pasted proved active but its permissions are unknown; any DNS change needs explicit approval with the exact records);
provider firewall 22, 80 (certbot HTTP-01, temporarily), 443/tcp, 443/udp, 8443/tcp; SSH key login verified plus provider console recovery; a phone on Russian mobile data and Wi-Fi for the device phase.

Candidate: the merged `main` tagged `v1.3.1-rc.7` (do not reuse rc.6); record the commit, source and binary archive sha256, attestation verification on the workstation, and the installer's on-host verification; pin the source hash in the update script.

Run, in order, recording PASS/FAIL/BLOCKED per line, keeping failures:

1. Install exit then relay with the published installer; `doctor` 0 FAIL on both; unpaired relay answers 503 on every format.
2. `pair issue-exit \| pair accept-exit` for a test user; `pair status` on both; pinned link returns 200 with one server; `?format=hiddify` returns 400 with the pinned hint.
3. Path proof: capture on the exit shows only the relay as the client address; ordinary traffic exits from the exit (IPv4 and IPv6); with the hairpin enabled, Google leaves from the relay and 0 Google-bound packets leave the exit.
4. `tls-spoof set` then reboot: the capability drop-in survives, doctor OK, YouTube handshakes pass; `tls-spoof clear` removes the drop-in and the capabilities.
5. Vision: Chromium HTTP/2 to `m.youtube.com` with Vision on; if it hangs, repeat with `--flow none`; record which passes.
6. Lifecycle: re-run the installer, `render-config` x2 (byte-identical), `update.sh --repair`, reconcile timer, user add/disable/expire, credential rotation (relay and exit sides), subscription refresh, upgrade to the next candidate, downgrade refusal, interrupted update recovery (SIGKILL matrix; verify `interrupt_matrix.sh all` now exits non-zero unless every stage passes), backup then restore on the same node (restore must refuse a role/node mismatch).
7. Fail-closed: stop the exit -> relayed profile fails visibly and nothing reaches the target directly; stop the entry -> same; remove the hairpin -> Google fails, no leak from the exit address.
8. Security: no secrets in journal, CI artefacts or reports; `RUST_LOG` default; ports 22/443/8443 only; SSH key-only after the owner hardening procedure.
9. Device (separate gate): Hiddify import of the single link on Android and iPhone, connect, browse, Shorts signed in/out, long video, Telegram, reconnect, airplane-mode cycle, refresh, app restart; Russian mobile and Wi-Fi.

Blockers requiring the owner: merge/approve #205 and #209-#213; two fresh VPSs and a real domain with DNS access; SSH key + recovery procedure; decision on the Evolus debug drop-in and hardening drop-in (live changes); a device and tester in Russia.
