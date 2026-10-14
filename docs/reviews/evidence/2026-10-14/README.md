# Evidence for the 2026-10-14 performance, security and privacy audit

Raw results behind [`../../2026-10-14-vpn-audit/`](../../2026-10-14-vpn-audit/).
Everything was produced on a disposable Linux VM (4 vCPU Intel Xeon
@ 2.80 GHz, 16 GiB, kernel 6.18.44, `rmem_max`/`wmem_max` 16 MiB, **no IPv6,
no `sch_netem`, no `fq`**) as root,
in network namespaces, with the lab in [`deploy/lab/perf/`](../../../../deploy/lab/perf/),
the pinned sing-box **1.14.1** (SHA-256 checked against `deploy/lib/versions.env`)
and, for comparison runs only, upstream **1.14.3**. Server documents were
rendered by `vpn-admin` built from `af66c63f66b4`; client documents came from
the real `subscription` service. No VPS, phone, real credential or external
account was used. Lab key material was generated for the run and discarded.

Evidence classes: **B** = same-host protocol overhead (no added delay),
**C** = controlled network simulation (userspace delay/loss + `htb` shaping).
Neither is real Internet, VPS or device evidence. Lab topology: client on CPU 2,
exit on CPU 1 (a "1 vCPU" proxy), load generator on CPU 0, delay emulator and
relay on CPU 3. Lab addresses (11.0.x.x) are not real hosts.

## Files

| File | Content |
|---|---|
| `results-matrix-1.14.1.jsonl` | C: no tunnel / REALITY / Hysteria2 / two-hop at 20, 80, 160 ms RTT, 0 % loss, 100 Mbit/s access link (300-packet queue): throughput, new-connection latency, idle and loaded RTT, UDP sessions |
| `results-matrix-1.14.1-loss.jsonl` | the same at 80 ms/1 %, 160 ms/1 %, 80 ms/3 %. Two-hop UDP probes replaced by one `udp_supported:false` record (TCP-only route) |
| `results-classB-1.14.1.jsonl`, `results-classB-1.14.3.jsonl` | B: single-flow throughput and CPU, exit pinned to one CPU (and two), with and without Salamander, REALITY carrying HTTP vs HTTPS |
| `results-ab-reality-payload-1.14.1-vs-1.14.3.jsonl` | B: interleaved A/B re-run of the one class-B difference that looked real (REALITY carrying HTTPS) — it was noise |
| `pprof-reality-download-1.14.1.txt`, `pprof-hy2-download-1.14.1.txt` | `go tool pprof -top` of the exit during a class-B download (lab-only debug listener) |
| `results-concurrency-classB.jsonl` | B: 1/5/10/25/50 simultaneous downloads (REALITY: connections of one client; Hysteria2: separate client processes = separate QUIC connections) — aggregate, fairness, CPU, RSS, fds |
| `results-concurrency-20ms-100m.jsonl` | C: the same through a 20 ms, 100 Mbit/s shared access link (1/5/10/25) |
| `results-concurrency-20ms-netemtun-limited.jsonl` | **invalid as a sing-box measurement**: run with a 1 Gbit/s link, where the userspace delay emulator itself capped traffic at ~105–165 Mbit/s. Kept only so the run is not silently dropped |
| `results-brutal-80ms-1pct.jsonl` | C: Hysteria2 fixed-rate (Brutal) clients declaring 90 and 200 Mbit/s vs the default BBR client, exit with opt-in caps 1000/1000 |
| `loaded-latency-20ms.jsonl`, `bbr-profile-20ms.jsonl` | C: where the Hysteria2 loaded-latency goes (same vs separate QUIC connection, REALITY, cross-transport, server BBR profiles). The first two `conservative` rows failed because the client hit the restart stall below |
| `sameconn-ratelimit-20ms.jsonl` | C: same-connection ping with the download sender-limited to 50/80 Mbit/s (the first Hysteria2 "unlimited" row reached only 30 Mbit/s and is not comparable) |
| `sameconn-queue-20ms.jsonl` | C: client UDP receive queue, exit UDP send queue and UDP drop counters during the same-connection test (all ~empty) |
| `sameconn-direction-20ms.jsonl`, `sameconn-wire-20ms.jsonl` | C: plaintext captures on both ends: the ~0.5 s is between the client app handing over the request and the client putting it on the wire |
| `sameconn-client-variants.jsonl` | C: client `bbr_profile` variants, `disable_chrome_parrot`, and a lab-only client build whose sender uses a fixed-rate controller (523 → 45 ms) |
| `sameconn-up-only-declaration.jsonl` | C: a client that declares only `up_mbps` still runs BBR on both sides in sing-quic (the exit answers `RxAuto`), so it is not a workaround |
| `restart-recovery-*.jsonl` | what clients see when the exit restarts (every user change): REALITY vs Hysteria2, ICMP delivered/dropped, client active/idle, with/without Salamander, official build vs the upstream-fix prototype |
| `restart-and-sameconn-1.14.3.jsonl` | sing-box **1.14.3 on both ends**: idle-client restart recovery (Hysteria2 4.3 s, 0 failed — the stall is gone) and the same-connection ping (524 ms — unchanged). Run on the second VM (see below) |
| `restart-recovery-1.14.3-server-vs-client.jsonl` | which side fixes the stall: server 1.14.3 + clients 1.14.1 → 4.3 s; server 1.14.1 + clients 1.14.3 → 34.6 s. Second VM |
| `hy2-server-idle-timeout-20s.jsonl` | C: exit `idle_timeout: "20s"` on the Hysteria2 inbound: client keepalive packet count unchanged (10 in 35 s idle), restart stall 34.6 → 24.6 s |
| `egress-probe.jsonl` | C-16 egress probes through both transports (loopback, own IP, rebinding, metadata, RFC 1918, TCP/25, relay used as an exit) |
| `own-ip-gap.jsonl` | the own-public-address gap when `public_ipv4` is not recorded (domain installs) |
| `revocation-fresh-clients.jsonl` | disable → 40 s of retries from **new** client processes on both transports (0 successes in 39 attempts each) → re-enable |
| `hy2-close-repro.jsonl` | 10 + 10 runs at 80 ms / 3 % loss trying to reproduce the "control socket has closed unexpectedly" Hysteria2 failures seen in the matrix: 0/10 on either transport |
| `greedy-client-20ms-100m.jsonl` | C: three users share a 20 ms, 100 Mbit/s link; one Hysteria2 user declares 500 Mbit/s. Exit as rendered on `main` vs with `ignore_client_bandwidth` (no caps) |
| `hy2-bandwidth-caps.txt` | the opt-in Hysteria2 bandwidth caps locked out every client that declares no bandwidth (before this branch) and do not after |
| `tcpdump-reality-cold.txt`, `tcpdump-hy2-cold.txt` | packet timing of one new proxied request per transport at 80 ms RTT (client side) |
| `upstream-proposal/` | lab-only patches: `sing-quic-close-on-shutdown.patch` + `sing-box-inbound-close-order.patch` (send QUIC `CONNECTION_CLOSE` to established Hysteria2 clients when the server stops; built to confirm the mechanism — sing-box 1.14.3 already does the equivalent through its new lifecycle `Scope`, so there is nothing to send upstream for this one) and `LAB-ONLY-client-fixed-rate-cc-attribution.patch` (attribution of the same-connection delay only, **not** a fix) |

