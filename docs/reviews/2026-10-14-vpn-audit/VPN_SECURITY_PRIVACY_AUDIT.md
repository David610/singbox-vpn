# VPN security and privacy audit — 2026-10-14

Scope: `David610/singbox-vpn` `main` at `af66c63f66b4` (v1.3.1 workspace,
latest stable tag v1.3.0), pinned sing-box **1.14.1**. Companion documents:
[baseline](VPN_PERFORMANCE_BASELINE.md), [roadmap](VPN_OPTIMIZATION_ROADMAP.md),
[results](VPN_OPTIMIZATION_RESULTS.md), [assessment](VPN_FINAL_ENGINEERING_ASSESSMENT.md).

Evidence classes (from `CLAUDE.md`): **code verified**, **lab verified**
(real pinned sing-box in local network namespaces, configs rendered by the
production `vpn-admin`), **CI/test verified**, **server verified**,
**native device verified**, **user reported**, **unverified hypothesis**.
Nothing here is server- or device-verified: this audit had no VPS, no phone and
no IPv6 (the lab kernel has no IPv6 stack). No production host, credential or
external account was touched.

## 1. Threat model used

The repository's [`docs/THREAT_MODEL.md`](../../THREAT_MODEL.md) is correct for
what it covers. This audit adds the actors the current code already defends
against but the document does not name (see W14):

| Actor | Can | Relevant controls |
|---|---|---|
| Network observer between client and exit | see client IP, exit IP, timing, sizes; REALITY ClientHello with the decoy SNI | REALITY/TLS 1.3; Hysteria2 TLS + Salamander |
| Censor / DPI | block, throttle, probe the listeners | REALITY decoy, Hysteria2 masquerade + Salamander; not a guarantee |
| **Authenticated user (or anyone with one leaked credential)** | send arbitrary traffic from inside the exit | C-16 route policy + uid-scoped nftables table |
| Exit operator / compromised exit | see client IP, credential identity, every destination | out of scope (`THREAT_MODEL.md` non-goal); logging kept at `fatal` |
| Exit's DNS resolver (provider or system) | sees every destination domain the exit resolves | none (design; see W8) |
| Hairpin target node (when configured) | sees exit IP + every Google/YouTube destination of every exit user | design trade-off (W9) |
| Relay (two-hop) | sees client IP and the exit address, not destinations | inner REALITY to the exit |
| Holder of a subscription URL | fetch that user's config | hashed token, constant-time compare, generic 404 |

Out of scope by design (unchanged): Tor-class anonymity, a malicious VPS
provider, a compromised client device, hiding the server IP.

## 2. Verified controls

