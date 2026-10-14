# VPN final engineering assessment — 2026-10-14

Scope: `David610/singbox-vpn` `main` at `af66c63f66b4`, pinned sing-box 1.14.1,
branch `claude/vpn-audit-2026-10-14`. Companion documents:
[baseline](VPN_PERFORMANCE_BASELINE.md), [security and privacy](VPN_SECURITY_PRIVACY_AUDIT.md),
[roadmap](VPN_OPTIMIZATION_ROADMAP.md), [results](VPN_OPTIMIZATION_RESULTS.md),
[benchmark suite](VPN_BENCHMARK_SUITE.md), raw data in
[`evidence/2026-10-14/`](../evidence/2026-10-14/).

**Evidence limits.** Everything measured here is class B (same-host protocol
overhead) or class C (controlled simulation in network namespaces) on a
disposable VM with no IPv6. There was no VPS (D), no real client network (E)
and no iPhone (F). No production host, credential or external account was
touched. The related repositories (`vpn-web`, `tamara`, `tamara-next`) were not
in this session's scope and were not reviewed. Your target host is an Ubuntu
VPS: the repo lists Ubuntu 22.04/24.04 as **recognized / best-effort** (the
debian-family install path has container CI only; no live Ubuntu install is
verified in the repo), while AlmaLinux is the primary path.

## Answers

**1. How fast is the current VPN under each environment actually tested?**
- B, exit pinned to one 2.8 GHz core, one flow: REALITY ~2.4 Gbit/s down
  (5.3 Gbit/s when the payload is already TLS), Hysteria2 with Salamander
  ~0.5 Gbit/s down/up. Aggregate on that core: REALITY 2.5–3.7 Gbit/s,
  Hysteria2 ~0.55 Gbit/s regardless of user count.
- C, 100 Mbit/s access link: both transports deliver at least 85 % of the
  no-tunnel rate at 20–160 ms RTT and up to 3 % random loss (e.g. 80 ms:
  REALITY 84.9, Hysteria2 76.2, no tunnel 88.8 Mbit/s); at 160 ms / 1 % loss
  Hysteria2 beats plain TCP (66.8 vs 56.3).
- D/E/F: not measured. The only historical real number is one 72 Mbit/s
  Hysteria2 transfer on a Hetzner VPS (2026-09-27), CDN-dependent.

**2. Which protocol provides higher throughput, and under what conditions?**
REALITY on clean or low-RTT paths and whenever the server CPU is the limit
(≈5× cheaper per byte). Hysteria2 keeps more of the link at high RTT with
loss (160 ms / 1 %: 66.8 vs 52.0 Mbit/s). On a typical phone link neither is
the bottleneck. Hysteria2 runs failed 3 of 9 times under loss in the matrix
but 0 of 10 in a targeted repro — unresolved.

**3. Which provides lower latency and better tail latency?**
- New connections: Hysteria2, clearly — ≈1 RTT vs ≈3 RTT for REALITY (at
  160 ms: 170 vs 504 ms p50). This is what users feel as page-load speed.
- Established idle connections: equal (path RTT + 1–3 ms).
- Under load: REALITY, clearly. A download saturating the link through
  Hysteria2 delays every other request on the same QUIC connection by ~0.4–0.5
  s (client-side sing-quic BBR; unchanged in 1.14.3); REALITY adds ~35 ms.
- After a server restart: REALITY (~1 s) — Hysteria2 stalls ~30 s on the
  pinned 1.14.1 server, not on a 1.14.3 server.

**4. What is the current bottleneck?**
For one user: the access network, then REALITY's 3-RTT connection setup on
high-RTT mobile paths. For responsiveness: Hysteria2's same-connection
queueing during downloads and the 30 s restart stall. For capacity: exit CPU
for Hysteria2 (per-packet syscalls without GSO behind Salamander, pprof).
Not the kernel tuning, socket buffers, route evaluation or C-16 rules (none
visible in profiles; no UDP drops). The real host was not measured.

