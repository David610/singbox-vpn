# Benchmark suite and instructions — 2026-10-14

Three groups, never mixed up. Results from one group are not evidence for
another (classes A–F in [the baseline](VPN_PERFORMANCE_BASELINE.md)).

## 1. Read-only on a real server (safe any time)

Nothing here restarts sing-box, creates users or reads credentials.

| Tool | Measures | Command |
|---|---|---|
| `vpn doctor` | L1–L4 health, listeners, firewall, C-16 own-address rule | `sudo vpn doctor` |
| `vpn doctor --performance` | CPU, steal, memory, NIC, qdisc, buffers (measurements only) | `sudo vpn doctor --performance` |
| `vpn-admin doctor --protocol` | a real REALITY handshake through the reserved probe principal | `sudo vpn-admin doctor --protocol --require-protocol` |
| `vpn-benchmark` (this branch or later) | host, path (ping/MTU), raw download, and a **disposable loopback** REALITY/Hysteria2 instance with throwaway keys | `sudo vpn-benchmark --runs 3 --json --output /root/bench.json` |
| `vpn-benchmark --skip-tunnel` (older releases) | the same without the protocol layer; use this on releases before this branch, whose default restarted sing-box 4× | `sudo vpn-benchmark --skip-tunnel --runs 3` |
| `vpn-benchmark --compare` | numeric deltas between two JSON runs | `vpn-benchmark --compare before.json after.json` |
| soak sampler + evaluator | 90-min or longer service health, restarts, RSS/fd growth, reconcile, self-tests | `deploy/lib/soak-sampler.sh` → `deploy/lib/soak_evaluate.py` (see the assessment's test plan) |
| kernel counters | UDP drops, socket memory, connection counts | `nstat -az UdpRcvbufErrors UdpSndbufErrors`, `ss -Huamn 'sport = :443'`, `conntrack -C` |

## 2. Mutating on a real server (owner approval, maintenance window)

| Tool | What it changes | Command |
|---|---|---|
| `vpn-benchmark --production-listener` | creates a temporary user that expires after 30 min and removes it (on exit and on INT/TERM/HUP); **each create/remove restarts sing-box** and disconnects every user | `sudo vpn-benchmark --production-listener --runs 3` |
| lifecycle acceptance | full install/update/rollback cycle, including the live-listener benchmark | `deploy/almalinux/lifecycle-acceptance.sh` (passes `--production-listener` explicitly) |
| restart-recovery check | one real user change while a phone is idle on each transport; time to the next working page load | see the assessment's test plan §3 |

## 3. Lab only (disposable machine, never a VPN server)

[`deploy/lab/perf/`](../../../deploy/lab/perf/README.md): network namespaces, a
userspace delay/loss emulator, `htb` shaping, the pinned sing-box, configs from
the production renderer, lab-only keys. It changes the machine it runs on
(namespaces, `/etc/netns/*`, socket-buffer ceilings).

| Script | Measures | Class |
|---|---|---|
| `matrix.sh` | throughput, new-connection latency, idle and loaded RTT, UDP sessions over an RTT × loss grid, per transport and two-hop | C |
| `classb.sh LABEL BINARY` | CPU-bound throughput and CPU %, with/without Salamander, TLS vs plain payload, 1 vs 2 CPUs | B |
| `profile.sh reality\|hy2` | CPU profile of the exit during one download | B |
| `concurrency.sh T N DUR` | 1–50 simultaneous users: aggregate, fairness (Jain), CPU, RSS, fds, latency under load | B/C |
| `loaded-latency.sh CASE` | where latency goes under load (same vs separate QUIC connection, cross-transport) | C |
| `restart-recovery.sh` | what clients see when the exit restarts (`PRE_IDLE`, `IDLE_GAP`, `SERVER_CFG`) | C |
| `brutal.sh DECLARED` | fixed-rate Hysteria2 clients against the BBR default | C |
| `egress-probe.sh` | C-16 through both transports, relay fail-closed | C |
| `soak.sh DURATION` | long-lived connections, churn and periodic bulk with RSS/fd/CPU sampling | C |
| `outage.sh SECONDS` | full client↔exit outage: open-connection survival and time to the first working new request | C |
| `summarize.sh FILE` | Markdown tables from matrix output | — |

Rules that keep comparisons honest: same condition for every transport in a
comparison, at least three repetitions, medians with every run shown,
interleave A/B runs when a difference matters, restart the clients whenever
the exit restarts (otherwise Hysteria2's restart stall pollutes the next
run), keep shaped links ≤ 100 Mbit/s (the emulator caps near 150 Mbit/s),
and run curl through SOCKS under `env -i … --noproxy ''`.
