# VPN performance baseline — 2026-10-14

Repository `David610/singbox-vpn`, branch `main` at `af66c63f66b4`
(workspace 1.3.1; latest stable tag v1.3.0), pinned sing-box **1.14.1**
(SHA-256 matches `deploy/lib/versions.env`). Upstream stable is **1.14.3**.

**Read the evidence classes first.** Every new number in this document is
class **B** (same-host protocol overhead) or class **C** (controlled network
simulation) from the lab in [`deploy/lab/perf/`](../../../deploy/lab/perf/).
None is a VPS (D), real client network (E) or native iPhone (F) result, and
none should be quoted as "the VPN's speed".

| Class | Meaning | In this audit |
|---|---|---|
| A | host-only | lab host: 4 vCPU Intel Xeon 2.8 GHz, 16 GiB, kernel 6.18.44 (Firecracker VM), no IPv6, no `sch_netem`, no `fq` |
| B | same-host protocol overhead | yes |
| C | controlled network simulation | yes (userspace delay/loss, `htb` 100 Mbit/s access link, 300-packet queue) |
| D | real VPS-to-VPS | no |
| E | real client-to-VPS | no |
| F | native mobile acceptance | no |

## 1. Architecture: the real packet path

```
iPhone app ──► client TUN (Hiddify core / Tamara) ──► client routing
   Hiddify: our selector/urltest/route are discarded; ≥2 routes become a
   per-connection round-robin "balance" group (repo analysis of hiddify-core)
        │
        ├─ REALITY:  one TCP + TLS 1.3 (uTLS chrome, REALITY auth, Vision)
        │            connection per proxied TCP connection AND per UDP session
        │            ──► exit :443/tcp
        └─ Hysteria2: one QUIC connection (UDP :443, Salamander-obfuscated)
                     carrying every proxied TCP stream and UDP session
                     ──► exit :443/udp

exit: sing-box.service (user sing-box, Nice -5, 1 process)
  REALITY inbound: per new connection, dial the decoy (handshake.server:443)
  route (in order):
    [probe principal: loopback self-test, 3-destination allow-list, reject]
    resolve (host resolver; sing-box DNS cache)            ← C-16
    reject C-16 IPv4 / IPv6 ranges, TCP/25, own public IP  ← C-16
    [ipv6_literal = reject_google: sniff v6:443 ≤100 ms, reject Google v6]
    [hairpin: sniff all → reject Google literal v6 → Google domains ──► paired
     node via VLESS+REALITY+Vision (a new TCP+TLS connection per proxied
     connection and per UDP session)]
    final: direct (happy-eyeballs dial to the checked addresses)
  host nftables output chain (uid sing-box): C-16 ranges, TCP/25
  ──► kernel ──► NIC ──► destination

two-hop: client ──REALITY/TCP──► relay (allow-list: exit:443/tcp; reject rest)
         ──TCP──► exit; the inner REALITY session is client↔exit. TCP only.
```

### Where cost appears

| Cost | REALITY (VLESS + Vision) | Hysteria2 (Salamander) |
|---|---|---|
| New proxied TCP connection | client TCP handshake + REALITY/TLS handshake + first data = **≈3 client↔exit RTT**, plus the exit's decoy dial (TCP + TLS exchange, ≈2 decoy RTT) and destination dial | **≈1 RTT** (new stream on a warm QUIC connection); the QUIC connection itself costs 1 RTT once |
| New UDP session (QUIC apps) | a new TCP + TLS connection (UDP over TCP): **≈3 RTT** | ≈1 RTT |
| Per byte, server CPU | TLS records (AES-GCM, AES-NI) + large TCP writes, kernel segmentation | per ~1.2 KB packet: QUIC AEAD + header protection + Salamander BLAKE2b/XOR + one syscall; Salamander's wrapper hides the socket from quic-go, so no GSO/batched I/O and no socket-buffer sizing on the server |
| Copies | kernel→user→kernel per direction | same, per packet |
| Loss recovery | TCP per proxied connection (head-of-line blocking within that connection only) | QUIC per stream; one congestion controller per device connection |
| Extra connections | one per proxied connection (no mux with Vision) | one per device |

Route evaluation (`resolve`, CIDR sets) and the nftables output chain are per
connection or per packet but cheap; they never appeared in CPU profiles.

## 2. What the code establishes (no measurement needed)