| # | Control | Evidence | Class |
|---|---|---|---|
| C1 | **C-16 exit egress isolation** (fixes 2026-09-27 SVPN-F01): exit route rules `resolve` → reject IPv4/IPv6 special ranges → reject TCP/25 → reject own public IP, ahead of `final: direct`, for both inbounds | rendered by `vpn-admin` in the lab; see §2.1 for the probe results | lab verified (IPv4), code verified (IPv6) |
| C2 | DNS-rebinding resistance of C-16: after `resolve`, sing-box dials exactly the checked addresses (`route/conn.go` uses `DestinationAddresses`) | sing-box 1.14.1 source; lab: a domain resolving to 127.0.0.1 is refused | code + lab verified |
| C3 | Host second layer: `inet arcana_egress_isolation` nftables table rejects the same ranges (minus loopback) and TCP/25 for the `sing-box` uid | `deploy/almalinux/nftables-egress-isolation.sh`; unit `Requires=` it | code verified |
| C4 | Probe credential confined to three destinations; loopback self-test exception scoped to the reserved principal only | rendered config (lab) | lab verified (rendering) |
| C5 | Relay is fail-closed: forwards only to declared exits, ends in `reject` | rendered relay config; §2.1 | lab verified |
| C6 | sing-box logs at `fatal`: after ~3 h of lab traffic (thousands of proxied connections, C-16 rejections) the server and relay logs are **0 bytes** | lab | lab verified |
| C7 | Pinned sing-box hash is checked **before** upstream `checksums.txt` (fixes SVPN-F05) in `install.sh`, `update.sh` and CI | code; the 1.14.1 tarball hash matched `versions.env` | code verified |
| C8 | Subscription tokens: SHA-256 stored, constant-time compare, `no-store`, service bound to loopback with `IPAddressAllow=127.0.0.0/8 ::1/128` | `credentials.rs`, `subscription/src/lib.rs`, unit file | code verified |
| C9 | Token rotation does not claim to revoke transport credentials (CLI text says so) | `apps/admin/src/main.rs` `rotate-token` output | code verified |
| C10 | Revocation: disable/remove/expiry re-renders the config without the user; sing-box restarts, so open sessions of that user end too (every other user's as well, and Hysteria2 users stall ~30 s, W16) | §2.2 (fresh clients, both transports) | lab verified |
| C11 | sing-box unit: dedicated user, only `CAP_NET_BIND_SERVICE`, `ProtectSystem=strict`, `SystemCallFilter=@system-service`, memory/task limits, restart budget + watchdog | `sing-box.service` | code verified |
| C12 | Agent no longer wedges on a failed job report (independent report queue) and job `vpn-admin` children are killed on timeout (SVPN-F03/F04) | `report_queue.rs`, `dispatch.rs` | code verified; F04 residual fixed in this audit (W2) |
| C13 | Lease pool default is 0 (SVPN-F02); unchanged config never restarts sing-box | `config.rs`, `render_and_apply_singbox_config` | code verified |
| C14 | Hysteria2 congestion control is not client-selectable: without operator caps every client is told to use BBR; with caps, fixed-rate senders are capped at them | rendered `ignore_client_bandwidth` (this branch); lab fairness run; interop test | lab + CI verified (W18) |

### 2.1 Lab probes (pinned 1.14.1, IPv4, configs from `vpn-admin`)

All requests go through the client's local SOCKS inbound and the tunnel to the
exit. `curl` is run under `env -i` with `--noproxy ''`: the audit sandbox's
`NO_PROXY` lists 127.0.0.0/8, 169.254.0.0/16 and RFC1918, which would otherwise
make `curl` skip the tunnel for literal private addresses (the 2026-09-27
evidence README warns about the same trap; this audit's first literal-address
run was invalid for that reason and was repeated).

Raw results: [`egress-probe.jsonl`](../evidence/2026-10-14/egress-probe.jsonl),
[`own-ip-gap.jsonl`](../evidence/2026-10-14/own-ip-gap.jsonl). `000` means
`curl` received no HTTP response because the exit's route rejected the
connection; the control column is the same URL fetched from the exit
namespace itself, which proves the target was listening.

| Target (requested through the tunnel) | Control from the exit | via REALITY | via Hysteria2 |
|---|---|---|---|
| loopback literal `127.0.0.1:8081` | 200 | 000 | 000 |
| own public IP literal (`public_ipv4` set) | 200 | 000 | 000 |
| domain that resolves to `127.0.0.1` (rebinding) | 200 | 000 | 000 |
| domain that resolves to the own public IP | 200 | 000 | 000 |
| metadata literal `169.254.169.254` | 200 | 000 | 000 |
| domain that resolves to the metadata address | 200 | 000 | 000 |
| RFC 1918 literal `10.20.30.40:8081` | 200 | 000 | 000 |
| TCP/25 | connects | refused | refused |
| ordinary destination (positive control) | 200 | 200 | 200 |
| **exit rendered without `public_ipv4`** (domain install, W3): own public IP literal | 200 | **200** | not run |
| same, domain that resolves to the own public IP | 200 | **200** | not run |
| same, loopback literal (C-16 still active) | 200 | 000 | not run |

Relay (two-hop): a client profile that uses the relay as its exit
(`relay-only` route) got `000`; the declared relay→exit route got `200`.
So the relay forwards only to its declared exit and is not an open proxy.

Revocation (C10), REALITY and Hysteria2: see §2.2.

### 2.2 Revocation (C10), lab

[`revocation-fresh-clients.jsonl`](../evidence/2026-10-14/revocation-fresh-clients.jsonl).
A first run that reused the running clients was not conclusive for
Hysteria2, because a reused Hysteria2 client also fails for ~30 s after any
restart (W16). The re-run starts **new client processes** with the user's
original subscription at every stage:

| Stage | REALITY | Hysteria2 |
|---|---|---|
| user enabled | works (1st attempt) | works (1st attempt) |
| `vpn-admin user disable`, config re-rendered (user absent from both inbounds), sing-box restarted | **0 of 39** attempts over 40 s | **0 of 39** attempts over 40 s |
| `user enable`, restart | works (1st attempt) | works (1st attempt) |

So a disabled credential stops working on both transports as soon as the
restart completes, and cannot open new sessions.

## 3. Weaknesses and open items

Severity is for the documented product (one self-managed VPS, small trusted
user group) unless stated. "Fleet" items only matter when the provisioning
agent is installed, which a standalone install does not do.

| ID | Severity | Item | Exploitability / impact | Evidence | Status |
|---|---|---|---|---|---|
| W1 | Medium (availability) | `vpn-benchmark` (the documented performance tool) created and deleted a real user per transport by default: **four sing-box restarts per run**, every user disconnected; a run killed mid-way left a **live credential with no expiry** | any operator following `PERFORMANCE_OPTIMIZATION_PLAN.md` "What to run on the actual VPS" | code; `test-vpn-benchmark-safety.sh` fails on the old script; lifecycle harness comment already counted "four restarts in about six seconds" | **Fixed** (see results doc) |
| W2 | Low (fleet) | agent `lease-pool sync` and `external-authorizations` still used `timeout(.., output())`: on timeout the `vpn-admin` child kept waiting on the state lock and could apply after the agent reported failure (SVPN-F04 residual) | needs a long lock hold (update, slow disk) while leases are enabled | code | **Fixed** + source-scan test |
| W3 | Low | Own-public-IP C-16 rule exists only when `public_ipv4`/`public_ipv6` are set. A domain-based install records neither (the installer only fills `public_ipv4` for the sslip.io path; `public_ipv6` is never filled). Then a tunnel user can reach services listening on the node's own public address (including ones the host firewall blocks from outside, e.g. a stock nginx :80) | authenticated user; impact limited to node services bound to public addresses | code; lab: refused with `public_ipv4` set, **reachable (200) without it** (§2.1) | Open — `vpn doctor` now warns (this branch); operator remedy: add `public_ipv4 = "<ip>"` (and `public_ipv6`) to `deployment.toml`, then `vpn render-config` |
| W4 | Medium (privacy, two-hop only) | Direct and "via relay" routes use the same exit credential (SVPN-F10): one Direct use lets the exit link all of that user's Privacy+ sessions to their IP | design | lab subscription: both routes carry the same UUID | Open (design) |
| W5 | Low | `uninstall.sh` deleted `/usr/local/bin/sing-box` when no ownership record exists (`ownership_get SINGBOX_BIN_PRE_EXISTED "0"`), although its own log text says it leaves the binary when ownership is unknown (SVPN-F13) | **reproduced in this audit**: the repo's own `test-uninstall-idempotency.sh`, run as root on the lab VM (never installed), deleted the lab's sing-box binary | lab reproduced | **Fixed** (this branch): unknown ownership defaults to "leave in place"; static check in `test-f13-binary-ownership-invariant.sh`; `test-uninstall-idempotency.sh` now backs up and asserts a foreign binary survives (fails on `main`, passes here), skips hosts running sing-box, and reports failures instead of dying under `set -e` |
| W6 | Low (fleet) | provisioning agent runs as root with light sandboxing and starts probe sing-box clients as root (SVPN-F09); `ensure_probe_user` runs `vpn-admin` synchronously with no timeout | compromised control plane or probe peer | unit file, `protocol_probe.rs` | Open |
| W7 | Low | `/etc/vpn/compat/sing-box` is `sing-box:sing-box 02750`, root writes the temp config there without `O_NOFOLLOW`/`O_EXCL` (SVPN-F25). The running unit cannot write there (`ReadOnlyPaths`), so this is defence in depth | needs code running as uid `sing-box` outside the unit sandbox | code | Open |
| W8 | Medium (privacy) | the exit resolves every destination through the host resolver in plaintext (no `dns` section); the provider's resolver can build a per-exit domain log | provider / on-path observer of exit DNS | rendered config | Open (design; documented 2026-09-27) |
| W9 | Medium (privacy, hairpin only) | with `google-egress-hairpin set`, every user's Google/YouTube flows leave from another node (and country) and the exit sniffs SNI of all flows in memory (SVPN-F26) | by design, owner-approved (INV-7/INV-8) | code | Accepted trade-off; keep it visible in user-facing docs |
| W10 | Low | nginx `limit_req` keys on the full client address, so an IPv6 /64 can rotate addresses past it and drain the backend's deployment-wide bucket (429 for other users' subscription refresh). Tunnel path to the backend is closed by C-16 | needs IPv6 on the host | code | Open |
| W11 | Medium (robustness) | pinned 1.14.1 is two patch releases behind 1.14.3, which includes upstream robustness and input-validation fixes in code the exit runs (TCP accept loop, TLS/QUIC sniffing used by the hairpin/IPv6 policy, local DNS transport) | upstream-assessed; not reproduced here | upstream commits `3f0958b`, `52f9bad`, `60f0a03`, `a1f948c`, `ddbf346` | Open — upgrade through the release process (roadmap R8; the same upgrade fixes W16) |
| W12 | Info | the two-hop route is TCP-only (`network: tcp`): UDP/QUIC through Privacy+ fails closed; apps must fall back to TCP | lab: 0/70 UDP probes through two-hop succeeded | lab verified | By design; document for users |
| W13 | Low (docs) | `PERFORMANCE_OPTIMIZATION_PLAN.md` still describes sing-box 1.13.18 and says the 16 MiB UDP ceilings help Hysteria2 — they do not reach the sing-box 1.14.1 Hysteria2 **server** socket when Salamander is on (it keeps `rmem_default`, 208 KiB; lab: `rb212992`) | n/a | lab verified | **Fixed** (plan doc corrected in this branch) |
| W14 | Low (docs) | `THREAT_MODEL.md` does not name the authenticated user as an untrusted egress principal (C-16), the agent/control plane, exit DNS visibility or the hairpin; it still mentions removed experimental crates | n/a | doc review | **Fixed** (this branch) |
| W15 | Low (docs) | `docs/clients/HIDDIFY_IOS.md` step 9 says the subscription "starts you on REALITY"; the repo's own §13 analysis (`YOUTUBE_FINAL_ROOT_CAUSE.md`) shows Hiddify discards the selector and round-robins per connection across REALITY and Hysteria2 for the default `?format=hiddify` link | n/a | code-verified upstream analysis in repo | **Fixed** (client guide and the `vpn doctor` info line, this branch) |
| W16 | Medium (availability) | After every sing-box restart (each user change, expiry, rotation, update), a Hysteria2 client that was idle while the server was down stalls for **~30 s** from its first request; REALITY recovers in ~1 s. The stopping process sends no QUIC `CONNECTION_CLOSE` and the new one cannot send a valid stateless reset (random key per process; disabled with Salamander) | every Hysteria2 user, at every authorization change | lab C (`restart-recovery-*.jsonl`): 34.6 s with or without Salamander; a **1.14.3 exit**: 4.3 s with unchanged clients | Open on the 1.14.1 pin; **fixed by moving the exit to 1.14.3** (roadmap R8); operator message and `ALMALINUX_DEPLOYMENT.md` corrected (this branch) |
| W17 | Low (performance, privacy-neutral) | While one app saturates the link through Hysteria2, other requests on the same QUIC connection wait ~0.5 s (client-side sing-quic BBR) | every Hysteria2 user during bulk downloads | lab C, attributed with a lab client build | Open — upstream; roadmap R4 |
| W18 | Medium (availability) | With no bandwidth caps (the default), sing-quic honoured any receive rate a client declared and sent to it with an **uncapped** fixed-rate (Brutal) controller that ignores loss; one authenticated user (or anyone with one leaked credential) could take nearly the whole link of the node | any user who edits their client's Hysteria2 `down_mbps`; impact: every other user of the node, both transports | lab C ([`greedy-client-20ms-100m.jsonl`](../evidence/2026-10-14/greedy-client-20ms-100m.jsonl)): greedy 88.5–88.8 of 100 Mbit/s, others 0.1–16; after the fix 27–30 vs 28–37 | **Fixed** (this branch): no caps → `ignore_client_bandwidth: true`; test `hysteria_default_has_no_caps_and_forces_bbr_for_every_client`, interop `hysteria2_default_server_still_serves_clients_that_declare_bandwidth` |

