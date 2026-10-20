# Issue registry (generated from issue-registry.json — do not edit by hand)

States: CONFIRMED_OPEN, IN_PROGRESS, FIXED_IN_BRANCH, CI_VERIFIED, SERVER_VERIFIED, DEVICE_VERIFIED, PRODUCTION_VERIFIED, BLOCKED, UNVERIFIED.

## New / re-assessed this session

| ID | Sev | Component | State | Title | Regression test | Device | Remaining risk |
|---|---|---|---|---|---|---|---|
| I-37 | medium | admin/doctor | FIXED_IN_BRANCH | doctor reported sing-box outbound UDP flow sockets as FAIL; first heuristic could hide a mistaken inbound on an ephemeral port | apps/admin public_surface_tests::* (red before/green after) | no | relies on the on-disk config matching the running process (doctor already checks drift) |
| I-38 | high | staging host egress | BLOCKED | 62.238.46.190: both address families refused for Shorts (0/3,0/6,0/3 true Shorts; literal v4/v6 0/4) | youtube_evidence_probe.py (live acceptance, not CI) | yes | no same-run known-good control on this host |
| I-39 | high | staging host ssh | IN_PROGRESS | PermitRootLogin yes + PasswordAuthentication yes, empty authorized_keys, 447+ failed attempts | deploy/lib/tests/test-ssh-harden.sh (mutation-checked) | no | passwords still enabled until a key is supplied and proven |
| I-40 | medium | staging host firewall | SERVER_VERIFIED | no host firewall before install; provider firewall not visible | acceptance-test.sh ufw checks | no | provider/cloud firewall UNVERIFIED; IPv6 inbound untested |
| I-41 | high | installer | SERVER_VERIFIED | install aborts (useradd exit 9) when sing-box/vpn-subscription primary group already exists (leaked by root-run tests or interrupted install) | deploy/lib/tests/test-service-users-preexisting-group.sh (red before) | no | uninstall removes an adopted orphan group (intended) |
| I-42 | low | tests | FIXED_IN_BRANCH | SSH-port tests depended on the host's real sshd/ss | same test | no | - |
| I-43 | medium | deploy/almalinux/acceptance-test.sh | SERVER_VERIFIED | printed 'PASSED (all required checks passed)' under FAIL lines; no ufw verification | deploy/lib/tests/test-acceptance-summary-honesty.sh (red against original) | no | Alma-only SELinux/firewalld checks still non-applicable on Ubuntu (now stated) |
| I-44 | medium | nginx subscription vhost | SERVER_VERIFIED | HSTS silently dropped from /sub/ and /v1/provision/ (nginx add_header inheritance) | deploy/lib/tests/test-nginx-location-headers.sh (red before) | no | reaches existing nodes only via installer re-run (I-45) |
| I-45 | medium | update.sh | CONFIRMED_OPEN | update.sh never re-renders the nginx vhost, so template-only fixes do not reach deployed nodes | - | no | workaround: re-run install.sh (verified idempotent on staging) |
| I-46 | high | release gate | FIXED_IN_BRANCH | stable release gate accepted a text 'Device acceptance: PASS' although the device matrix was waived | deploy/lib/tests/test_release_evidence.py (13 tests) | yes | gate untested in a real release run; v1.0.0-v1.3.0 exempt by name |
| I-47 | medium | youtube probe | FIXED_IN_BRANCH | probe counted non-Shorts pages as Shorts (baseline overstated 1/4,1/4,2/8; true 0/3,0/3,0/6) | deploy/lib/tests/test_shorts_health.py | no | - |
| I-48 | low | tests | FIXED_IN_BRANCH | five shell tests read the live host (/etc/vpn, /opt, /var/lib, /usr/local/bin) and fail on any installed host | 76/76 pass hermetically on staging | no | - |
| I-49 | low | installer release identity | CONFIRMED_OPEN | local-tree installs are recorded as channel 'main' with a source-tree digest and no git commit | - | no | weak provenance for staging/dev installs |
| I-50 | low | tests | CONFIRMED_OPEN | root-run Rust tests create system groups vpn-subscription/sing-box and never remove them | - | no | - |
| I-52 | high | update.sh --dev-rebuild / test isolation | FIXED_IN_BRANCH | the updater's test-phase guard recorded the doctor's read-only `ufw status` as a refused host-control attempt, so --dev-rebuild could never pass its test phase on a ufw host | deploy/lib/tests/test-isolation-ufw-readonly.sh (red before) | no | prebuilt-release update path was not exercised |
| I-51 | high | Hiddify iOS (client) | UNVERIFIED | profile JSON missing from Library/Caches/Working/configs (carried as I-08); upstream hiddify-app#2372 open, no fix | - | yes | trigger on the owner's iPhone unconfirmed |

## Carried from the 2026-10-15 audit (not re-verified unless listed above)