**5. Which existing optimizations work correctly?**
Vision (HTTPS payload forwarded with one TLS layer: ~2× plain-HTTP rate);
BBR on the host (lab used it; it does not need `fq`); fatal-only logging
(0-byte logs after hours of traffic); unchanged renders never restart
sing-box; DNS caching at the exit. Partly: the 16 MiB buffer ceilings help
the client side and REALITY but never reach the Hysteria2 server socket with
Salamander; `default_qdisc` changes only new qdiscs; `Nice=-5` is unmeasured.

**6. Which previously reported defects are fixed?**
Fixed before this audit and checked here: C-16 egress isolation (lab, IPv4),
pinned-hash check order, lease-pool default 0 and the agent report queue
(code). Fixed in
this branch: `vpn-benchmark` mutating production (finding C), the caps
lockout, the agent timeout residual (F04), operator name validation (F17),
the uninstall deleting a foreign sing-box (F13, reproduced live), stale docs
(including the provisioning contract's lease-pool default). Explained, not a
defect: finding A (Hysteria2 throughput is per-packet CPU). Corrected: finding
B's expectations. Confirmed, open: finding D (selection is not
throughput-aware, and Hiddify ignores the selector anyway).

**7. Which security vulnerabilities remain?** No critical or high item found.
Open: own-address C-16 rule inactive on domain installs until
`public_ipv4/6` is recorded (now warned by `vpn doctor`); shared exit
credential between direct and relayed routes (privacy); agent runs as root
(fleet); temp-config write without `O_NOFOLLOW` (defence in depth); nginx
`limit_req` keyed per IPv6 address; the 1.14.1 pin lacks upstream input
validation fixes. Fixed here: an authenticated user could starve the node
through Hysteria2 declared bandwidth (W18). Dependency audit: four rustls
advisories published today, fixed by a lockfile bump in this branch (agent
only).