- Defaults: REALITY on TCP/443 with `xtls-rprx-vision`, Hysteria2 on UDP/443
  with Salamander after `init`, BBR for Hysteria2 (on `main` a client that
  declared its own bandwidth got an uncapped fixed-rate sender; this branch
  tells every client to use BBR unless the operator sets caps),
  `log.level = fatal`, C-16 rules on every exit.
- Kernel tuning (`deploy/lib/perf-tuning.sh`): `rmem_max`/`wmem_max` 16 MiB,
  BBR when available, `default_qdisc` fq/fq_codel when available, baseline
  capture + verified rollback. `default_qdisc` changes only qdiscs created
  later; the live NIC keeps its qdisc until reboot or link reset. BBR does not
  need `fq` (TCP-internal pacing since Linux 4.13).
- sing-box unit: `Nice=-5`, `LimitNOFILE=65535`, `TasksMax=512`,
  `MemoryHigh=75%`, `MemoryMax=90%`, no CPU quota.
- Every authorization change re-renders and restarts sing-box (no hot user
  reload in sing-box 1.14); an unchanged render is a no-op.
- Hiddify discards the subscription's selector, so the "REALITY default" only
  applies to clients that honour a `selector` (sing-box apps, Tamara).

## 3. Latency (class C)

Condition: lab topology in `deploy/lab/perf/`, client↔exit RTT 20/80/160 ms
(exit↔destination 2 ms), random loss 0/1/3 %, 100 Mbit/s `htb` access link
with a 300-packet FIFO. Probes: [`labtool`](../../../deploy/lab/perf/labtool/)
through each client's SOCKS inbound. Medians of 30–100 samples; raw data in
[`results-matrix-1.14.1*.jsonl`](../evidence/2026-10-14/).

### 3.1 Idle and new-connection latency (0 % loss)

| RTT | Path | Idle RTT on an open connection p50 | New proxied TCP connection p50 / p95 | New UDP session p50 |
|---:|---|---:|---:|---:|
| 20 | REALITY | 25.1 | 82.4 / 86.8 | 79.9 |
| 20 | Hysteria2 | 25.4 | 31.1 / 34.7 | 27.4 |
| 80 | REALITY | 85.1 | 262.4 / 272.5 | 262.6 |
| 80 | Hysteria2 | 85.7 | 90.2 / 92.5 | 86.9 |
| 80 | two-hop (REALITY via relay) | 86.9 | 314.0 / 335.5 | fails (TCP-only route) |
| 160 | REALITY | 165.4 | 503.5 / 527.5 | 499.0 |
| 160 | Hysteria2 | 165.5 | 169.7 / 177.3 | 167.6 |
| 160 | two-hop | 166.8 | 632.6 / 635.8 | fails |

All values in ms. Direct ICMP to the destination: 23.2 / 85.9 / 163.7 ms.

- **Idle RTT**: both transports add about 1–3 ms over the path. The tunnel
  itself is not a latency problem on an established connection.
- **New connections are the real difference**: REALITY costs about **3 RTT**
  per new proxied TCP connection and per new UDP session (TCP handshake,
  REALITY/TLS handshake, then the request), plus the exit's decoy dial.
  Hysteria2 costs about **1 RTT** (a new stream on a QUIC connection that is
  already up). At a 160 ms mobile RTT that is ~0.5 s versus ~0.17 s for
  every new connection — this is what a user feels as "slow to start" on
  REALITY, not throughput.
- Two-hop adds about one more RTT (outer handshake to the relay, then the
  inner REALITY handshake to the exit). At 20 ms the two-hop path is not
  comparable (its relay→exit leg alone is 40 ms in the lab).
- Packet timing of one new connection per transport at 80 ms:
  [`tcpdump-reality-cold.txt`, `tcpdump-hy2-cold.txt`](../evidence/2026-10-14/).

### 3.2 Latency under load (a download saturating the 100 Mbit/s link)

| RTT / loss | REALITY: ping on a separate TCP connection, p50 / p95 | Hysteria2: ping on the **same** QUIC connection as the download, p50 / p95 |
|---|---:|---:|
| 20 ms / 0 % | 58 / 112 | 533 / 549 |
| 80 ms / 0 % | 114 / 170 | 385 / 539 |
| 160 ms / 0 % | 196 / 352 | 310 / 811 |
| 80 ms / 1 % | 114 / 177 | 358 / 567 |
| 80 ms / 3 % | 99 / 169 | 533 / 625 |

