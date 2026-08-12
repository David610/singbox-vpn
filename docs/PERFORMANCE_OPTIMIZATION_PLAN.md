# Performance Optimization Plan

Status: implemented (P0/P1 items below are shipped in this pass). This
document is the evidence trail for those changes and the measurement
methodology for everything that still needs production data.

## How to read this document

Every claim below is tagged:

- **VERIFIED (code)** — confirmed by reading this repository directly;
  not in dispute.
- **VERIFIED (upstream)** — confirmed against current sing-box/Hysteria2
  documentation.
- **SUSPECTED — needs production measurement** — plausible from the
  architecture, but this repo has no access to the production VPS, so it
  is NOT claimed as fact. Use `vpn doctor --performance` and
  `vpn-benchmark` (both shipped in this pass — see below) to confirm or
  refute it on the real host before acting on it further.

Nothing here was "optimized" by guessing. Every code change below either
fixes a verified gap (something the upstream vendor documents as
mattering, that this repo's deploy pipeline simply never configured) or
adds the instrumentation needed to make the next decision from evidence.

## Executive summary

The reported slowness has at least one **structural** cause that was
confirmed directly in the code, independent of any network measurement:
**this deployment never configured any of the kernel-level network
tuning that Hysteria2/QUIC's own upstream documentation says to apply**
(UDP socket buffers, TCP congestion control). That gap is fixed in this
pass (P0). Two other credible contributors — the deliberately
conservative REALITY-by-default subscription behavior, and Hysteria2
running without a fixed bandwidth hint — are also addressed, as opt-in
capabilities, not silent default changes, because the evidence does not
support flipping either default outright (see the per-item write-ups).

Everything else the task description asked us to "verify" — CPU steal,
actual packet loss to Russia, whether the VPS's own uplink is
oversubscribed, whether 1 vCPU is actually saturated — **cannot be
verified from this repository**. There is no access to the production
VPS in this environment. Those items are listed under "Suspected —
needs production measurement" with the exact commands to run.

---

## Verified bottlenecks (confirmed in code)

### V1. No kernel network tuning existed anywhere in the deploy pipeline — P0, FIXED

**Evidence (code):** repo-wide search for `sysctl`, `rmem`, `wmem`,
`tcp_congestion_control`, `bbr`, `fq_codel`, `net.core`, `net.ipv4`
across `deploy/`, `crates/`, `apps/`, `services/`, and every `docs/*.md`
returned zero matches before this pass. `deploy/almalinux/install.sh`'s
17 stages configured OS packages, sing-box, systemd units, TLS, REALITY
keys, nginx, and the firewall — never the kernel network stack.

**Evidence (upstream):** the official Hysteria2 performance guide
(`https://v2.hysteria.network/docs/advanced/Performance/`) explicitly
recommends raising `net.core.rmem_max`/`net.core.wmem_max` to 16 MiB on
Linux for QUIC/UDP throughput, because these are *ceilings* a UDP socket
can request via `setsockopt` — the OS default (a few MB on most distro
kernels) can cap Hysteria2's actual throughput independent of CPU,
network, or sing-box's own code.

**Fix shipped:** `deploy/lib/perf-tuning.sh`, sourced by
`install.sh`/`update.sh`, writes `/etc/sysctl.d/99-vpn1-dataplane.conf`
with `net.core.rmem_max = 16777216` / `net.core.wmem_max = 16777216`,
idempotently (re-running install/update is a no-op if unchanged).

**Expected benefit:** removes a real ceiling on Hysteria2 throughput
under load; helps most on a lossy/high-BDP (bandwidth-delay product)
path — exactly the "Russia-facing" scenario in the task description.

**Downside:** none identified. These are ceilings, not forced
allocations — raising them does not increase memory use unless a socket
actually asks for more, and 16 MiB per busy socket is negligible against
2 GB RAM for the handful of concurrent Hysteria2 sessions this
deployment handles.

**Rollback:** `rm /etc/sysctl.d/99-vpn1-dataplane.conf && sysctl --system`.

### V2. No TCP congestion control tuning for the REALITY/TCP path — P0, FIXED

**Evidence (code):** same search as V1 — `tcp_congestion_control` was
never set; the host's distro-default congestion control (`cubic` on
stock AlmaLinux/Ubuntu/Debian kernels) was left in place for the
VLESS+REALITY TCP/443 path.

