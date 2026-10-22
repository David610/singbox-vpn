# VPN optimization results — 2026-10-14

What this branch changed, what each change was checked against, and what the
lab measured. Base: `main` at `af66c63f66b4`, pinned sing-box 1.14.1.
Evidence classes as in [the baseline](VPN_PERFORMANCE_BASELINE.md): **B**
same-host overhead, **C** controlled simulation. Nothing here is VPS (D),
real-network (E) or iPhone (F) evidence, and no production host, credential
or external account was touched. Raw data:
[`docs/reviews/evidence/2026-10-14/`](../evidence/2026-10-14/).

## 1. Changes in this branch

| # | Change | Files | Verified by | Behaviour change for users |
|---|---|---|---|---|
| 1 | `vpn-benchmark` no longer mutates production by default: the protocol layer runs a disposable loopback sing-box server+client with throwaway keys (mirrors only the live Salamander/caps booleans); the old live-listener test needs `--production-listener`, uses a user that expires after 30 min, removes it on exit or INT/TERM/HUP, and says MUTATING. Tunnel `curl`s use `--noproxy ''` | `deploy/lib/vpn-benchmark.sh`, `vpn-benchmark-lib.sh`, `deploy/almalinux/lifecycle-acceptance.sh` (passes the flag), harness tests | new `test-vpn-benchmark-safety.sh` (4 cases; **fails against `main`'s script**), updated set-e regression test, lifecycle harness assertion, one real-binary run in the lab ([`vpn-benchmark-default-run-lab.json`](../evidence/2026-10-14/vpn-benchmark-default-run-lab.json)) | None on the server. Operators get a non-disruptive default; acceptance keeps the live test explicitly |
| 2 | Hysteria2 opt-in caps (`[hysteria2] up_mbps/down_mbps`) render without `ignore_client_bandwidth` | `crates/compat-config/src/server.rs`, `model.rs`/`deployment.rs` docs | unit test; new real-binary interop test `hysteria2_bandwidth_caps_keep_standard_clients_working`; CI interop count 9 → 10; lab before/after ([`hy2-bandwidth-caps.txt`](../evidence/2026-10-14/hy2-bandwidth-caps.txt)) | Only for hosts that set the caps: users who were locked out (HTTP 404 at auth) can connect again with BBR |
| 3 | Provisioning agent: `lease-pool sync` and `external-authorizations` `vpn-admin` calls go through `run_vpn_admin` (bounded, kills the process group and reaps on timeout) | `apps/provisioning-agent/src/{dispatch,lease_pool,external_authorization}.rs` | new source-scan test `vpn_admin_spawns_are_always_bounded_and_killed` (fails if any `vpn_admin_command(..)…output()` call site appears) | Fleet only; a timed-out job can no longer apply later |
| 4 | `vpn-admin user create` rejects control characters and names over 128 characters (2026-09-27 F17) | `apps/admin/src/main.rs`, `apps/admin/tests/cli.rs` | CLI test (rejects `\n`, `\t`, ESC, 129 chars; accepts `Мария · Berlin` and 128 chars) | Operator input only |
| 5 | `vpn doctor` reports whether the C-16 own-address rule exists on an exit (WARN when no public IP is recorded; never prints the address) | `apps/admin/src/main.rs` | 3 unit tests | Diagnostic only |
| 6 | Corrected operator/user text: the reload message and `ALMALINUX_DEPLOYMENT.md` now say Hysteria2 clients can stall ~30 s after a restart; the `vpn doctor` info line and `HIDDIFY_IOS.md` step 9 no longer claim Hiddify starts on REALITY | `apps/admin/src/main.rs`, `docs/ALMALINUX_DEPLOYMENT.md`, `docs/clients/HIDDIFY_IOS.md` | text review against lab evidence (§2.3) and the repo's hiddify-core analysis | Text only |
| 7 | Docs: `PERFORMANCE_OPTIMIZATION_PLAN.md` (pin 1.14.1; 16 MiB ceilings do not reach the Hysteria2 server socket; benchmark default), `THREAT_MODEL.md` (authenticated user as egress principal, C-16, privacy limits) | docs | review | — |
| 8 | Reproducible performance lab (`deploy/lab/perf/`, disposable machines only) | new | shellcheck `-S warning` clean; `test-no-broad-process-kills.sh` passes | — |
| 9 | Hysteria2 inbound without caps (the default) renders `ignore_client_bandwidth: true`: every client is told to use BBR, so no client can get an uncapped fixed-rate sender | `crates/compat-config/src/server.rs` (+ field docs), `deploy/lab/perf/lab.sh` | unit test (default renders the flag, caps never do); new real-binary interop test `hysteria2_default_server_still_serves_clients_that_declare_bandwidth` (1.14.1 and 1.14.3); CI interop count 10 → 11; lab fairness run (§2.9) | Clients that declare nothing: identical server path. A client that declares a bandwidth now gets BBR instead of uncapped Brutal |
| 10 | `uninstall.sh` keeps `/usr/local/bin/sing-box` when ownership is unknown (no record); the uninstall idempotency test protects and checks a pre-existing binary and reports its own failures | `deploy/almalinux/uninstall.sh`, `deploy/lib/tests/test-uninstall-idempotency.sh`, `test-f13-binary-ownership-invariant.sh` | idempotency test fails against `main`'s script (binary removed, restored from backup) and passes with the fix; static check | Uninstall on a host singbox-vpn never managed no longer deletes someone else's sing-box |
| 11 | `Cargo.lock`: rustls 0.23.45 → 0.23.46 (RUSTSEC-2026-0346/0347/0348/0349, published 2026-10-14; one medium, three low) | `Cargo.lock` | `cargo audit` 4 vulnerabilities → 0; workspace build and tests | Provisioning agent only (its HTTPS client); no behaviour change expected |
| 12 | The REALITY decoy-size regression test uses a 600-SAN certificate (12248-byte Certificate record) instead of 800 SANs (16048 bytes); `vpn-admin doctor --protocol` and its doc comment now name the current limit (REALITY drops a connection when more than 16384 bytes of the decoy's reply are buffered unparsed) instead of the old 8192-byte one | `crates/compat-config/tests/common/mod.rs`, `tests/reality_decoy_budget.rs`, `apps/admin/src/main.rs` | the CI failure's own trace (first read 4096 bytes, then > 16384 unparsed → dropped; the same commit passed in the other run); 30/30 passes on 1.14.1 and on 1.14.3; on 1.13.19 (old pin, checksum-verified) the test still fails 5/5 with the old "too long" rejection while the small-decoy tests pass ([`reality-decoy-buffer-race.txt`](../evidence/2026-10-14/reality-decoy-buffer-race.txt)) | Test and diagnostic text only (W19) |

Not changed, deliberately: no default transport, routing, hairpin, IPv6,
sniffing, firewall, unit or kernel-tuning change; no new egress; no release.
Change 9 is the only data-plane behaviour change and touches only the
Hysteria2 congestion-control answer to clients that declare a bandwidth. The
YouTube invariants file and its enforcing tests are untouched;
`YOUTUBE_FIX_INVARIANTS.md` was re-read in full before change 9.

### 1.1 Before / after per implemented change

| # | Original behaviour | Updated behaviour | Environment and method | Before → after | Security / privacy regression | Remaining limitation |
|---|---|---|---|---|---|---|
| 1 | Default `vpn-benchmark` run: 4 sing-box restarts, every user cut; a killed run left a non-expiring credential | Default run: 0 restarts, no user created; live test only with `--production-listener` (expiring user, signal cleanup) | stub-binary safety test (4 cases) + one real-binary lab run | restarts per default run 4 → 0 (−100 %); credentials left behind by a killed run: possible → impossible by default, 30-min expiry with the flag | none: no new secret handling; the disposable instance uses throwaway keys and reads only booleans from the live config | the `--production-listener` path still restarts sing-box (by design, labelled MUTATING) |
| 2 | Caps set → every standard client refused (HTTP 404) | Caps set → standard clients use BBR, declaring clients Brutal ≤ caps | real binary, loopback | standard-client connection success with caps 0 % → 100 % | none | only hosts that opted into caps were affected |
| 3 | Agent `vpn-admin` children survived timeouts and could apply later | Killed (process group) and reaped on timeout | source-scan test + existing agent tests | n/a (correctness) | none | fleet only |
| 4 | Any display name accepted, including control characters | Control characters and > 128 chars rejected | CLI test | n/a | removes a log/terminal-injection vector in operator output | existing stored names are not re-validated |
| 5 | No signal when the own-address rule is missing | `vpn doctor` WARNs; never prints the address | unit tests | n/a | none (field names only) | the installer still does not record the address |
| 6 | Text said Hysteria2 users are "briefly" disconnected; Hiddify "starts on REALITY" | Text matches lab and code evidence | review | n/a | none | the stall itself is upstream (§2.3) |
| 9 | No caps: a declaring client got uncapped Brutal and could starve the node | No caps: every client gets BBR | lab C, 20 ms, shared 100 Mbit/s, two runs per case | other users' throughput with a greedy client: REALITY 0.1 / 16 → 37.2 / 34.4 Mbit/s, Hysteria2 2.2 / 0.7 → 29.7 / 28.3 Mbit/s; greedy client 88.5 / 88.8 → 27 / 30.4 | none; removes an availability attack by an authenticated user | lab only; a user who hand-declared an accurate rate loses Brutal's edge on lossy links |
| 10 | Uninstall on a never-installed host deleted a foreign `/usr/local/bin/sing-box` | Left in place when ownership is unknown | real `uninstall.sh` twice in a PID namespace, root, disposable VM | binary removed → kept | removes an integrity/availability side effect on shared hosts | installs older than the ownership record also keep the binary on uninstall (residue, by design) |

Percentages are given only where the before and after numbers come from the
same method and environment.

## 2. Measurements

### 2.1 `vpn-benchmark` default run (change 1)

Before: one run created and deleted a user per transport — four sing-box
restarts, every connected user cut four times — and a run killed between
create and delete left a credential with no expiry (`main`'s script fails the
new safety test). After: the default run calls no `user create/remove`, never
restarts sing-box, and reports `production_listener_status: SKIPPED`. In the
lab exit namespace the disposable instance measured REALITY 4387 Mbit/s and
Hysteria2 663 Mbit/s (one unpinned `--quick` sample; a functional check, not a
comparison number) and read the Hysteria2 socket memory (`rb212992`).

### 2.2 Hysteria2 caps lockout (change 2)

Server with caps 100/100: with `ignore_client_bandwidth` (old rendering), a
client that declares no bandwidth fails with `authentication failed, status
code: 404` and the server logs `BBR disabled by server`; without it (new
rendering) the same client works, and a client declaring 50/50 works in both.

### 2.3 Hysteria2 restart stall (W16, roadmap R3) — found in this audit

80 ms RTT; clients keep running while the exit restarts; a new request every
250 ms; 3–4 rounds per row, every round within ±0.1 s
([`restart-recovery-*.jsonl`](../evidence/2026-10-14/)).

| Exit build / condition | REALITY | Hysteria2 |
|---|---:|---:|
| official 1.14.1, client sending during restart, ICMP delivered | 1.3 s | 1.3 s |
| official, client sending, ICMP dropped | 1.3 s | **30.5 s** (9 failed) |
| official, client idle during restart, ICMP delivered | 4.3 s | **34.6 s** (10 failed) |
| official, idle, **no Salamander** | 4.3 s | **34.6 s** |
| same source, lab build, unpatched (control) | 4.3 s | **34.6 s** |
| same source + upstream-fix prototype (close QUIC connections on shutdown) | 4.3 s | **4.3 s** (0 failed) |
| prototype, client sending, ICMP dropped | 1.3 s | **1.3 s** |
| official, exit `idle_timeout: "20s"` on the Hysteria2 inbound | 4.3 s | **24.6 s** (7 failed) |
| **official 1.14.3** on both ends, idle client | 4.3 s | **4.3 s** (0 failed) |
| official 1.14.3 **server**, 1.14.1 clients | 4.3 s | **4.3 s** (0 failed) |
| official 1.14.1 server, 1.14.3 clients | 4.3 s | **34.6 s** (10 failed) |

(4.3 s = 1.3 s restart + the 3 s the idle client waited before its first
request.) Cause, from the sources: sing-quic's `Service.Close()` only closes
the listener, quic-go leaves accepted connections alone and the process exits
without `CONNECTION_CLOSE`; the next process uses a random stateless-reset key
(and disables resets entirely with Salamander, which is reasonable: answering
unknown packets would make an obfuscated server probe-able), so the client
waits out its idle timeout. The client's timeout is fixed at 30 s
(`disable_chrome_parrot` docs), but QUIC uses the smaller of both sides'
values, so a 20 s server value shortens it; the idle client's keepalive
traffic stayed the same (10 packets per 35 s, 30 s vs 20 s), because quic-go
keeps alive at min(10 s, timeout/2).

What this means for a user: every user change on the server costs every
Hysteria2 user up to ~30 s of failing requests; on Hiddify's per-connection
balancer that is about half of all new connections. REALITY users lose about
a second.

**sing-box 1.14.3 already fixes it on the server side.** Its new lifecycle
`Scope.Close()` cancels the context and then runs cleanups in reverse order,
so the Hysteria2 service ends its QUIC connections (sending
`CONNECTION_CLOSE`) before the UDP socket closes — the same thing the lab
prototype ([`upstream-proposal/`](../evidence/2026-10-14/upstream-proposal/))
does by hand. With a 1.14.3 exit, 1.14.1 clients recovered in 4.3 s; a 1.14.1
exit with 1.14.3 clients still stalled 34.6 s. So the fix reaches existing
clients (Hiddify included, by the same mechanism, not tested) as soon as the
exit runs 1.14.3 — the strongest reason found for the upgrade (roadmap R8).
The 20 s `idle_timeout` mitigation is unnecessary once that ships.

### 2.4 Hysteria2 same-connection latency (W17, roadmap R4)

20 ms RTT, 100 Mbit/s link, download and 64-byte ping-pong on the same
Hysteria2 client:

| Step | Result | File |
|---|---|---|
| Separate QUIC connection through the same full queue | 47 ms vs 529 ms same-connection | `loaded-latency-20ms.jsonl` |
| Server BBR profile conservative | queue 4× smaller, ping unchanged (530 ms) | `bbr-profile-20ms.jsonl` |
| Download sender-limited to 50 / 80 Mbit/s | p50 31 ms (p95 437) / 553 ms | `sameconn-ratelimit-20ms.jsonl` |
| Client UDP receive queue, drops | ≤ 4.6 KB, 0 `RcvbufErrors` on both ends | `sameconn-queue-20ms.jsonl` |
| Where the time goes (plaintext captures both ends) | client→exit 509 ms, exit→client 13 ms, echo 3 ms | `sameconn-direction-20ms.jsonl` |
| Wire level | app → client's first matching UDP packet: **498 ms**; wire → exit 10 ms; exit → echo 0.5 ms | `sameconn-wire-20ms.jsonl` |
| Client `bbr_profile` standard / aggressive / conservative | 524 / 524.5 / 524 ms | `sameconn-client-variants.jsonl` |
| Client `disable_chrome_parrot: true` | 702 ms (worse) | same |
| Client declares only `up_mbps` | 525 ms — sing-quic still runs BBR on both sides | `sameconn-up-only-declaration.jsonl` |
| Lab client build whose **sender** uses a fixed-rate controller, exit unchanged | **45 ms**, download 85 → 92 Mbit/s | `sameconn-client-variants.jsonl` |

So the client's sing-quic BBR holds small requests ~0.5 s while the client is
busy receiving. The attribution patch is not a fix (a fixed-rate sender is
unsafe on real uplinks); the fix belongs in sing-quic's client congestion
control. Hiddify's core was not tested and may differ. Until then, REALITY is
the transport to recommend for responsiveness during large downloads.

### 2.5 sing-box 1.14.1 vs 1.14.3 (roadmap R8)

Class B with the exit on one CPU, 3 runs per test
([`results-classB-*.jsonl`](../evidence/2026-10-14/)): REALITY and Hysteria2,
download and upload, with and without Salamander — all inside run-to-run
spread. The one apparent difference (REALITY carrying HTTPS: 5303 vs 4601
Mbit/s, ranges not overlapping) disappeared in an interleaved A/B re-run:
1.14.1 4293–5357, 1.14.3 4479–5480 Mbit/s
([`results-ab-reality-payload-1.14.1-vs-1.14.3.jsonl`](../evidence/2026-10-14/results-ab-reality-payload-1.14.1-vs-1.14.3.jsonl)).
The real-binary interop suites with 1.14.3 are in §3. Throughput does not
change, but a 1.14.3 exit removes the Hysteria2 restart stall (§2.3), and the
same-connection delay is unchanged (524 ms with 1.14.3 on both ends).

### 2.6 Capacity (roadmap R9, R16)

See [baseline §5.1](VPN_PERFORMANCE_BASELINE.md). One 2.8 GHz core: ~0.55
Gbit/s Hysteria2 total (flat from 5 to 50 users, exit CPU 94–97 %), 2.5–3.7
Gbit/s REALITY; no failed download at 50 users; RSS ≤ 96 MB; ≤ 112 fds.
CPU profiles: Hysteria2 time is per-packet syscalls (42 %), Salamander XOR
(10 %), quic-go packet assembly (~15 %), AES-GCM (3 %); REALITY time is
syscalls (69 %) and AES-GCM (21 %).

### 2.7 Hysteria2 connections closed under loss (roadmap R18)

Matrix: 3 of 9 lossy Hysteria2 iperf3 runs ended with "control socket has
closed unexpectedly"; 0 of 9 on REALITY. Targeted repro, 80 ms / 3 % loss,
debug logs on both ends: **0 of 10** on Hysteria2 (70–74 Mbit/s each) and 0 of
10 on REALITY ([`hy2-close-repro.jsonl`](../evidence/2026-10-14/hy2-close-repro.jsonl)).
Not reproduced, not explained; kept open as a hypothesis.

### 2.8 Brutal vs BBR (80 ms, 1 % loss, 100 Mbit/s)

| Client | Download | Same-connection ping p50 / p95 |
|---|---:|---:|
| declares 90 Mbit/s (Brutal both ways) | 87.2 | 86 / 114 ms |
| declares 200 Mbit/s (above link capacity) | 78.6 | 128 / 379 ms |
| declares nothing (BBR, the product default) | 72.6 | 392 / 540 ms |

A correct declaration is better on a fixed link, a wrong one is worse, and a
phone's real capacity changes all the time — so Brutal stays an operator
opt-in: without caps every client now gets BBR (change 9), with caps
undeclared clients keep BBR and declared ones are capped (change 2). These
rows were measured with the exit capped at 1000/1000, which is what lets the
declaring clients use Brutal.

### 2.9 One user starving others (change 9)

20 ms RTT, one shared 100 Mbit/s link, three simultaneous 20 s downloads
([`greedy-client-20ms-100m.jsonl`](../evidence/2026-10-14/greedy-client-20ms-100m.jsonl)):

| Exit | REALITY user | Hysteria2 user (declares nothing) | Hysteria2 user declaring 500 Mbit/s |
|---|---:|---:|---:|
| as rendered on `main`, no greedy user | 50.3 | 42.9 | — |
| as rendered on `main`, greedy user (2 runs) | 0.1 / 16.0 | 2.2 / 0.7 | 88.5 / 88.8 |
| this branch (`ignore_client_bandwidth`, no caps), greedy user (2 runs) | 37.2 / 34.4 | 29.7 / 28.3 | 27.0 / 30.4 |
| this branch, no greedy user | 52.1 | 40.3 | — |

### 2.10 Security probes

C-16 (both transports), own-address gap, relay fail-closed and revocation
with fresh clients: [security audit §2](VPN_SECURITY_PRIVACY_AUDIT.md).

## 3. Stability and regression runs

### 3.1 Lab soak, 90 minutes

Class C, `deploy/lab/perf/soak.sh 5400` on the second VM (Xeon @ 2.10 GHz,
4 MiB socket-buffer ceilings), 20 ms RTT, 0 % loss, no shaping, sing-box
1.14.1 on both ends, no exit restart during the run. Per transport: two
long-lived connections with a 64-byte ping-pong every second, 20 new
connections every 10 s, and a 10 s bulk download every 60 s (both
transports' downloads run at the same time, so their rates are not a
throughput result). Raw data:
[`soak-90min-20ms/`](../evidence/2026-10-14/soak-90min-20ms/).

| Measure | REALITY | Hysteria2 |
|---|---|---|
| Long-lived connections | 2 of 2 survived; 0 failed of 5238 + 5242 pings | 2 of 2 survived; 0 failed of 5134 + 5133 pings |
| Long-lived ping p50 / p95 / p99 / max | 24.4 / 58 / 66–76 / 304–512 ms | 25.0 / 150–153 / 674–687 / 1077–1092 ms |
| New connections | 8340, 0 failed; median per-batch p50 79.9 ms | 9000, 0 failed; median per-batch p50 28.9 ms |
| Bulk downloads | 90, 0 failed; median 49.2 Mbit/s | 90, 0 failed; median 44.0 Mbit/s |

Exit process, sampled every 10 s (539 samples): alive for the whole run;
RSS 58.1 MB at start, median 63.9 MB in the first 10 min, 64.7 MB at
10–30 min, 65.3 MB at 30–60 min, 65.5 MB at 60–90 min, max 66.2 MB; open
fds median 16 (max 22); 6–7 threads; average CPU 4.9 % of one core.

What it shows: in 90 minutes under mixed load nothing failed and nothing
grew without bound — RSS levels off after ~30 min and fds stay flat. That
rules out a fast leak, not a slow one; the multi-day proposal in the
assessment still applies. The Hysteria2 tail is the same-connection delay
of §2.4: the pings share one QUIC connection with the periodic download,
which runs one sixth of the time, so p95/p99 land in download windows.
REALITY's pings use their own TCP connection and are not affected. The
run had no restarts (§2.3) and no loss (§2.7).

### 3.2 Network outage

Class C, `deploy/lab/perf/outage.sh SECONDS 2` on the same VM, 80 ms RTT,
0 % loss: every packet between the client and exit networks is dropped for
5, 20 or 40 s while a proxied connection does a 1 s ping-pong (120 s
per-ping deadline), then new requests are tried every 0.5 s. Two rounds
each; both rounds agreed. Raw data:
[`outage-80ms.jsonl`](../evidence/2026-10-14/outage-80ms.jsonl).

| Outage | REALITY open connection | Hysteria2 open connection | First new request after the outage (REALITY / Hysteria2) |
|---|---|---|---|
| 5 s | survived; longest ping 4.9 s | survived; longest ping 4.9–5.0 s | 0.28 s / 0.10 s |
| 20 s | survived, but its longest ping took 37.4–38.1 s: it resumed ~17–19 s **after** the network was back | survived; longest ping 19.9–20.0 s (resumed at once) | 0.28–0.31 s / 0.10–0.14 s |
| 40 s | survived, longest ping 74.9–75.5 s: resumed ~35 s after the network was back | **closed** (the 30 s QUIC idle timeout expired during the outage) | 0.27–0.37 s / 0.28–0.35 s |

What it shows: after a gap, **new** connections work within 0.4 s on both
transports. An **open** REALITY connection survives but stays stuck for a
while after the network returns, because TCP doubles its retransmission
timeout during the outage and waits for the next scheduled retransmission.
An open Hysteria2 connection resumes immediately after gaps shorter than
its 30 s idle timeout and is closed after longer ones; the next request
then opens a new QUIC connection in ~0.3 s. Lab Linux TCP on both ends: an
iPhone's TCP stack schedules client retransmissions differently, so the
REALITY delay is a lab value, not a device value. Whether a server-side
retransmission cap shortens the stall for downloads is roadmap R19.

### 3.3 Tool runs

Run on the lab VM on 2026-10-14 against this branch's final code unless a
row says otherwise. "Env" marks a failure caused by the environment, with the
exact cause; such a test did not pass and is not counted as passing.

| Check | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | pass |
| Lints | `cargo clippy --all-targets --locked -- -D warnings` | pass (Cargo prints one pre-existing manifest note: `main.rs` is in two bin targets) |
| Unit + integration tests, real binaries required | `ip netns exec itest env SING_BOX_BIN=/usr/local/bin/sing-box SINGBOX_VPN_REQUIRE_REAL_INTEROP=1 cargo test --offline --workspace --locked --no-fail-fast` | **1035 passed, 4 failed.** Env: `real_core_literal_family_and_proxy_recovery_boundaries`, `core_start_failure_stays_visible_with_production_logging`, `s13_relay_reaches_only_the_declared_exit_target` — `EAFNOSUPPORT`, the kernel has no IPv6; `probes_a_real_singbox_direct_outbound_successfully` — needs Internet, which the test namespace has none of (passes in the root namespace: provisioning-agent 126/126) |
| Why a namespace | same command in the root namespace (before the last fixes) | 1013 passed, 25 failed, all Env: 1 no IPv6; 3 REALITY tests use `localhost` as the decoy and this VM's resolver (8.8.8.8, no systemd-resolved) answers NXDOMAIN; 21 `two_hop_system` tests need a `dummy` link type the kernel lacks. The `itest` namespace adds a resolver answering `localhost` and a `sudo` shim that creates a veth pair where the test asks for `dummy`; no test code or assertion was changed |
| CI-equivalent real interop | `cargo test -p compat-config --test reality_interop --test reality_decoy_budget --test hysteria2_interop` (namespace, `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`) | 11 ok = the CI expected count |
| REALITY decoy guard after the fixture change (change 12) | `reality_decoy_budget` alone, 30 runs each (namespace, `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`); then against sing-box 1.13.19 | 1.14.1 30/30, 1.14.3 30/30; 1.13.19 0/5 (expected: the guard still catches the old limit; `reality_interop` with the small decoy 3/3 on 1.13.19) |
| Real-binary suites with sing-box **1.14.3** | reality_interop, reality_decoy_budget, hysteria2_interop, quic_reject_config, two_hop_system | 33 passed, 2 failed (Env: the same two IPv6 binds) |
| YouTube/IPv6 invariants | the tests named in `YOUTUBE_FIX_INVARIANTS.md` + `youtube_invariants_doc.rs` | all pass (the IPv6 runtime lab is the Env failure above) |
| sing-box config validation | interop tests run `sing-box check` on rendered configs; every lab config was checked with 1.14.1 (and 1.14.3 where used) | pass |
| Shell lint | `shellcheck -S warning` on the CI file set, and on `deploy/lab/perf/*.sh`; `bash -n` on every script | pass |
| Shell regression tests (69) | `bash deploy/lib/tests/*.sh` as root | **69 of 69 exit 0.** Skips found and handled: `idn2`/`tshark` were missing and the checkout was shallow (no historical fixture) — installed/fetched, and `test-idn-punycode`, `test-vpn-investigate` and `test-uninstall-ownership-checkpoint2` re-run with no skip, all pass; `test-uninstall-hardening` still skips one sub-check because this VM's `/dev/tty` node is world-accessible without a controlling terminal (the gate fails closed anyway: the prompt write/read fails and it aborts); `test-uninstall-idempotency` skipped while lab sing-box processes were running and was re-run after the lab was stopped: passes with no skip, and the pre-existing `/usr/local/bin/sing-box` survived both uninstall runs with an unchanged checksum (its earlier run on `main`'s script reproduced W5). The first full run (before the W5 fix) failed that test after it deleted the lab's `/usr/local/bin/sing-box` |
| Dependency audit | `cargo audit` (cargo-audit installed for this run) | before: 4 vulnerabilities (rustls 0.23.45, published 2026-10-14) → after the lockfile bump: 0; one existing "yanked" warning (yoke-derive 0.8.3), allowed |
| Benchmark tests | `test-vpn-benchmark-safety.sh`, `test-vpn-benchmark-set-e-regression.sh`; one real run of the new default `vpn-benchmark` in the lab | pass |
| Not available | IPv6, a VPS, an iPhone, a real mobile network, Hiddify's core | not run — see the assessment's test plan |

## 4. Not verified

- No VPS, no iPhone, no real mobile network, no censor: every number above is
  class B or C. The restart stall and the same-connection latency must be
  confirmed on a real VPS with Hiddify before anyone quotes them as user
  experience.
- No IPv6 in the lab kernel: IPv6 egress, the exit IPv6-literal policy and
  `egress_family_interop` were not exercised (the latter fails here with
  `EAFNOSUPPORT`, on `main` as well).
- Hiddify's core (and its sing-quic version) was not tested; R4 may differ
  there.
- No production node was inspected; current node inventory and settings
  (`public_ipv4`, caps, BBR) are unknown.