Diagnosis at 20 ms ([`loaded-latency-20ms.jsonl`](../evidence/2026-10-14/loaded-latency-20ms.jsonl),
[`bbr-profile-20ms.jsonl`](../evidence/2026-10-14/bbr-profile-20ms.jsonl)):

| Case | Ping p50 / p95 ms | Access-link queue, median packets |
|---|---:|---:|
| Hysteria2 idle (reference) | 26 / 32 | 0 |
| Hysteria2, ping and download in the same QUIC connection | 529 / 547 | 160 |
| Hysteria2, ping in a **second** QUIC connection (own client process) | 47 / 63 | 148 |
| REALITY, separate TCP connections | 59 / 106 | 197 |
| download on REALITY, ping on Hysteria2 | 60 / 120 | 188 |
| same-connection, server `bbr_profile: conservative` | 530–535 / 552 | 33–46 |

So the extra ~0.5 s is **inside one Hysteria2 connection**, not in the network
queue (a second connection through the same full queue sees +21 ms; the
conservative BBR profile shrinks the queue fourfold and leaves the 0.5 s
unchanged). Follow-up measurements ([results §2.4](VPN_OPTIMIZATION_RESULTS.md))
put it in the **client**: the request leaves the app and reaches the wire
~500 ms later, the network and the exit add ~11 ms, the client's UDP queue
stays empty, and a lab client whose sender uses a fixed-rate controller
instead of sing-quic's BBR brings the ping down to 45 ms with the exit
unchanged. Every app on a phone shares that one QUIC connection, so while one
app saturates the link through Hysteria2 (a video buffering burst, an update),
other apps' requests through Hysteria2 wait about half a second; through
REALITY they wait about the queue delay (~35 ms here). Whether Hiddify's
core behaves the same was not tested.

## 4. Throughput

### 4.1 One flow through a shaped access link (class C)

Single download, 100 Mbit/s access link; medians of 3 runs (all runs in brackets).

| RTT / loss | No tunnel | REALITY | Hysteria2 | Two-hop |
|---|---:|---:|---:|---:|
| 20 ms / 0 % | 92.3 | 88.0 | 84.4 | 64.5 * |
| 80 ms / 0 % | 88.8 | 84.9 | 76.2 | 78.8 |
| 160 ms / 0 % | 75.9 | 65.6 | 72.8 | 62.4 |
| 80 ms / 1 % | 86.4 | 80.9 | 73.3 (75.5 / 73.3 / **failed**) | 78.5 |
| 160 ms / 1 % | 56.3 | 52.0 | 66.8 (66.8 / 67.5 / **failed**) | 57.0 |
| 80 ms / 3 % | 75.1 | 69.7 | 73.5 (74.9 / **failed** / 73.5) | 74.9 |

\* two-hop at "20 ms" has a 42 ms path (see 3.1) and shared a CPU with the
delay emulator. Mbit/s of iperf3 goodput. A failed run counts as 0 in the
median.

- On a 100 Mbit/s link both transports deliver at least 85 % of the
  no-tunnel rate. Hysteria2 (BBR, loss-tolerant) keeps more of it at high RTT
  with loss (160 ms / 1 %: 66.8 vs 52.0 Mbit/s for REALITY's TCP, and above
  the 56.3 of plain TCP); REALITY is slightly ahead on clean paths.
- **3 of 9 Hysteria2 runs under loss failed** ("control socket has closed
  unexpectedly": iperf3's idle control connection, proxied as a Hysteria2
  stream, was closed). 0 of 9 REALITY runs failed. A targeted repro (10 runs
  per transport at 80 ms / 3 %) did not reproduce it
  ([results §2.7](VPN_OPTIMIZATION_RESULTS.md)); treat it as open.

### 4.2 CPU-bound throughput (class B, exit pinned to one CPU)

No added delay, kernel forwarding, iperf3 single flow, 3 runs each;
[`results-classB-1.14.1.jsonl`](../evidence/2026-10-14/results-classB-1.14.1.jsonl).