**Evidence (upstream):** BBR is widely documented (including by
sing-box/Xray community deployment guides) as measurably better than
cubic for long-lived, potentially lossy proxy connections, because it is
not purely loss-based — cubic treats any packet loss as a congestion
signal and halves its window, which is a poor fit for a path with
non-congestive loss (common on cross-border routes). This is *not* claimed
as a sing-box-specific optimization — it is a general kernel TCP setting
that benefits any TCP workload on a lossy path, including REALITY.

**Fix shipped:** `deploy/lib/perf-tuning.sh` checks
`/proc/sys/net/ipv4/tcp_available_congestion_control` at install/update
time. Only if the running kernel actually reports `bbr` available does
it set `net.ipv4.tcp_congestion_control = bbr` and
`net.core.default_qdisc = fq` (falling back to `fq_codel` if the `fq`
qdisc module isn't loadable) — BBR's documented pairing, since BBR paces
its own writes and a plain FIFO qdisc's drop behavior fights that pacing.
**Never forced on a kernel that doesn't support it** — this was an
explicit requirement and is enforced by the `perf_kernel_supports_bbr`
check before anything is written.

**Expected benefit:** better TCP throughput/latency stability under loss
on the REALITY path, on any kernel that already ships BBR (Linux ≥ 4.9,
which is effectively every supported AlmaLinux 9 / Ubuntu 22.04+ /
Debian 12+ kernel).

**Downside:** BBR can be less fair to other TCP flows sharing a
bottleneck link in some topologies — a known, debated tradeoff in the
congestion-control literature, not specific to this deployment. Given
this host's role (a VPN egress, not a general-purpose multi-tenant
router), this tradeoff is judged acceptable.

**Rollback:** delete `/etc/sysctl.d/99-vpn1-dataplane.conf`, run
`sysctl --system` (reverts to the distro's default congestion control on
next reboot / immediately for new connections).

### V3. Hysteria2 had no bandwidth hint / Brutal congestion control option — P1, FIXED (opt-in, default unchanged)

**Evidence (code):** `crates/compat-config/src/server.rs`'s
`render_singbox_server_config` never set `up_mbps`, `down_mbps`, or
`ignore_client_bandwidth` on the Hysteria2 inbound before this pass —
confirmed by direct inspection and a repo-wide grep. sing-box's Hysteria2
implementation without those fields defaults to its adaptive BBR-based
congestion control.

**Evidence (upstream):** sing-box's Hysteria2 docs describe Brutal (a
fixed-rate mode using `up_mbps`/`down_mbps` + `ignore_client_bandwidth:
true`) as the recommended setting **"for servers whose admin already
knows the real bandwidth."** The same docs warn that setting these
higher than the network can actually sustain "will backfire, causing
network congestion and unstable connections" — i.e. a guessed value is
not a safe default.

**Fix shipped:** `Hysteria2ServerParams::{up_mbps,down_mbps}` (both
`Option<u32>`, both-or-neither enforced by
`DeploymentConfig::validate`), rendered into the Hysteria2 inbound only
when both are set, alongside `ignore_client_bandwidth: true`. **Left
unset by default** — the task explicitly said not to hardcode a guessed
bandwidth, and this repo has no measurement of the production VPS's real
throughput to base a default on. An operator who runs `vpn-benchmark`
and gets a real sustained-throughput number can now set
`up_mbps`/`down_mbps` in `deployment.toml` and re-run
`vpn-admin render-config`.

**Expected benefit:** if the real bandwidth is known and set correctly,
Brutal mode gives Hysteria2 more consistent throughput under loss than
BBR's adaptive backoff — this is the upstream vendor's own stated reason
for the feature.

**Downside:** a wrong (too high) value causes self-induced congestion —
this is why it stays opt-in with the operator required to supply a
*measured*, not guessed, number.

**Rollback:** remove `up_mbps`/`down_mbps` from `[hysteria2]` in
`deployment.toml`, run `vpn-admin render-config`.

### V4. No `Nice=` scheduling hint on the sing-box systemd unit — P1, FIXED

**Evidence (code):** `deploy/almalinux/systemd/sing-box.service` set
`LimitNOFILE=65535` and extensive sandboxing directives, but no
`Nice=`/`CPUSchedulingPolicy=` — sing-box ran at default priority,
competing equally with nginx, sshd, cron, and everything else on a
1-2 vCPU host.