## Method notes that matter when re-running

- `curl` honours `NO_PROXY`/`no_proxy`, also for `--socks5-hostname`. On a host
  whose `NO_PROXY` lists private or link-local ranges, a "tunnel" request to a
  literal private address silently goes direct. Every C-16 probe here runs
  under `env -i` with `--noproxy ''` (the first literal-address attempt in this
  audit was invalid for exactly this reason and was repeated).
- The lab DNS server returns an SOA record with negative answers. Without it,
  sing-box re-queried the server name for every connection and added one full
  RTT to every cold-connection sample.
- netem-tun and the relay shared CPU 3 in the matrix runs; two-hop numbers are
  therefore a lower bound under CPU contention. netem-tun cannot forward much
  more than ~150 Mbit/s; keep shaped tests at or below 100 Mbit/s.
- Restarting the exit while a Hysteria2 client keeps running costs that client
  up to ~30 s (see `restart-recovery-*`). Lab scripts that change the exit
  restart the clients too; a stalled first run after a restart is this effect,
  not a measurement.
- A cargo build overlapped one matrix condition and which one was not
  recorded; read a single outlier in the matrix files with that in mind.
- The session's worker restarted once, and the later runs executed on a
  **different VM**: Xeon @ 2.10 GHz with 4 MiB socket-buffer ceilings. Files
  from that host: `sameconn-queue-20ms`, `sameconn-direction-20ms`,
  `sameconn-wire-20ms`, `sameconn-up-only-declaration`,
  `sameconn-client-variants`, `greedy-client-20ms-100m`,
  `restart-and-sameconn-1.14.3`, `restart-recovery-1.14.3-server-vs-client`
  and `vpn-benchmark-default-run-lab.json`. The second VM ran Ubuntu 24.04.5
  LTS; the first VM's OS was not recorded. No singbox-vpn install was run on
  either. The 1.14.1 restart stall
  reproduced there unchanged (34.55 s) inside the server-vs-client run. They are
  only compared within themselves; the same-connection delay reproduced there
  at the same value (523–525 ms at 20 ms RTT). `lab-net.sh up` now sets the
  16 MiB ceilings itself.
- `vpn-benchmark-default-run-lab.json`: one `--quick` run of the new default
  (non-mutating) `vpn-benchmark` inside the lab exit namespace, real sing-box,
  to show the disposable-instance path works end to end. Its numbers come from
  an unpinned loopback instance and are not comparable to the class-B table.
