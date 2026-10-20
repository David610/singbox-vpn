# Consolidated issue registry (2026-10-20)

Source: `docs/audits/2026-10-15/issues.json` (36 issues, history preserved there) plus issues found this session.
States: CONFIRMED_OPEN, IN_PROGRESS, FIXED_IN_BRANCH, CI_VERIFIED, SERVER_VERIFIED, DEVICE_VERIFIED, PRODUCTION_VERIFIED, BLOCKED, UNVERIFIED.
An issue is closed only when its evidence requirement is met; carried states below were NOT re-verified this session unless stated.

## New this session

| ID | Sev | Component | State | Evidence / fix | Regression test | Device needed | Remaining risk |
|---|---|---|---|---|---|---|---|
| I-37 | medium | admin doctor | FIXED_IN_BRANCH (CI-verifiable) | outbound UDP flow sockets misreported FAIL; commit bba27bb | `public_surface_tests::own_process_*`, `ephemeral_range_parser_*` | no | an unintended sing-box inbound on an ephemeral UDP port would be INFO (documented limit) |
| I-38 | high | staging host | BLOCKED | both families of 62.238.46.190 refused for Shorts (baseline 2026-10-20) | `youtube_evidence_probe.py` (live acceptance, not CI) | yes | needs authorized hairpin target |
| I-39 | high | staging host ssh | BLOCKED | PasswordAuthentication yes, PermitRootLogin yes, authorized_keys empty, 375 failed attempts | none yet | no | needs owner public key before hardening |
| I-40 | medium | staging host | CONFIRMED_OPEN | no host firewall (ufw inactive, nft empty); provider firewall unknown | none yet | no | exposure |

## Carried from 2026-10-15 audit

| ID | Sev | Audit status | Registry state | Title |
|---|---|---|---|---|
| I-01 | high | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | Hysteria2 links from vpn-web use the REALITY decoy as sni |
| I-02 | high | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | Hysteria2 links from vpn-web omit the node's Salamander obfuscation |
| I-03 | medium | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | /sub/[token] had no tests |
| I-04 | medium | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | `xray` format is not an Xray config ({version,remarks,servers}); INCY's JSON option expects full Xray configs |
| I-05 | high | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | INCY is declared VLESS-REALITY-only; a third-party report says INCY 2.4.5 imports VLESS-REALITY-only subscript |
| I-06 | high | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | No user-facing Shadowrocket (Base64) link although plain body is known to be rejected |
| I-07 | medium | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | PR #83 UI: throwing helper called in render; clipboard-blocked message replaced by a misleading one |
| I-08 | high | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | iOS: profile JSON in purgeable Library/Caches/Working/configs while profile DB is persistent; connect() does n |
| I-09 | high | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | Issue B: 'failed to start core / CoreAlert.startService' on Hiddify 4.0.0 dev |
| I-10 | high | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | 7 of 10 user operations restart sing-box and drop every client's connections |
| I-11 | critical | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | Burst of 8 restarts in 300 s parks sing-box in start-limit-hit: full outage until the watchdog (3 min 35 s mea |
| I-12 | high | fixed-on-main | CODE: on main, full workspace suite green at de8f7d5; not server/device verified | Hysteria2 caps render ignore_client_bandwidth=true and lock out every standard client (v1.3.0) |
| I-13 | high | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | Stable v1.3.0 lacks fixes already on main: rustls bump (RUSTSEC-2026-0346..0349), vpn-benchmark restarting pro |
| I-14 | medium | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | doctor FAILs a correct config once public_ipv4/public_ipv6 are set (contradicts its own warning) |
| I-15 | high | fixed-on-main | CODE: on main, full workspace suite green at de8f7d5; not server/device verified | Installer leaves the C-16 own-address reject off when public_host is a DNS name: tunnel users reach the node's |
| I-16 | medium | fixed-on-main | CODE: on main, full workspace suite green at de8f7d5; not server/device verified | Dev-channel installer over an existing install reuses the old source tree and records version 'main' |
| I-17 | low | fixed-on-main | CODE: on main, full workspace suite green at de8f7d5; not server/device verified | doctor release-identity WARN on source builds because vpn and vpn-admin are separate compiles with different h |
| I-18 | medium | fixed-on-main | CODE: on main, full workspace suite green at de8f7d5; not server/device verified | doctor --protocol and installer self-test cover REALITY only; Hysteria2 breakage passes health checks |
| I-19 | medium | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | Existing deployments keep exit ipv6_literal=allow after update; dual-stack destinations are reached over IPv6 |
| I-20 | medium | blocked | BLOCKED | Exit IPv4 91.244.71.165 is refused by YouTube for Shorts (0/6 default and IPv4-only; control 6/6) |
| I-21 | medium | blocked | BLOCKED | UDP/443 from the test PC is filtered; Hysteria2/QUIC real-client tests and the real reconnect-stall measuremen |
| I-22 | medium | blocked | BLOCKED | Hysteria2 client stalls up to ~30 s after a server restart (1.14.1 sends no QUIC CONNECTION_CLOSE; 1.14.3 does |
| I-23 | low | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | PR #156 left user-visible and doc references to archived Tamara/singbox-client; over-stated 'does not use embe |
| I-24 | low | fixed-on-branch | UNVERIFIED (audit says fixed on a branch; not located in this repo) | README/INSTALLATION listed only Hiddify; no per-app guidance or evidence status |
| I-25 | informational | informational | INFORMATIONAL | `vpn user list` hides the reserved probe user that `vpn status` and doctor count (2 vs 1) |
| I-26 | low | informational | INFORMATIONAL | Install prints the first user's token and QR (and so does any install log) |
| I-27 | low | environmental | INFORMATIONAL | Windows checkouts get CRLF shell scripts (core.autocrlf=true) that fail under bash; one test uses a Windows op |
| I-28 | medium | blocked | BLOCKED | Not qualified: no physical-device evidence examined; PR #47 (iOS PacketTunnel) open |
| I-29 | low | informational | INFORMATIONAL | 10 pre-existing eslint warnings |
| I-30 | medium | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | Interrupted-deployment recovery, disk/memory pressure, DNS-leak, IPv6-leak, kill-switch, token-entropy, secret |
| I-31 | high | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | update.sh does not backfill public_ipv4/public_ipv6: a host updated from rc.1 to rc.2 keeps the tunnel-to-own- |
| I-32 | medium | fixed-on-main | CODE: on main, full workspace suite green at de8f7d5; not server/device verified | v1.3.1-rc.1's updater rollback fails ('UPDATE FAILED - ROLLBACK ALSO FAILED') when sing-box restarts are close |
| I-33 | low | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | The installer downloads its pinned cosign (141 MB) twice, once in the bootstrap and once in install.sh, each w |
| I-34 | low | open | CONFIRMED_OPEN (carried from audit; not yet re-reproduced) | After changing [hysteria2] listen_port and running vpn render-config, vpn-subscription keeps advertising the o |
| I-35 | informational | informational | INFORMATIONAL | Intermittent single-probe misses (about 1 in 10 to 1 in 24) with the real REALITY client after the reboot, for |
| I-36 | informational | informational | INFORMATIONAL | Let's Encrypt allows 5 certificates per exact hostname per 168 h; repeated fresh installs on one name exhaust  |