**8. Which privacy risks remain?** The exit sees every client IP and
destination (in memory; nothing logged at `fatal`); the exit's resolver sees
every destination domain in plaintext; the hairpin node sees all
Google/YouTube flows of every user (owner-approved); one Direct use links a
user's Privacy+ sessions to their IP (shared credential); with Hiddify the
server cannot enforce any client-side rule (DNS, IPv6, kill switch are the
client's). It is a circumvention VPN, not an anonymity system.

**9. What limits real-world reliability?** Every user change restarts the
data plane for everyone (and costs Hysteria2 users ~30 s on 1.14.1); one VPS
with no failover; REALITY depends on the decoy and DNS at handshake time; the
hairpin is a single point of failure for YouTube; Hiddify mixes both
transports per connection, so a page can half-work; mobile handover,
suspend/resume and IPv6 were not testable here.

**10. What is the cost of two-hop routing?** About one extra RTT per new
connection (80 ms total path: 314 vs 262 ms), no UDP/QUIC at all (fails
closed by design), similar bulk throughput on a 100 Mbit/s link (78.8 vs
84.9), a second VPS and its operations. The privacy benefit is mostly
theoretical while Direct and via routes share the exit credential and nothing
resists traffic correlation between the two hops.

**11. Which improvements were implemented and verified?** Ten, listed with
before/after in [results §1](VPN_OPTIMIZATION_RESULTS.md): non-mutating
benchmark; caps lockout; Hysteria2 fairness (BBR for every client without
caps); agent timeouts; name validation; own-address doctor check; uninstall
ownership default; corrected operator/user text; lab tooling; rustls bump.
All with tests; the two data-plane-relevant ones (caps, fairness) with
real-binary interop tests on 1.14.1 and 1.14.3.

**12. Which changes require more measurements?** The 1.14.3 upgrade (VPS
lifecycle + native YouTube checklist); the Hiddify default link (single route
vs balance); R4 on Hiddify's core; capacity and `RcvbufErrors` on the real
host; conntrack/fd headroom; automatic transport selection; the Hysteria2
drops under loss.

**13. Would upgrading the VPS help?** Not for a small group, unless the host
shows it. One 2.8 GHz core moved ~0.55 Gbit/s of Hysteria2 and several Gbit/s
of REALITY; a handful of phones rarely need more. Upgrade only if `vpn doctor
--performance` / `pidstat` show sing-box near a full core or high steal while
users complain. A second core added only 23–31 % to a single flow.

**14. Would switching server providers help?** Possibly more than any server
setting, for two reasons the code cannot fix: YouTube accepts or refuses an
egress IP (INV-1/13), and REALITY's 3-RTT setup scales with RTT to the users,
so a provider closer to them (or with better peering) shortens every page
load. Any change of provider or egress is an owner decision (INV-7/8).

**15. Would newer sing-box versions help?** Yes — 1.14.3 is the most valuable
single change found. A 1.14.3 exit removes the Hysteria2 restart stall for
unchanged clients (lab: 34.6 → 4.3 s) and carries 66 upstream fixes; speed is
unchanged and every real-binary suite this lab can run passes with it (only
the IPv6-only cases could not run). It does not fix the
same-connection delay. It still needs the release process and native
acceptance.

**16. Is a custom sing-box build justified?** No. Both protocol issues found
are upstream matters (one already fixed in 1.14.3, the other a sing-quic
client issue); profiles show kernel and crypto time, not something a fork
removes. Per-user accounting is the only feature argument: per-user
counters exist only in the V2Ray stats API (`with_v2ray_api`, not in the
official build), and the Clash API's connection objects carry no user
(re-checked in the 1.14.1 source). It is not a current requirement. A fork
would add supply-chain and maintenance cost.

**17. Is the architecture suitable for a small private deployment?** Yes.
Strong points: fail-closed C-16 egress policy at two layers, no logs, pinned
and hash-checked binary, careful install/rollback tooling. Weak points that
users will feel: restarts per user change (batch them; move to 1.14.3), the
Hiddify balancer, and one VPS.

**18. What must change before a commercial multi-user rollout?**
Reliable per-user accounting and limits (needs a different data-plane build
or design); user changes without data-plane restarts; high availability with
client-visible failover that works in the main client; separate
credentials per route; verified IPv6 behaviour; real-device acceptance per
release; abuse handling and capacity planning per node; control-plane
security (agent least privilege); a supported OS matrix that includes the
OS actually used (Ubuntu is best-effort today); a jurisdiction review of the
hairpin (every user's Google/YouTube traffic leaves from another node and
country).

## Availability questions from the brief

| Question | Answer | Evidence |
|---|---|---|
| Is the lease-pool default fix present? | Yes: `default_lease_pool_size()` returns 0 with a regression test | code (`apps/provisioning-agent/src/config.rs`) |
| Do templates and existing installs use it? | No template sets the field; a standalone install has no agent at all. Existing nodes: unknown (the 2026-10-11 record had `ru1` rotating every 20 min and the agent was stopped, INV-10). The contract doc said "default 32" — fixed here | code, docs |
| Can managed configs still produce restart storms? | Bounded but yes: with `lease_pool_size > 0`, at most one rotation (= restart) per batch window (default 600 s) plus one per urgent revocation | `lease_pool.rs`, contract |
| Do authorized user operations restart the whole data plane? | Yes, every create/disable/enable/remove/rotate/expiry; unchanged renders do not | code; lab |
| Can restart frequency be reduced without weakening revocation? | Yes: batch non-urgent changes; keep disable/expiry immediate. 1.14.3 makes each restart cheap for Hysteria2 users. sing-box has no live user update | lab |
| Do long connections survive maintenance? | No: every restart ends every connection (REALITY reconnects in ~1 s; Hysteria2 ~30 s on 1.14.1). The lab soak measured connection survival under steady load without restarts (results §3) | lab |

## Multi-node and two-hop

| Question | Answer |
|---|---|
| Supported today | Exit role, relay role with declared exit allow-list and reject-all default, paired subscriptions with Direct and "via relay" routes, Privacy+ pinned profile |
| Only loopback/lab tested | The whole two-hop path in this audit: `two_hop_system` 20/22 here (2 need IPv6), relay egress probes |
| Verified on real VPSs | Not by this audit; the repo marks real two-hop acceptance as out of scope/pending (`TWO_HOP_SYSTEM_TESTS.md`) |
| Static peer endpoints operational | In the lab, yes (relay→exit route 200) |
| Real multi-node orchestration | The provisioning agent and contract exist; the control plane was recorded as unreachable since 2026-10-06; not verified |
| Client-visible failover verified | No. Hiddify balances instead of failing over |
| Direct and relayed routes isolated | Routing yes; credentials no (shared exit credential, W4) |
| Credential scopes independent | Lab: rotation test s10 passes (old fails, new works, scopes independent) |
| Relay failures fail closed | Lab: s06/s07/s17 pass; relay used as an exit gets nothing |
| Exit failure domains represented | Not verified |
| Health checks prove genuine forwarding | `doctor --protocol` proves a real REALITY handshake through the probe principal on the node itself; relay→exit forwarding health on real hosts not verified |
| Can an exit see a client's real IP through unintended direct probes | Yes, with the default multi-route Hiddify profile: the balancer sends some connections to the exit directly. The Privacy+ pinned link removes direct routes (s16 passes) |

## Fairness, admission control and per-user limits

- One user could monopolise a node through Hysteria2 by declaring a high
  bandwidth (lab: 88.5 of 100 Mbit/s, others 0.1–16). Fixed in this branch
  (W18): without operator caps every client is told to use BBR.
- Sharing is otherwise fair in the lab (Jain ≥ 0.94; 0.76 for 25 Hysteria2
  users on one 100 Mbit/s link). Fairness is per connection: a REALITY user
  who opens many parallel connections gets more of a congested link than one
  who opens few; a Hysteria2 device has one congestion controller.
- Per-user rate or connection limits are not available: sing-box 1.14.1 has
  no such option (checked in its option source), and host `tc` cannot tell
  users apart (one sing-box process; client IPs are not users behind NAT).
  They need a different data plane or build, like per-user accounting.
- Admission control is the operator's: users exist only when created;
  `user disable/remove` restarts sing-box at once (lab: a disabled user's new
  connections fail immediately), while an expiry takes effect at the next
  reconcile (`vpn-expiry-reconcile.timer`, every 10 minutes).

## Five highest-value next actions

1. **Move the exit to sing-box 1.14.3** through the release process (new
   `versions.env` hashes from two independent downloads, exact-candidate VPS
   lifecycle, native YouTube/Shorts checklist). It removes the ~30 s
   Hysteria2 outage after every user change and brings 66 upstream fixes.
2. **Run the read-only VPS + iPhone session below** before any tuning: it
   turns the lab classes B/C into D/E/F for the questions that matter
   (restart recovery, responsiveness during downloads, capacity, drops).
3. **Ship this branch's fixes** (fairness, caps, benchmark, uninstall,
   agent, rustls) and record `public_ipv4`/`public_ipv6` on the production
   exit so the own-address rule is active.