## 4. Privacy

| Who | Sees | Does not see |
|---|---|---|
| Client's ISP / local network | client IP, exit IP, port 443 TCP/UDP, sizes and timing, the decoy SNI (REALITY) or the exit host SNI inside Salamander-obfuscated QUIC (Hysteria2) | destinations |
| Exit | client IP, the user's opaque principal id, every destination (in memory) | nothing is logged at `fatal` (lab: 0 bytes) |
| Exit's DNS resolver | every destination domain, from the exit IP | client IP |
| Destinations | the exit IP (or the hairpin node IP for Google/YouTube when configured) | client IP |
| Relay (two-hop) | client IP, exit host | destinations, the exit credential |

Client-side DNS, IPv6, kill switch and split tunnelling stay client-owned by
contract (`docs/PROVISIONING_CONTRACT.md`); the server cannot detect or prevent
a client DNS or IPv6 leak and must not claim to. The first-party document
excludes DNS/TUN/MTU fields structurally (contract tests). With Hiddify, the
imported `route.rules` are discarded, so no server-side client rule can enforce
anything on the device (`YOUTUBE_FIX_INVARIANTS.md` INV-2).

Privacy-safe metrics are possible without new logging: per-node byte counters
and connection counts (Clash API, loopback with a secret, never per
destination), `Udp: RcvbufErrors`, restart counts and REALITY-rejection counts
as numbers. sing-box does not expose reliable per-user byte counters through
the Clash API (`docs/TRAFFIC_ACCOUNTING.md`); this audit did not change that.