| ID | Sev | Audit status | State | Title |
|---|---|---|---|---|
| I-01 | high | fixed-on-branch | UNVERIFIED | Hysteria2 links from vpn-web use the REALITY decoy as sni |
| I-02 | high | fixed-on-branch | UNVERIFIED | Hysteria2 links from vpn-web omit the node's Salamander obfuscation |
| I-03 | medium | fixed-on-branch | UNVERIFIED | /sub/[token] had no tests |
| I-04 | medium | open | CONFIRMED_OPEN | `xray` format is not an Xray config ({version,remarks,servers}); INCY's JSON option expects full Xray configs |
| I-05 | high | open | CONFIRMED_OPEN | INCY is declared VLESS-REALITY-only; a third-party report says INCY 2.4.5 imports VLESS-REALITY-only subscriptions as an |
| I-06 | high | fixed-on-branch | UNVERIFIED | No user-facing Shadowrocket (Base64) link although plain body is known to be rejected |
| I-07 | medium | fixed-on-branch | UNVERIFIED | PR #83 UI: throwing helper called in render; clipboard-blocked message replaced by a misleading one |
| I-08 | high | open | CONFIRMED_OPEN | iOS: profile JSON in purgeable Library/Caches/Working/configs while profile DB is persistent; connect() does not check t |
| I-09 | high | open | CONFIRMED_OPEN | Issue B: 'failed to start core / CoreAlert.startService' on Hiddify 4.0.0 dev |
| I-10 | high | open | CONFIRMED_OPEN | 7 of 10 user operations restart sing-box and drop every client's connections |
| I-11 | critical | fixed-on-branch | UNVERIFIED | Burst of 8 restarts in 300 s parks sing-box in start-limit-hit: full outage until the watchdog (3 min 35 s measured), op |
| I-12 | high | fixed-on-main | UNVERIFIED | Hysteria2 caps render ignore_client_bandwidth=true and lock out every standard client (v1.3.0) |
| I-13 | high | open | CONFIRMED_OPEN | Stable v1.3.0 lacks fixes already on main: rustls bump (RUSTSEC-2026-0346..0349), vpn-benchmark restarting production by |
| I-14 | medium | fixed-on-branch | UNVERIFIED | doctor FAILs a correct config once public_ipv4/public_ipv6 are set (contradicts its own warning) |
| I-15 | high | fixed-on-main | UNVERIFIED | Installer leaves the C-16 own-address reject off when public_host is a DNS name: tunnel users reach the node's own servi |
| I-16 | medium | fixed-on-main | UNVERIFIED | Dev-channel installer over an existing install reuses the old source tree and records version 'main' |
| I-17 | low | fixed-on-main | UNVERIFIED | doctor release-identity WARN on source builds because vpn and vpn-admin are separate compiles with different hashes |
| I-18 | medium | fixed-on-main | UNVERIFIED | doctor --protocol and installer self-test cover REALITY only; Hysteria2 breakage passes health checks |
| I-19 | medium | open | CONFIRMED_OPEN | Existing deployments keep exit ipv6_literal=allow after update; dual-stack destinations are reached over IPv6 |
| I-20 | medium | blocked | BLOCKED | Exit IPv4 91.244.71.165 is refused by YouTube for Shorts (0/6 default and IPv4-only; control 6/6) |
| I-21 | medium | blocked | BLOCKED | UDP/443 from the test PC is filtered; Hysteria2/QUIC real-client tests and the real reconnect-stall measurement are impo |
| I-22 | medium | blocked | BLOCKED | Hysteria2 client stalls up to ~30 s after a server restart (1.14.1 sends no QUIC CONNECTION_CLOSE; 1.14.3 does) |
| I-23 | low | fixed-on-branch | UNVERIFIED | PR #156 left user-visible and doc references to archived Tamara/singbox-client; over-stated 'does not use embedded confi |
| I-24 | low | fixed-on-branch | UNVERIFIED | README/INSTALLATION listed only Hiddify; no per-app guidance or evidence status |
| I-25 | informational | informational | UNVERIFIED | `vpn user list` hides the reserved probe user that `vpn status` and doctor count (2 vs 1) |
| I-26 | low | informational | UNVERIFIED | Install prints the first user's token and QR (and so does any install log) |
| I-27 | low | environmental | UNVERIFIED | Windows checkouts get CRLF shell scripts (core.autocrlf=true) that fail under bash; one test uses a Windows openssl with |
| I-28 | medium | blocked | BLOCKED | Not qualified: no physical-device evidence examined; PR #47 (iOS PacketTunnel) open |
| I-29 | low | informational | UNVERIFIED | 10 pre-existing eslint warnings |
| I-30 | medium | open | CONFIRMED_OPEN | Interrupted-deployment recovery, disk/memory pressure, DNS-leak, IPv6-leak, kill-switch, token-entropy, secret-logging a |
| I-31 | high | open | CONFIRMED_OPEN | update.sh does not backfill public_ipv4/public_ipv6: a host updated from rc.1 to rc.2 keeps the tunnel-to-own-services e |
| I-32 | medium | fixed-on-main | UNVERIFIED | v1.3.1-rc.1's updater rollback fails ('UPDATE FAILED - ROLLBACK ALSO FAILED') when sing-box restarts are close together: |
| I-33 | low | open | CONFIRMED_OPEN | The installer downloads its pinned cosign (141 MB) twice, once in the bootstrap and once in install.sh, each with a 300  |
| I-34 | low | open | CONFIRMED_OPEN | After changing [hysteria2] listen_port and running vpn render-config, vpn-subscription keeps advertising the old port un |
| I-35 | informational | informational | UNVERIFIED | Intermittent single-probe misses (about 1 in 10 to 1 in 24) with the real REALITY client after the reboot, for both user |
| I-36 | informational | informational | UNVERIFIED | Let's Encrypt allows 5 certificates per exact hostname per 168 h; repeated fresh installs on one name exhaust it |
