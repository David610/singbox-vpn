# EU-only pilot evidence (2026-10-26, rc.8, de1 -> fin1)

Secret-free. Classes: **AUTO** = verified automatically from the authorized workstation/hosts; **DEVICE** = verified on a
real phone/PC (none yet); **UNVERIFIED**; **FAILED**. Browser tests are headless Chrome with iPhone emulation, signed
out, TCP only (`--disable-quic`): browser evidence, not native-app acceptance.

## Verdict
- **EU_STAGING_PASS**: yes.
- **EU_PILOT_READY**: not yet - waits for one real Hiddify connection (no critical defect known).
- **EU_FRIENDS_READY**: not claimed - needs Russian-network acceptance.

## Configuration confirmed (read-only)
rc.8 on both hosts; services, timers and `ufw` healthy (only 22, 443, 8443); `doctor` 0 FAIL on both; relay paired;
hairpin on; spoof capabilities present on the relay; reconcile timer every 10 minutes.

## Pilot account
`pilot-friend-01`, 14-day expiry, own relay credential + own exit credential (`peer-pilot01`, matched by hash). Its
pinned link returns a profile with ONE selectable server. Other URL forms: `shadowrocket`/`vision_off` -> HTTP 400
(refused); `quic_reject` and `youtube_direct` stay on the relay chain (analysis passes) - `youtube_direct` additionally
sends YouTube from the client's own connection and is never given to friends. Verified through the pilot's own link:
ordinary egress Finland (IPv4 and IPv6), HTTPS/h2, YouTube/Google answer.

## Results
| Check | Class | Evidence |
|---|---|---|
| Entry via de1, ordinary traffic exits fin1 | AUTO | egress `62.238.46.190` / `2a01:4f9:c014:13bd::1` |
| Google/YouTube leaves de1 over IPv4 | AUTO | capture: 315 IPv4 vs 1 IPv6 Google packets on de1 |
| No direct bypass | AUTO | profile chain analysis; fail-closed tests (exit/relay/hairpin down, user/credential disabled) |
| Import + refresh coherence | AUTO | same URL; 200, `no-store`; rotation then refresh works |
| Revocation | AUTO | disable: link 404 + connection rejected immediately; re-enable restores |
| Expiry | AUTO | link 404 at expiry; connection rejected at the next reconcile (12:41 run, 8 min later) |
| Hiddify core: DNS | AUTO | unique names resolved at the exit (36 packets on fin1, 0 on the entry) |
| sing-box client DNS | AUTO | 12 packets on fin1, 0 on de1 |
| IPv6 by hostname | AUTO | v6-only name exits from fin1's IPv6 (the earlier "failure" was `curl -6` to an IPv4 proxy) |
| Shorts, sustained (real path) | AUTO (browser) | 10 distinct Shorts x 35 s: 10/10 OK, 0 stalls, 0 unavailable |
| Shorts, signed in | UNVERIFIED | needs a device with an account |
| Long-form YouTube | UNVERIFIED | the headless probe fails identically with NO proxy (buffer stops refilling ~35 s, player resets ~65 s): a harness limit, not a VPN result |
| Telegram | UNVERIFIED | needs a device |
| App restart / refresh / network handover / airplane cycle | UNVERIFIED (engine-level reconnect after rotation, reboot and restarts passed) | needs a device |
| Native Hiddify (Windows/Android/iPhone) | UNVERIFIED | owner |
| Russian mobile / home Wi-Fi reachability | UNVERIFIED | the Russian friend |

## Google egress comparison (policy unchanged; isolated tests with SSH loopback SOCKS and pinned address family)
| Egress | Shorts (35 s each, signed out) |
|---|---|
| real pilot path (Google via de1) | 10/10 played |
| de1 address directly, IPv4 | 6/6 played |
| fin1 address directly, IPv4 | **0/6**: "unavailable" every time |
| fin1 / de1 with default resolution (IPv6) | all "unavailable" |

Conclusion: YouTube serves Shorts to de1's IPv4 address and withholds them from fin1's. The failure on fin1 is an
egress-reputation/content-delivery refusal, not a connectivity failure (page loads, TLS and h2 succeed). Keep the
hairpin; Google reaches YouTube over de1's IPv4. HTTP 200/204 results are not evidence of Shorts playback.

## Capacity
~60 Mbit/s aggregate over 4 streams through the chain; de1 (1 vCPU) ~10 % CPU, fin1 ~2 %. The workstation link was the limit.

## Defects found in this session
None in the product. Test-tool issues only (curl `-6` through a proxy; a loop-naive playback metric; a long-form
headless limitation; my own staging path permissions), all explained above.