**Evidence (upstream):** the Hysteria2 performance guide advises raising
process priority moderately under contention, explicitly cautioning
against jumping straight to real-time scheduling and to test
incrementally rather than maximizing priority.

**Fix shipped:** `Nice=-5` added to `sing-box.service` — a modest
priority bump, not `CPUSchedulingPolicy=fifo`/`rr` (which the task
explicitly said to avoid, since it can starve the rest of the host).

**Expected benefit:** matters only under real CPU contention (a 1 vCPU
host running nginx + sshd + sing-box + the admin/subscription binaries
simultaneously) — negligible on an idle host, plausibly meaningful during
a burst.

**Downside:** minor — other processes get slightly less CPU time under
contention. `-5` is small enough that this is not expected to be
noticeable for anything except very CPU-starved moments.

**Rollback:** delete the `Nice=-5` line from
`deploy/almalinux/systemd/sing-box.service`, then
`systemctl daemon-reload && systemctl restart sing-box`.

### V5. sing-box version pin — VERIFIED, no action needed

**Evidence:** `deploy/almalinux/install.sh:35` pins `SINGBOX_VERSION="1.13.14"`.
As of this writing that is upstream's current **stable** release (the
next tag, `v1.14.0-beta.2`, is a beta and was correctly NOT adopted, per
the task's explicit instruction not to move to a pre-release for
features). No version change made.

### V6. Subscription defaulted to VLESS+REALITY, not a throughput-based race — VERIFIED, addressed as an opt-in profile, default unchanged

**Evidence (code):** `crates/compat-config/src/render.rs`'s
`render_singbox_client_subscription` builds a manual `selector` outbound
whose `default` is the first VLESS+REALITY endpoint's tag, and points
`route.final` at that selector — not at the `urltest` (`auto`) group.
This is confirmed **deliberate**, not an oversight: the function's own
doc comment and dedicated tests
(`selector_default_is_reality_and_lists_hysteria2_and_auto`,
`route_final_points_at_manual_selector_not_urltest`) document that this
was a specific decision made during a "Telegram-reliability pass" (see
`docs/TELEGRAM_RESILIENCE_PLAN.md`), because `urltest`'s
`https://www.gstatic.com/generate_204` probe only proves generic HTTPS
reachability — it says nothing about sustained throughput, long-lived
connections, or behavior under active DPI, and Hysteria2/QUIC is more
exposed to UDP blocking/throttling than REALITY's TCP/443 disguise.

**Does this explain the reported slowness?** Partially, and only in one
direction: if Hysteria2 would in fact be *faster* on this user's network
right now, they are on REALITY by default and would need to switch
manually. That is a real, verified fact about current behavior. But
unilaterally flipping the default to Hysteria2 — or to a plain
`urltest` race — was explicitly out of scope: `urltest` provides no
throughput signal at all (sing-box's `urltest` outbound type only
measures connection latency/success, it has no throughput-race mode),
and `crates/network-state`/`crates/failure-classifier` (checked
directly) currently record only boolean success/failure per transport,
not latency or throughput — so there is no data source in this codebase
today that could drive a genuinely throughput-aware automatic selector.
Building one is real instrumentation work, not a config flag; see
"Suspected / future work" below.

**Fix shipped:** `SelectionProfile` (`reliability` / `performance` /
`auto`) in `crates/compat-config/src/render.rs`, exposed via
`?profile=` on the subscription URL
(`services/subscription/src/lib.rs`). `reliability` (REALITY default) is
unchanged and remains the default when `profile` is omitted — no
existing subscription URL changes behavior. `performance` sets the
selector's default to Hysteria2 for users who want to opt in.  `auto`
sets the selector's default to sing-box's own `auto` (urltest) group.
Every profile still lists every transport in the selector, so a user can
always override by hand regardless of profile — this does not remove or
weaken the REALITY fallback.

**Expected benefit:** users who know Hysteria2/QUIC is unblocked and
faster for them can opt in with one URL parameter, without anyone
changing the fleet-wide default (which stays the DPI-resilient choice).

**Downside:** none for existing users (default unchanged). `performance`
and `auto` profiles trade some of REALITY's DPI resilience for
Hysteria2's typically-higher throughput when it isn't blocked/throttled
— an explicit, informed tradeoff the user opts into, not a surprise.