4. **Decide the Hiddify default link**: test REALITY-only
   (`compat=hiddify-pinned`) against the current balanced profile on the
   iPhone; single-route is likely more predictable (no mixed fate, no
   Hysteria2 same-connection stalls during video).
5. **Report the sing-quic client BBR delay upstream** with the attribution
   evidence, and recommend REALITY for latency-sensitive use until it is
   fixed.

## Production test plan (Ubuntu VPS + iPhone)

Read-only first; anything mutating is marked and needs the owner's
go-ahead and a maintenance window. Record for every run: date, git commit
and release, sing-box version, `uname -r`, `lsb_release -ds`, CPU model and
vCPUs, RAM, provider/region, client app and version, network (Wi-Fi/LTE,
operator), transport, warm/cold state, sample count.

### 1. Server, read-only (no restarts, no users touched)

```sh
sudo vpn doctor                       # L1–L4, includes the new C-16 own-address line
sudo vpn doctor --performance         # CPU/steal, memory, NIC, qdisc, UDP buffers
sudo vpn-admin doctor --protocol --require-protocol   # real REALITY handshake via the probe principal
uname -r; lsb_release -ds; nproc; free -m; cat /proc/cpuinfo | grep -m1 'model name'
systemctl show sing-box -p NRestarts -p ActiveEnterTimestamp -p MainPID
journalctl -u sing-box -u vpn-expiry-reconcile --since -7d | grep -c 'reloading sing-box'   # restarts per week
nstat -az UdpInErrors UdpRcvbufErrors UdpSndbufErrors       # repeat after an evening of use
ss -s; sudo ss -Huamn 'sport = :443'                         # Hysteria2 socket memory (rb/tb)
sysctl net.core.rmem_max net.core.wmem_max net.ipv4.tcp_congestion_control net.core.default_qdisc
tc qdisc show
pid=$(systemctl show -p MainPID --value sing-box); grep -E 'VmRSS|Threads' /proc/$pid/status; sudo ls /proc/$pid/fd | wc -l
sudo apt-get install -y sysstat conntrack && pidstat -u -p $pid 10 30 && sudo conntrack -C
# benchmark: on a release BEFORE this branch use --skip-tunnel (the old default restarts sing-box 4x)
sudo vpn-benchmark --skip-tunnel --runs 3 --json --output /root/bench-$(date +%F).json
# on a release WITH this branch the default is non-mutating:
sudo vpn-benchmark --runs 3 --json --output /root/bench-$(date +%F).json
```

