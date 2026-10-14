# VPN optimization roadmap — 2026-10-14

Ranked by evidence-backed value for the real product: one self-managed VPS, a
small group of users, iPhone + Hiddify as the main client, YouTube/Shorts
working through the protected hairpin/IPv6 design
([invariants](../../YOUTUBE_FIX_INVARIANTS.md)). Numbers come from
[VPN_PERFORMANCE_BASELINE.md](VPN_PERFORMANCE_BASELINE.md) and
[VPN_OPTIMIZATION_RESULTS.md](VPN_OPTIMIZATION_RESULTS.md) (lab classes B/C
only) unless marked otherwise.

Status vocabulary: **Implemented + tested** (in this branch) · **Already
implemented** · **No change needed** · **Confirmed issue** · **Hypothesis —
needs measurement** · **Needs real VPS** · **Needs native device** ·
**Upstream** (fix belongs in sing-box/sing-quic; lab prototype or attribution
exists) · **Rejected**.

## Decision table

| Pri | Component | Problem | Evidence | Proposed change | Expected benefit | Risk | Status |
|---|---|---|---|---|---|---|---|
| R1 | `vpn-benchmark` | Default run created/removed a real user per transport: 4 sing-box restarts, every user cut; a killed run left a credential with no expiry | code; new safety test fails on the old script; lab run of the new default | Default protocol layer = disposable loopback instance; live-listener test only with `--production-listener`, expiring user, cleanup on INT/TERM/HUP; tunnel `curl` ignores `NO_PROXY` | Measuring no longer disturbs production | Low | **Implemented + tested** |
| R2 | Hysteria2 opt-in bandwidth (`[hysteria2] up_mbps/down_mbps`) | Rendering added `ignore_client_bandwidth: true`, which makes the server refuse every client that does not declare bandwidth — every client this product provisions | lab, real 1.14.1: "BBR disabled by server", HTTP 404 auth; with the fix both client kinds connect | Render the caps only; runtime interop test | The opt-in no longer locks out users | Low (opted-in hosts only) | **Implemented + tested** |
| R2b | Hysteria2 fairness (no caps, the default) | Any authenticated user could declare a bandwidth in their client and get an **uncapped fixed-rate (Brutal) sender**, starving everyone else on the node | lab C, 20 ms / shared 100 Mbit/s: greedy client declaring 500 took 88.5–88.8 Mbit/s, a REALITY user 0.1–16, a default Hysteria2 user 0.7–2.2; with the fix 27–30 vs 34–37 and 28–30 | Without caps, render `ignore_client_bandwidth: true` (server tells every client to use BBR); with caps unchanged (Brutal capped at the caps) | One user cannot take the whole node | Low: clients that declare nothing follow the identical server path; a user who hand-declared an accurate rate loses Brutal's edge on lossy links (lab, 80 ms/1 %: 87 vs 73 Mbit/s) | **Implemented + tested** (unit + real-binary interop on 1.14.1 and 1.14.3) |
| R3 | Hysteria2 restart stall | After every sing-box restart (every user change), an idle Hysteria2 client stalls **~30 s** from its first request; REALITY ~1 s. Not caused by Salamander | lab C: 34.6 s / 10 failed requests (idle client, ICMP delivered), same without Salamander; 1.3 s when the client happened to send while the port was closed and got the ICMP error | (a) Move the exit to sing-box **1.14.3**, which already closes QUIC connections on shutdown (lab: 34.6 → 4.3 s with unchanged 1.14.1 clients) — see R8; (b) until then: batch user changes, document; (c) not needed after (a): opt-in `idle_timeout: "20s"` (measured 34.6 → 24.6 s) | Hysteria2 users stop losing 30 s on every change | as R8 | **Fixed upstream in 1.14.3** (lab verified, server side); docs + operator message corrected for the 1.14.1 pin; ships with R8 |
| R4 | Hysteria2 same-connection latency | While one app saturates the link through Hysteria2, every other request on the same QUIC connection waits **~0.4–0.5 s** (REALITY: +~35 ms) | lab C, localized: the request sits in the **client** for ~500 ms before reaching the wire; client queue/drops ruled out; a lab client whose sender uses a fixed-rate controller: 523 → 45 ms with the exit unchanged; unchanged with 1.14.3 on both ends (524 ms) | **Upstream** (sing-quic client-side BBR with an app-limited sender); here: document; prefer REALITY for latency-sensitive use while downloading | Responsive apps during video buffering / downloads | n/a | **Upstream**; **Needs native device** (Hiddify core may differ) |
| R5 | Hiddify default import | Default `?format=hiddify` link (REALITY + Hysteria2) becomes a per-connection round-robin `balance` group in Hiddify; the "REALITY default" never applies; combined with R3/R4 a user gets Hysteria2's stalls on about half of new connections | code-verified analysis in repo (`YOUTUBE_FINAL_ROOT_CAUSE.md` §13); lab R3/R4 | Make the printed default link single-route (`compat=hiddify-pinned`, REALITY) or tell users to pick a server; keep the multi-route link for clients that honour `selector` | One predictable transport per device | Medium: changes what new users import | **Needs native device** |
| R6 | C-16 own-address rule | Inactive on domain installs (`public_ipv4`/`public_ipv6` not recorded): a tunnel user reaches services on the node's own public address | code; lab: 200 without the field, refused with it | `vpn doctor` now warns (this branch). Operator: add the fields + `vpn render-config`. Installer: record the addresses preflight already verified | Tunnel users cannot reach node services on its public IPs | Low | **Implemented + tested** (warning); installer change **proposed** |
| R7 | Provisioning agent (fleet) | `lease-pool sync` / `external-authorizations` `vpn-admin` children survived their timeout and could apply later (SVPN-F04 residual) | code | Route through `run_vpn_admin` (kill process group + reap); source-scan test | No stale applies, no restart pile-up | Low | **Implemented + tested** |
| R8 | sing-box pin 1.14.1 → 1.14.3 | The pin lacks 66 upstream commits, including the lifecycle change that **removes the Hysteria2 restart stall (R3)**, a TCP accept-loop CPU spin fix, half-close through REALITY wrappers, DNS init fixes and sniffing/parser input validation | upstream diff + source; lab: restart stall gone with a 1.14.3 exit (4.3 s vs 34.6 s, clients unchanged), same throughput (interleaved A/B), all real-binary interop and two-hop suites pass with 1.14.3 except the IPv6-only cases the lab cannot run | Bump `versions.env` (both hashes, second independent download) through the release process | Every user change stops costing Hysteria2 users ~30 s; robustness | Medium: data-plane binary change, needs the exact-candidate VPS lifecycle and the native YouTube/Shorts checklist (INV-12) | **Needs real VPS + native device** — highest-value next step |
| R9 | Hysteria2 CPU cost / capacity | Exit CPU per Mbit/s ≈ 5× REALITY with Salamander; one 2.8 GHz core carries ~0.55 Gbit/s of Hysteria2 in total (flat from 5 to 50 users), ≥3 Gbit/s of REALITY; at 100 Mbit/s paced flows Hysteria2 already uses ~40–48 % of a core | lab B/C + pprof (per-packet syscalls, no GSO behind Salamander) | Capacity planning: size by Hysteria2 load; keep Salamander; watch `vpn doctor --performance` | Avoid CPU saturation on 1 vCPU | n/a | **Confirmed (lab)**; **Needs real VPS** for absolute numbers |
| R10 | Hysteria2 server UDP buffer | Server socket keeps `rmem_default` (208 KiB) with Salamander; 16 MiB `rmem_max` never used by it; no `RcvbufErrors` in any class-B run | lab | Watch `Udp: RcvbufErrors`; only if it grows under real load: opt-in `rmem_default` experiment or upstream fix | Fewer drops under bursts | Global memory/latency effect | **Needs real VPS** |
| R11 | REALITY new-connection cost | ≈3 RTT per new proxied TCP connection and per new UDP session vs ≈1 RTT for a new Hysteria2 stream (~0.5 s vs ~0.17 s at 160 ms) | lab C + packet trace | No safe server-only fix: multiplexing needs Vision off (TLS-in-TLS visible); TFO is a fingerprint. Keep | — | High if changed | **Rejected** as a default; documented |
| R12 | Hairpin QUIC setup | Each new Google UDP session opens a new REALITY connection to the paired node (+≈2–3 exit↔pair RTT; historical ~130 → ~320 ms) | historical (INV-8) + lab mechanism (R11) | Optional multiplexed hairpin outbound | Faster Shorts start | High: protected path (INV-1..INV-12), DPI exposure on the hop | **Rejected without owner approval + native acceptance** |
| R13 | Kernel tuning (`perf-tuning.sh`) | `default_qdisc` only affects new qdiscs; 16 MiB ceilings do not reach the Hysteria2 server socket with Salamander | code + lab | Documented (plan doc corrected) | Correct expectations | — | **No change needed** |
| R14 | Two-hop | TCP-only (UDP/QUIC fails closed); +1 RTT per new connection; Direct and via routes share the exit credential (SVPN-F10) | lab + code | Separate exit credential for the via route, accepted only from declared relays | Real per-user unlinkability | Medium (design) | **Confirmed issue** |
| R15 | Restart on every authorization change (SVPN-F11) | sing-box cannot change users live; each change cuts every connection (and costs Hysteria2 users R3's stall) | code + lab | Batch non-urgent changes; keep revocation immediate (lab: a disabled user's new connections failed 39/39 over 40 s on both transports) | Fewer disconnects | Medium | **Confirmed issue** (unchanged by design) |
| R16 | conntrack / fd limits | Untuned; lab peak 112 fds with 50 REALITY downloads, exit RSS up to ~96 MB with 50 Hysteria2 users | lab B | Measure `conntrack -C` and fds on the real host before changing | — | — | **Needs real VPS** |
| R17 | Throughput-aware automatic transport selection | `urltest` probes HTTPS reachability only; Hiddify discards it anyway | code | Only after R5 and real per-network measurements: EWMA of setup time + failures with hysteresis and cooldown, never mid-session switching | — | High (oscillation, DPI behaviour) | **Hypothesis — needs measurement** |
| R18 | Hysteria2 stream drops under loss | 3 of 9 lossy matrix runs lost iperf3's idle control connection through Hysteria2 (0/9 REALITY) | lab C; targeted repro at 80 ms / 3 %: 0/10 Hysteria2, 0/10 REALITY | None until it reproduces; keep the probe in the lab | — | — | **Hypothesis — needs measurement** (not reproduced) |