**Rollback:** none needed — omit `?profile=` (or pass
`?profile=reliability`) to get the original, unchanged behavior.

---

## Suspected — needs production measurement

None of the following can be confirmed or refuted from this repository.
They require running the shipped diagnostics (`vpn doctor --performance`,
`vpn-benchmark`) **on the actual production VPS**, ideally with a
--target-host on the actual client-side (Russian ISP) network path.

| # | Suspected factor | How to check | What would confirm it |
|---|---|---|---|
| S1 | CPU steal is significant (noisy-neighbor host) | `vpn doctor --performance` (steal %), or `vpn-benchmark` | Steal consistently > ~5-10% during load |
| S2 | 1 vCPU is saturated by Hysteria2/QUIC userspace crypto under real load | `vpn-benchmark`'s "sing-box client CPU ticks during one transfer", or `top -H -p $(pgrep sing-box)` during a real transfer on the VPS | sing-box pinned near 100% of one core while throughput plateaus below line rate |
| S3 | Packet loss / high RTT / jitter on the Russia→VPS path specifically | `vpn-benchmark --target-host <a host on that path>`, or `mtr <target>` | Loss consistently > ~1-2%, or RTT/jitter far above geographic expectation |
| S4 | The VPS provider's own uplink is bandwidth-capped or oversubscribed | `vpn-benchmark`'s "Raw VPS throughput" section, compared against the provider's advertised rate, run at different times of day | Raw (non-tunneled) download throughput consistently well below advertised, independent of anything VPN-related |
| S5 | MTU/PMTU issues on the path | `vpn-benchmark`'s MTU/PMTU probe | Effective path MTU well below 1500, or asymmetric behavior between raw and tunneled tests |
| S6 | DNS resolution latency inside the tunnel | not currently instrumented | would need a dedicated DNS-timing probe — not built this pass; flag if S1-S5 don't explain observed slowness |

**None of these should be "fixed" by guessing.** If S1/S2 come back
positive (CPU-bound, high steal), the correct action is a VPS resize —
see "1 vCPU → 2 vCPU" below — not more kernel tuning. If S3/S4/S5 come
back positive, the bottleneck is the network path or the provider, and
**no server-side software change in this repository can fix that** —
say so to the user plainly rather than attempting more sysctl tuning
that cannot address a routing/peering problem.

### On "1 vCPU → 2 vCPU"