### 2. Client side (laptop on the users' ISP, and the iPhone)

- Laptop with the official sing-box 1.14.x and the user's subscription
  (`?format=singbox`), one profile per transport: `curl -w` time-to-first-byte
  for 20 new connections per transport (cold), a 60 s download, and a ping
  during the download — on the same QUIC connection for Hysteria2 — to
  reproduce R4 on a real path (`deploy/lab/perf/labtool rtt` works against any
  SOCKS port).
- iPhone + Hiddify, the checklist in `docs/DEVICE_ACCEPTANCE_TESTS.md` plus:
  (a) REALITY-only and Hysteria2-only profiles: page loads during a large
  download; (b) Wi-Fi ↔ LTE switch and 10 min lock/unlock with a video
  paused; (c) the YouTube/Shorts checklist of INV-12 on both transports.

### 3. Mutating tests (owner approval, maintenance window)

- Restart recovery: with the phone idle on Hysteria2, have the operator run
  one `sudo vpn-admin user create --name restart-probe --expires-at $(( $(date +%s) + 3600 ))`
  (or `vpn-admin render-config` after a real change) and time how long the
  next page load takes on the phone; repeat on REALITY. Expect ~30 s vs ~1 s
  on 1.14.1, and ~1 s for both after the 1.14.3 upgrade. Remove the user
  afterwards.
- Live-listener benchmark: `sudo vpn-benchmark --production-listener --runs 3`
  (restarts sing-box twice per transport; expiring user, cleaned up on exit).

### 4. 90-minute soak (release criterion)

On the VPS, read-only:

```sh
sudo bash /opt/singbox-vpn/deploy/lib/soak-sampler.sh --duration 5400 --interval 30 \
     --output /root/soak-$(date +%F).csv --selftest-every 20
python3 /opt/singbox-vpn/deploy/lib/soak_evaluate.py /root/soak-$(date +%F).csv --json
```

Criteria are fixed in `soak_evaluate.py` before the run: ≥ 90 min, services
active in 100 % of samples, 0 unplanned restarts, RSS growth ≤ +25 % and
+64 MiB, fd growth ≤ +50 % and +64, reconcile ran ≥ 8 times and always
succeeded, watchdog active, every protocol self-test passed. During the run
keep real or synthetic load on both transports: two long-lived connections
per transport with a 1 s ping-pong, 20 new connections every 10 s, a 10 s
download every minute (the lab `soak.sh` pattern, from a client machine).
Record any client-side disconnect with its timestamp; there should be none
without a server restart.

### 5. Multi-day stability proposal

Seven days, same sampler with `--interval 60`, rotated daily; plus a client
probe that every 5 minutes opens one new connection per transport and fetches
a small object, logging success and time. Pass: no unplanned restart; restarts
only at user changes (cross-check `journalctl`); daily RSS and fd medians
flat within the soak limits; no growth trend in `UdpRcvbufErrors`; probe
success ≥ 99.5 % per transport per day outside planned restarts; after each
planned restart both transports' probes succeed within 5 s (on 1.14.3).
Include one VPS reboot (planned) and one certificate renewal; record recovery
time for each.
