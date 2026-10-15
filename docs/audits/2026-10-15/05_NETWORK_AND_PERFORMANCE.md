# 05 - Network and performance

Vantage point for every client-side number: one Windows 11 PC on a home line
(~10 Mbit/s downstream), Germany -> VPS 91.244.71.165. Server-side numbers were taken
on the VPS itself. Nothing here is native-device evidence.

## Connectivity ladder (REALITY, sing-box 1.14.1 client)

| Step | v1.3.0 | v1.3.1-rc.1 (after upgrade) | after reboot |
|---|---|---|---|
| config valid (`sing-box check`) | PASS | PASS | PASS |
| handshake + 204 on `generate_204` | PASS | PASS | PASS (first success 0.73 s) |
| egress IPv4 = VPS | 91.244.71.165 | 91.244.71.165 | 91.244.71.165 |
| HTTPS to Wikipedia | 200, TTFB 0.34 s | 200, TTFB 0.28 s | 200, TTFB 0.17 s |

Hysteria2/UDP: **BLOCKED from the test PC** (see `02`, R7). Verified over the VPS loopback:
HTTP 204, correct egress, with and without a declared client bandwidth on the default server.

## Hiddify's own core against the live candidate (CLI, Windows)

`hiddify-core dbf3ffb` (current main). `parse` = its config parser plus `libbox.CheckConfig`;
`run` = its fork of sing-box started on the same body, traffic pushed through it.

| Body | parse | run: first 204 | egress | Wikipedia |
|---|---|---|---|---|
| plain share links (VLESS + Hysteria2) | exit 0, 2 outbounds | 1.0 s | VPS IP | 200 |
| Base64 body (`encoding=base64`) | exit 0, 2 outbounds | - | - | - |
| bare `/sub` (partial sing-box JSON) | exit 0, 5 outbounds | 0.7 s | one TLS error on `api.ipify.org`, other requests fine | 200 |
| pinned REALITY JSON | exit 0 | 0.7 s | VPS IP | 200 |
| single REALITY link | exit 0 | 0.8 s | VPS IP | 200 |
| old vpn-web Hysteria2 link (decoy SNI, no obfs) | **exit 0** (parser cannot see it is unusable) | not run | - | - |
| malformed / empty | exit 1, "unable to determine config format" | - | - | - |

Reading: at parse/validate/start/handshake level the current server output is accepted and
works in Hiddify's current core. This does **not** reproduce Issue B and says nothing
about the iOS Network Extension. See `03` for the v4.0.0 comparison.

## DNS

Resolution through the tunnel works (hostnames resolved by the exit; the REALITY test
resolved Wikipedia and one.one.one.one via the proxy). One slow request (`one.one.one.one`,
10.2 s) was seen once and not investigated. **No DNS-leak test was performed** (needs a
TUN client on a device).

## IPv6

- Server egress: an IPv6-only host (`ipv6.icanhazip.com`) and an IPv6 literal
  (`[2606:4700:4700::1111]`) are reachable through the tunnel and show the VPS's IPv6
  address `2a12:bec4:19a5:9aa::`.
- On an existing deployment the exit policy is `allow`: a dual-stack destination
  (`api64.ipify.org`) is reached over **IPv6**, so Google/YouTube-type hosts would be too.
  Fresh installs from v1.3.1-rc.1 render `ipv6_literal = "reject_google"`; an in-place
  update does not change an existing `deployment.toml`.
- With `reject_google` the probe's IPv6-literal mode is refused at the proxy
  (`ERR_SOCKS_CONNECTION_FAILED`), as designed. The docs correctly say this is "a proxy
  close, not proof of native IPv4 retry": whether an iPhone app recovers over IPv4 is
  **not tested**.
- No client-side IPv6 leak, DNS64/NAT64, IPv6-only cellular or PMTU testing was possible.

## YouTube / Shorts (browser evidence only)

Project probe (`deploy/lib/youtube_shorts_probe.py`, Chrome with iPhone emulation,
TCP only, `--disable-quic`):

| Path | Result |
|---|---|
| direct from the test PC (control) | 6/6 playing |
| via REALITY, default family | 0/6 ("video unavailable") |
| via REALITY, IPv4 literals only | 0/6 |
| via REALITY, IPv6 literals only | refused by `reject_google` policy (expected) |

The control plays, so the probe and Chrome work. The refusal is a function of the exit's
IPv4 address (consistent with `docs/YOUTUBE_FINAL_ROOT_CAUSE.md` section 18), not of the
VLESS/REALITY transport. The documented fix is the Google-domain hairpin through a second
node whose IP YouTube accepts; its credentials are not available here and third-party
egress such as WARP is forbidden by the owner. Native-iPhone Shorts: **not tested**.

## Latency and throughput

Direct versus REALITY tunnel from the test PC (n = 20 each; `perf-reality-v1.3.1-rc1.json`):

| Metric | direct | REALITY tunnel |
|---|---|---|
| TCP connect, median / p90 | 29 / 35 ms | 2 / 2 ms (local proxy) |
| TLS handshake complete, median | 85 ms | 123 ms |
| time to first byte, median / p90 | 104 / 115 ms | 151 / 167 ms |
| 25 MB download, three runs | 8.7, 12.1, 9.4 Mbit/s | 10.5, 11.1, 13.1 Mbit/s |

The tunnel adds about **47 ms** to time-to-first-byte. Throughput is limited by the
test line, not by the server.

Server capacity (REALITY client and server both on the VPS, 25 MB downloads from Cloudflare's
speed endpoint): 1 stream 1.09 Gbit/s; 3 of 4 parallel streams 2.49 Gbit/s aggregate; host
CPU peaked at 24 % in 1 s samples. An 8-stream round was rejected by Cloudflare's rate limit
(not a server error). No upload, UDP throughput, jitter, packet loss, concurrent-session or
long-session test was run.

## Restart and recovery

| Event | Measured |
|---|---|
| `systemctl restart sing-box` while a stream is open | stream breaks 3/3; new connections work again in 1.8-3.4 s (REALITY client) |
| 7 of 10 user operations | restart sing-box (table in `02`) |
| 8 restarts in 300 s (v1.3.0) | service parked `start-limit-hit`, 3 min 35 s outage until the watchdog |
| same burst, 12 restarts in under 5 min (candidate `c1c0a7f`) | 0 failures, 0 `start-limit-hit`, service active |
| Hysteria2 client, server restart, VPS loopback | recovers in ~0.3 s |

The loopback Hysteria2 number is **not representative**: loopback returns an immediate ICMP
"port unreachable" when the server is down. The documented ~30 s stall on a real path (a
1.14.1 server sends no QUIC CONNECTION_CLOSE on shutdown; 1.14.3 does) could not be
measured because no network with working UDP/443 was available. It remains a documented,
unmeasured claim.