| Test (sing-box 1.14.1) | Mbit/s, median (runs) | exit CPU % |
|---|---:|---:|
| REALITY download | 2371 (2367 / 2756 / 2371) | 82 |
| REALITY upload | 2236 (2473 / 2194 / 2236) | 94 |
| REALITY carrying plain HTTP (curl) | 2770 (2770 / 2766 / 2925) | 82 |
| REALITY carrying HTTPS (Vision skips the outer encryption) | 5303 (5303 / 5778 / 5098) | 85 |
| Hysteria2 download (Salamander, as deployed) | 512 (512 / 481 / 515) | 79 |
| Hysteria2 upload | 572 (572 / 501 / 590) | 77 |
| Hysteria2 download **without** Salamander (attribution only) | 956 (941 / 1036 / 956) | 71 |
| REALITY download, exit on 2 CPUs | 3108 (3066 / 3108 / 3379) | 93 |
| Hysteria2 download, exit on 2 CPUs | 632 (590 / 632 / 637) | 120 |

- Per Mbit/s, Hysteria2 with Salamander costs about **5×** the exit CPU of
  REALITY (0.16 vs 0.03 % CPU per Mbit/s); about half of that comes from
  Salamander (0.07 % without it). On this 2.8 GHz core one Hysteria2 flow
  tops out near 0.5 Gbit/s; that is far above a phone's link but matters for
  aggregate capacity on a 1 vCPU VPS (§5).
- A second CPU barely helps a single flow (REALITY +30 %, Hysteria2 +23 %):
  one connection is handled by one or two goroutines.
- Most real traffic is HTTPS, which REALITY+Vision forwards with one TLS
  layer, about **2×** the plain-HTTP rate.
- The Hysteria2 server socket keeps the default 208 KiB receive buffer with
  Salamander (`rb212992`) and gets 16 MiB without it; `Udp: RcvbufErrors`
  did not grow in these runs.