## Rejected optimizations (and why)

| Idea | Why rejected |
|---|---|
| Remove Salamander to "double Hysteria2 speed" | Lab: ~+90 % single-flow throughput and lower CPU without it, but it removes the obfuscation that keeps Hysteria2 from looking like plain QUIC to DPI. Not a trade this product should make for benchmark numbers. It is also not the cause of the restart stall (R3). |
| Brutal fixed bandwidth as a default (client declares a rate) | It would also hide R4 (fixed-rate sender), but a declared rate above real capacity causes loss and queueing (lab, declared 200 on a 100 Mbit/s link: 78.6 Mbit/s, p95 RTT 379 ms) and mobile capacity changes all the time. A client declaring only `up_mbps` does not help: sing-quic then runs BBR on both sides. |
| Raise UDP/TCP buffers further | No evidence of a buffer limit: the client UDP receive queue stayed under 5 KB and `RcvbufErrors` at 0 during the worst latency case. |
| TCP Fast Open / MPTCP / `udp_fragment` | Fingerprint risk, no client support, QUIC does its own PMTU. |
| Multiplexing as a default | Needs Vision off (TLS-in-TLS visible), concentrates head-of-line blocking. |
| Disable IPv6 globally / Cloudflare WARP / extra egress services | Forbidden by INV-2/INV-7 and owner decisions. |
| Third or fourth hop | No use case; each hop adds RTT, cost and a handshake. |
| Custom sing-box build / Rust rewrite | Profiles show time in kernel syscalls and AES-GCM, not in overhead a rewrite removes; the two upstream issues found (R3, R4) are small patches for upstream, not a reason to fork. A fork adds supply-chain and maintenance cost. |
| Bigger VPS by default | Only if the real host shows sustained sing-box CPU near one core or high steal (R9, `vpn doctor --performance`). |

## Next experiments, in order

1. Real VPS, no users disturbed: `vpn doctor --performance`, the new default
   `vpn-benchmark`, `nstat Udp` counters, sing-box CPU over a day (R9, R10, R16).
2. iPhone + Hiddify, one session each on REALITY-only and Hysteria2-only
   profiles: time from "user change on the server" to the next page load
   working (R3), and a web page load while a large download runs (R4). Then
   the default `balance` profile.
3. Prepare the 1.14.3 candidate (R8, which also fixes R3) and run the
   exact-candidate lifecycle + native checklist, including "user change
   while the phone is idle on Hysteria2".
4. File the upstream issue for R4 (client BBR holding small requests) with
   the lab evidence; R3 needs no issue (fixed in 1.14.3).
5. Record `public_ipv4`/`public_ipv6` on the production host (R6).