The product is a circumvention VPN, not an anonymity system.

## 5. Regression-test coverage

| Control / fix | Test(s) | Runs in CI |
|---|---|---|
| C-16 rule rendering and order | `c16_host_policy_contract.rs`, exit renderer unit tests | yes |
| C-16 runtime behaviour | `two_hop_system.rs` (s03, s12, s13), lab `egress-probe.sh` | system tests: yes (needs `dummy` netdev + IPv6); lab: manual |
| Relay fail-closed, credential scopes, revocation | `two_hop_system.rs` s04–s11, s14–s17 | yes |
| Hysteria2 caps never lock out standard clients (fix) | `hysteria_bandwidth_caps_never_lock_out_bbr_clients`, `hysteria2_bandwidth_caps_keep_standard_clients_working` (real binary) | yes (interop job, count 11) |
| Hysteria2 forces BBR without caps (W18 fix) | `hysteria_default_has_no_caps_and_forces_bbr_for_every_client`, `hysteria2_default_server_still_serves_clients_that_declare_bandwidth` (real binary) | yes |
| `vpn-benchmark` never mutates by default (W1 fix) | `test-vpn-benchmark-safety.sh`, `test-vpn-benchmark-set-e-regression.sh` | yes (shell job) |
| Agent `vpn-admin` children bounded and killed (W2 fix) | `vpn_admin_spawns_are_always_bounded_and_killed` | yes |
| Operator display names (F17 fix) | `user_create_rejects_control_characters_and_overlong_names` | yes |
| Own-address rule warning (W3) | `own_address_egress_rule_tests` | yes |
| Uninstall keeps a foreign sing-box (W5 fix) | `test-f13-binary-ownership-invariant.sh` (static), `test-uninstall-idempotency.sh` (real script, root) | yes |
| YouTube/IPv6 invariants | the tests named in `YOUTUBE_FIX_INVARIANTS.md`, `youtube_invariants_doc.rs` | yes (IPv6 lab part needs IPv6) |
| No credentials in logs | `two_hop_system.rs` log tests, `test-privacy-logging-hardening.sh` | yes |
| Hysteria2 restart stall (W16) | none automated; lab `restart-recovery.sh` | no — add to the 1.14.3 candidate checklist |
| Same-connection latency (W17) | none automated; lab `loaded-latency.sh` | no |

## 6. Tool runs

The full table is in [results §3.3](VPN_OPTIMIZATION_RESULTS.md). The
security-relevant outcomes: every C-16, relay, revocation, credential-scope
and no-credentials-in-logs test that the lab can run passed with 1.14.1 and
1.14.3; the only failures are tests that need IPv6 (absent in the lab kernel)
or Internet access from the test namespace. `cargo audit` found four rustls
advisories published on the audit date; the lockfile bump in this branch
clears them. No secret appears in any committed file, log excerpt or
evidence file (lab keys are generated per run and discarded; lab user ids are
scrubbed).

## 7. Remaining risks

- Nothing here is server- or device-verified. IPv6 C-16 behaviour, IPv6 client
  paths and the exit IPv6 policy were not exercised (no IPv6 in the lab kernel).
- REALITY depends at runtime on the decoy and DNS (SVPN-F12): unchanged.
- Every authorization change restarts sing-box for everyone (SVPN-F11):
  unchanged; revocation stays immediate (§2.2), and Hysteria2 users pay up to
  ~30 s per restart until the exit runs sing-box 1.14.3 (W16).
- Fleet items (agent sandboxing, health semantics, remote update) remain open;
  the owner deferred the fleet, so they do not block the standalone product.