Per the task's explicit instruction, this is a *conditional*
recommendation, not something implemented or assumed: **if and only if**
`vpn doctor --performance`/`vpn-benchmark` on the real VPS show sing-box
CPU-bound (S2) and/or high CPU steal (S1) during real Hysteria2/VLESS
load, moving from 1 vCPU to 2 modern vCPUs is the concrete next step —
Hysteria2/QUIC's userspace crypto and congestion-control processing is
CPU-bound work that benefits directly from more cores, and a 1-vCPU host
has zero headroom for the OS scheduler to move that work anywhere when
another process (nginx TLS termination, sshd, cron) needs the CPU at the
same moment. **RAM is not recommended to be increased** — nothing in
this investigation found evidence of memory pressure (see `vpn doctor
--performance`'s RAM/swap section), consistent with the task's
instruction not to recommend more RAM without evidence.

### Future work not implemented this pass (and why)

- **Throughput/latency-aware automatic transport selection.** Real
  automatic selection based on "connection success; latency; sustained
  throughput; recent failures; UDP availability; censorship symptoms"
  (as the task described) needs new data collection —
  `crates/network-state`'s `Observation` type currently has no
  latency/throughput field, only `Outcome::{Success, Failure(category)}`.
  Adding that is a real instrumentation project (new probe logic, new
  fields, new scoring in `crates/policy`), not a config change, and
  doing it hastily risks a worse regression than the current
  conservative default. The `SelectionProfile` mechanism shipped this
  pass gives users a manual lever today without pretending to have built
  the data-driven version.
- **DNS-latency-inside-tunnel measurement (S6).** Not built; flag as a
  next diagnostic if S1-S5 don't explain reported slowness.
- **Firewall conntrack/rate-limit/MTU rules.** No evidence was found
  (code or upstream) that this deployment's firewall scripts
  (`deploy/almalinux/firewall.sh`/`firewall-ufw.sh`) are a bottleneck —
  they only open ports. Speculatively adding conntrack tuning without
  evidence would violate the task's explicit instruction not to dump
  unjustified tweaks into the installer.

---

## Priority summary

| Priority | Item | Status |
|---|---|---|
| P0 | V1: UDP socket buffer sysctls (rmem_max/wmem_max) | Shipped |
| P0 | V2: BBR + fq/fq_codel, gated on kernel support | Shipped |
| P0 | Measurement tooling: `vpn doctor --performance`, `vpn-benchmark` | Shipped |
| P1 | V3: Hysteria2 Brutal bandwidth, opt-in | Shipped (off by default) |
| P1 | V4: `Nice=-5` on sing-box.service | Shipped |
| P1 | V6: `SelectionProfile` (reliability/performance/auto) | Shipped (default unchanged) |
| P2 | S1-S6: production-only measurements | Not actionable without VPS access — run the tooling above |
| P2 | VPS resize (1→2 vCPU) | Conditional on S1/S2 — do not do preemptively |
| P2 | Throughput-aware auto-selection | Future work — needs new instrumentation |

---

## Measurement tooling shipped this pass

### `vpn doctor --performance`

Read-only host/kernel/network **measurements** (never a recommendation,
never pass/fail): CPU model, vCPU count, load average, instantaneous
CPU utilization and %steal, RAM/swap, primary interface + MTU + qdisc,
current and available TCP congestion control, `net.core.rmem_max`/
`wmem_max`, cumulative TCP retransmit / UDP error counters from
`/proc/net/snmp`, and sing-box's own PID, nice value, open-file-descriptor
limit, and CPU-ticks-consumed. Every metric this process cannot read on
the running host prints `unavailable`, never a guess. Implemented in
`apps/admin/src/main.rs` (`cmd_doctor_performance` and its `perf_*`
helpers) — pure `/proc`/`/sys` reads plus `ip`/`tc`/`ps`, no new
dependencies.

### `vpn-benchmark` (`deploy/lib/vpn-benchmark.sh`)

A repeatable, multi-sample (`--runs`, default 3; reports min/median/max,
never a single number) benchmark across the layers the task asked for:

1. **Host** — CPU/RAM/steal/load (bash-only, no Rust binary dependency).
2. **Network path** (`--target-host`) — packet loss, RTT/jitter (`ping`),
   hop-by-hop loss (`mtr`, if installed), MTU/PMTU via a binary-search
   `ping -M do` probe.
3. **Raw VPS throughput** — a plain HTTPS download with no tunnel
   involved, isolating "this VPS's own uplink" from "the tunnel
   software." (No public upload-accepting endpoint is assumed to exist;
   the script tells the operator to use `iperf3` against a second host
   they control if upload matters.)
4. **VLESS+REALITY / Hysteria2 throughput** — a REAL sing-box client,
   built from this deployment's own live subscription JSON (fetched from
   the local subscription backend), dials this server's own public
   listener over the real network through a throwaway benchmark user
   (created and deleted by the script, never touching real users' data),
   while sampling sing-box's own CPU ticks during the transfer.

Every step that needs a tool or a live component this host doesn't have
prints `SKIPPED: <reason>` and continues — it never fabricates a number.
The final "Assessment" section is explicitly a decision *guide*, not a
verdict — it tells the operator how to read the numbers above it, it
does not compute one itself, consistent with "never claim a performance
improvement without comparing against a baseline" and "if the actual
bottleneck is bad VPS routing/peering, say so."

---

## What to run on the actual VPS

```
vpn doctor --performance
vpn-benchmark --runs 5 --target-host <a host on your users' real ISP path>
```

Then use the decision table in "Suspected — needs production
measurement" above. In particular:

- If raw VPS throughput (layer 3) is already far below the provider's
  advertised rate, with high CPU steal → **this is a provider problem**;
  no sysctl or sing-box change in this repository can fix it. Consider
  moving providers or filing a support ticket with the raw-throughput
  numbers as evidence.
- If raw throughput is fine but tunneled throughput is much lower with
  sing-box CPU low → **network path** (loss/PMTU/congestion), re-check
  `--target-host` packet loss/RTT and the MTU probe.
- If tunneled throughput is much lower AND sing-box is pinned near 100%
  of one core → **CPU-bound**; this is the case where a 2-vCPU resize is
  justified.