- CPU profiles ([`pprof-*-download-1.14.1.txt`](../evidence/2026-10-14/)):
  REALITY — 69 % in read/write syscalls, 21 % AES-GCM, route matching not
  visible. Hysteria2 — 42 % in per-packet `sendmsg`/`recvmsg` syscalls, 10 %
  Salamander XOR, quic-go packet assembly ~15 %, AES-GCM 3 %. The cost is
  per packet (no GSO/batching behind Salamander's wrapper), which explains
  the 5× gap better than cryptography does.

### 4.3 sing-box 1.14.1 vs 1.14.3 (class B, same tests)

| Test | 1.14.1 median (runs) | 1.14.3 median (runs) |
|---|---:|---:|
| REALITY download | 2371 (2367 / 2756 / 2371) | 2734 (2881 / 2734 / 2459) |
| REALITY upload | 2236 (2473 / 2194 / 2236) | 2386 (2364 / 2521 / 2386) |
| Hysteria2 download | 512 (512 / 481 / 515) | 546 (573 / 546 / 517) |
| Hysteria2 upload | 572 (572 / 501 / 590) | 555 (588 / 555 / 537) |
| REALITY carrying HTTPS | 5303 (5303 / 5778 / 5098) | 4601 (4862 / 4601 / 3960) |

Differences are inside run-to-run spread except REALITY-carrying-HTTPS,
where the ranges did not overlap. An interleaved A/B re-run (4 rounds each)
gave 1.14.1 4293–5357 and 1.14.3 4479–5480 Mbit/s for HTTPS, and 2372–2616
vs 2155–2694 for plain HTTP: **no measurable difference**
([results §2.5](VPN_OPTIMIZATION_RESULTS.md)).

## 5. Capacity, reliability and recovery

### 5.1 Many users at once

[`results-concurrency-classB.jsonl`](../evidence/2026-10-14/results-concurrency-classB.jsonl)
(B: no added delay, exit on one 2.8 GHz CPU) and
[`results-concurrency-20ms-100m.jsonl`](../evidence/2026-10-14/results-concurrency-20ms-100m.jsonl)
(C: 20 ms RTT, one shared 100 Mbit/s link). REALITY "users" are parallel
connections of one client process; Hysteria2 users are separate client
processes, i.e. separate QUIC connections, as separate phones would be.

| Users | REALITY B: total Mbit/s · exit CPU % · Jain | Hysteria2 B: total · CPU % · Jain | REALITY C: total · Jain | Hysteria2 C: total · Jain |
|---:|---|---|---|---|
| 1 | 2522 · 84 · 1.00 | 554 · 80 · 1.00 | 88.7 · 1.00 | 83.9 · 1.00 |
| 5 | 2839 · 73 · 1.00 | 589 · 96 · 1.00 | 83.5 · 0.99 | 91.1 · 1.00 |
| 10 | 2926 · 74 · 0.99 | 552 · 97 · 1.00 | 80.7 · 1.00 | 90.7 · 0.99 |
| 25 | 3002 · 92 · 0.96 | 521 · 95 · 1.00 | 75.5 · 0.95 | 91.1 · **0.76** |
| 50 | 3667 · 91 · 0.99 | 525 · 94 · 1.00 | not run | not run |

- No download failed at any level, on either transport.
- One core carries about **0.55 Gbit/s of Hysteria2 in total**, whatever the
  number of users; REALITY reaches 2.5–3.7 Gbit/s on the same core. On a
  shaped 100 Mbit/s link, Hysteria2 already uses 39–48 % of the exit core
  (REALITY 7–8 %): pacing many slower flows costs more per byte than one fast
  flow. A 1 vCPU VPS of this class should therefore be sized at roughly
  0.2–0.5 Gbit/s of simultaneous Hysteria2 traffic; measure on the real host.
- Sharing is fair (Jain ≥ 0.94) except 25 Hysteria2 users on one 100 Mbit/s
  link (0.76: 1.5–8.8 Mbit/s per user), a known property of many BBR flows in
  one queue.
- Exit memory: RSS 56 → 75 MB with 50 REALITY connections, up to 96 MB with 50
  Hysteria2 users (~0.5 MB per QUIC connection); peak open files 112. The unit
  limits (`MemoryHigh=75%`, `LimitNOFILE=65535`) are far away at this scale.
- Latency on a separate idle connection during these loads (C, p50/p95):
  REALITY 58/133 ms (1 user) to 137/454 ms (25), Hysteria2 44/59 to 62/476 ms.
  A run at a nominal 1 Gbit/s was limited by the lab's delay emulator
  (~150 Mbit/s) and is excluded. 100 users were not run: one client namespace
  cannot stand in for that many phones meaningfully, and the product targets
  a small group. Control-plane responsiveness under load was not measured.

### 5.2 What a user sees when the exit restarts

Every user change restarts sing-box (§2). Measured from already-running
clients that open a new proxied request every 250 ms
([`restart-recovery-*.jsonl`](../evidence/2026-10-14/), 3–4 rounds each, all
rounds within ±0.1 s of each other):

| Condition (80 ms RTT unless noted) | REALITY: first success after restart / failed requests | Hysteria2 |
|---|---:|---:|
| 0 ms, client sending during the restart | 1.05 s / 0 | 1.05 s / 0 |
| client sending during the restart, ICMP delivered | 1.3 s / 0 | 1.3 s / 0 |
| client sending during the restart, ICMP dropped on the path | 1.3 s / 0 | **30.5 s / 9** |
| client idle during the restart (first request 3 s after it), ICMP delivered | 4.3 s / 0 | **34.6 s / 10** |
| same, Hysteria2 without Salamander (attribution only) | 4.3 s / 0 | **34.6 s / 10** |
| same, exit built with the upstream-fix prototype (close QUIC connections on shutdown) | 4.3 s / 0 | **4.3 s / 0** |
| prototype, client sending, ICMP dropped | 1.3 s / 0 | **1.3 s / 0** |
| official **1.14.3 exit**, idle 1.14.1 clients | 4.3 s / 0 | **4.3 s / 0** |

A Hysteria2 client is not told that the server went away: the stopping
process sends no QUIC `CONNECTION_CLOSE`, and the new process cannot send a
valid stateless reset (its key is random per process; with Salamander the
reset is also disabled). The client keeps sending on a connection the new
server does not know until its idle timeout (fixed at 30 s on the client)
expires, counted from its first unanswered packet. Only a client whose
packet hit the stopped port *and* got the ICMP error back recovers quickly.
A phone is usually idle, so the 30 s row is the common case. REALITY's TCP
connections are reset and simply re-dialled. The lab prototype
([`upstream-proposal/`](../evidence/2026-10-14/upstream-proposal/), never
deployed) makes Hysteria2 recover exactly like REALITY without any client
change — and **sing-box 1.14.3 already behaves that way on the server side**
(its lifecycle scope closes the QUIC connections before the socket), so
moving the exit pin to 1.14.3 removes the stall for existing clients.

### 5.3 Long-running stability (lab soak)

90 minutes, class C, 20 ms RTT, both transports under mixed load
(details: [results §3.1](VPN_OPTIMIZATION_RESULTS.md)): 0 failures across
4 long-lived connections (~20,700 pings), 17,340 new connections and 180
bulk downloads; exit RSS 58 → 65.5 MB, level after ~30 min; fds flat at 16.
The Hysteria2 long-lived ping has a p99 of ~680 ms because the periodic
download shares its QUIC connection (§3.2); REALITY's p99 is 66–76 ms.
90 minutes rules out a fast leak only.

### 5.4 Network gaps (lab outage)

80 ms RTT, client↔exit traffic dropped for 5/20/40 s (results §3.2): new
connections work within 0.4 s after every gap on both transports. Open
REALITY connections survive but resume ~17–19 s (20 s gap) and ~35 s (40 s
gap) after the network returns, because of TCP retransmission backoff. Open
Hysteria2 connections resume at once after gaps up to 20 s and are closed
after 40 s (30 s idle timeout). Lab Linux TCP on both ends, not an iPhone.

## 6. Identified bottlenecks, in the order a user meets them

| # | Bottleneck | Where | Evidence | Consequence |
|---|---|---|---|---|
| 1 | Access network RTT × REALITY's 3-RTT setup | client ↔ exit | §3.1 | ~0.5 s per new connection at 160 ms; Hysteria2 ~0.17 s |
| 2 | Hysteria2 restart stall on the 1.14.1 exit | exit shutdown path | §5.2 | ~30 s of failing Hysteria2 requests after every user change; gone with a 1.14.3 exit |
| 3 | Hysteria2 same-connection queueing in the client (sing-quic BBR) | client sender | §3.2 | ~0.5 s extra per request while a download saturates the link |
| 4 | Exit CPU for Hysteria2 (per-packet syscalls, no GSO behind Salamander) | exit | §4.2, §5.1 | ~0.55 Gbit/s total per 2.8 GHz core; 40–48 % of a core at 100 Mbit/s of paced flows |
| 5 | Exit CPU for REALITY (syscalls, AES-GCM) | exit | §4.2 | several Gbit/s per core: not a practical limit here |
| — | Not bottlenecks in the lab: route evaluation and C-16 rules, socket buffers (no drops), BBR profile, kernel qdisc | | profiles, counters | |

## 7. Historical numbers (not reproduced as such)

| Source | Claim | Class | Status here |
|---|---|---|---|
| 2026-09-27 audit | single flow, 4 vCPU lab: no tunnel 13,018 / REALITY 6,039 / Hysteria2 809 / two-hop 5,906 Mbit/s | B | Same shape reproduced (REALITY ≈ 6× Hysteria2 per server core); absolute numbers differ with CPU, pinning and iperf3 placement |
| 2026-09-27 audit | new connection p50: REALITY 3.7 ms, Hysteria2 1.4 ms, two-hop 6.7 ms (300 sequential, no delay) | B | Methodology note: Hysteria2's figure is a new **stream** on a warm QUIC connection, REALITY's a full new connection; at real RTTs the gap is ≈3 RTT vs ≈1 RTT (§3) |
| v1.1.0 acceptance (2026-09-27, Hetzner) | Hysteria2 transfer 72 Mbit/s, one run | D (single sample, CDN-dependent) | not re-run |
| `YOUTUBE_FIX_INVARIANTS.md` INV-8 (2026-10-11) | new Google QUIC session ~130 ms direct vs ~320 ms via hairpin | D (owner's deployment) | mechanism confirmed in the lab (each UDP session over VLESS = new TCP+REALITY handshake to the paired node); the absolute values are not re-measured |

## 8. Unknowns and limits

- No VPS, ISP path, censor, mobile radio or iPhone was available: CPU steal,
  provider caps, real loss patterns, DPI behaviour, Wi-Fi↔cellular handover and
  iOS suspend/resume are not covered.
- No IPv6 in the lab kernel: IPv6 paths, the exit IPv6-literal policy and
  dual-stack happy-eyeballs at the exit were not exercised.
- The lab's loss is random (Bernoulli) and its queue is a plain tail-drop FIFO;
  real mobile networks have bursty loss and deeper, adaptive buffers.
- Lab decoy and DNS are 1 ms away; real ones add their own RTT to every new
  REALITY connection and every uncached lookup.
