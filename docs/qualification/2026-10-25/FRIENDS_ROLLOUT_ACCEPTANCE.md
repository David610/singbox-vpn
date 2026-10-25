# Friends-only rollout: acceptance procedure and evidence register (2026-10-25)

Scope: a small group of friends in Russia. One personal subscription URL per friend, imported once in Hiddify (Android/iPhone), then Connect.
No secrets (tokens, UUIDs, keys, full URLs) appear in this document. Status words: VERIFIED (observed by a test, environment stated), USER-REPORTED (owner statement, not reproduced), FAILED, BLOCKED (cannot be run with current authorisation or without a device), UNVERIFIED (no evidence).
Server tests are never proof of real-device success.

## Decision: NO-GO for the friends rollout (as of 2026-10-25)

1. FAILED: the default REALITY profile (SNI `www.cloudflare.com` to fin1:443) does not connect from the Russian datacenter vantage (ru1): 0/5 tunnels, and 0/4 for each alternative SNI tested. Hysteria2 to fin1 worked 5/5 from the same host. No friend profile prefers Hysteria2 today.
2. BLOCKED: no test on a real device in a Russian cellular or Wi-Fi network has been run. Datacenter results do not predict consumer ISPs or mobile operators.
3. RESOLVED 2026-10-25: ru1 now runs the published v1.3.1-rc.4 (items 18-19). The rc.4 Release run failed once at the gate (`singbox-validate`, test `harness_readiness_is_owned_by_the_started_process`, two_hop_system.rs:1873: "a sing-box that could not bind its inbound was reported ready"; run 40175153386 attempt 1) and passed on re-run of the failed jobs (attempt 2). Both attempts stand; the failure is an unreproduced timing race in an existing harness test and is not fixed.
4. One pilot account exists on fin1 (`friend-pilot-1`); no other friend accounts (items 20-21).

## Evidence register

| # | Item | Status | Environment / evidence |
|---|------|--------|------------------------|
| 1 | fin1 runs published rc.3 binaries | VERIFIED | executable hashes match rc.3 (vpn-admin 5b7943d5, sing-box 9334a1c1, vpn-subscription-svc b6f37e36); doctor 0 FAIL |
| 2 | ru1 runs a release | VERIFIED (was FAILED earlier the same day) | before 08:30Z ru1 ran a development build (958a05f2). Now rc.4, see item 18 |
| 3 | Subscription served over valid HTTPS | VERIFIED | Let's Encrypt certificate for the host, valid to 2027-01-22, TLS 1.2/1.3, port 8443 (home line, WSL, ru1) |
| 4 | Token hygiene | VERIFIED | nginx /sub/ has access_log off; token string absent from access and error logs; Cache-Control no-store; unknown token returns a generic 404 |
| 5 | Refresh does not regenerate credentials | VERIFIED | 3 fetches plus one made hours earlier return byte-identical bodies (same hash) for both profile formats |
| 6 | Unknown / disabled token | VERIFIED | bogus token 404 (from ru1 and home); disabled user 404 (lifecycle run on a disposable rc.3 node) |
| 7 | Credential create/rotate/disable/enable/expire/reconcile | VERIFIED | disposable rc.3 nodes (fin1 earlier, 91.x on 2026-10-24); NOT re-run on production fin1 today (user creation restarts sing-box) |
| 8 | Profile formats and balancer | VERIFIED (server output) / UNVERIFIED (Hiddify behaviour) | format=hiddify: URI list, 2 routes (Reality, Hysteria2). Bare URL: sing-box JSON with a selector defaulting to Reality plus a urltest group. format=singbox&compat=hiddify-pinned: one route (Reality) only. Whether Hiddify adds a per-connection balancer when importing the 2-route URI list is documented in docs/HIDDIFY_ANDROID.md but not reproduced on a device |
| 9 | Subscription endpoint reachable from Russia | VERIFIED (datacenter) / UNVERIFIED (consumer) | ru1 (Selectel) completes TLS to fin1:8443 and gets the expected 404 for a bogus token. No consumer ISP or mobile test |
| 10 | REALITY transport from Russia | FAILED (datacenter) | ru1 real sing-box client to fin1:443: 0/5 requests, context deadline exceeded. Plain TLS sweep toward fin1:443 from ru1: www.cloudflare.com, cloudflare.com, dash.cloudflare.com, speedtest.net 0/4 each, while one.one.one.one, example.com, discord.com complete 4/4; all names 4/4 from the home line. A real REALITY client with the alternative SNIs: 0/4 each (the server accepts only its configured name) |
| 11 | Hysteria2 transport from Russia | VERIFIED (datacenter) | ru1 real client to fin1:443/udp with salamander: 5/5 requests, exit IP = fin1, about 0.2 s |
| 12 | Google/YouTube via two-hop (fin1 to ru1 with TLS spoof) | VERIFIED (server/browser) | headless browser, signed out, through the real chain: 16/20 played, 0 bot challenges, 0 "Video unavailable" (2 runs). Not a device test |
| 13 | Shorts on a real device | USER-REPORTED | owner says Shorts now work on their device; version, network, transport, signed-in/out not recorded |
| 14 | Hiddify import, Connect, app restart restore, refresh, disabled-user behaviour on a device | BLOCKED | needs a phone |
| 15 | Russian cellular / Wi-Fi acceptance | BLOCKED | needs a friend or phone in Russia |
| 16 | DNS leak, public IP, Telegram | BLOCKED | needs a device |
| 17 | Fresh friend user end to end on production | VERIFIED (server) for the pilot | see items 20-21; device part BLOCKED |
| 18 | ru1 upgraded to published v1.3.1-rc.4 | VERIFIED | documented updater with source sha256 pinned (ba031890...); backup taken and copied off-host; executables match the rc.4 archive (vpn-admin e6d519ea, subscription fc34a6a7, sing-box 9334a1c1); doctor 0 FAIL, release identity consistent with v1.3.1-rc.4; release asset checksums and attestation verified; rollback not needed |
| 19 | Preserved on ru1 after upgrade | VERIFIED | credential fingerprints (users, REALITY keys, short id, Hysteria2 cert) identical before and after; users default and fin1-hairpin unchanged; TLS spoof in deployment.toml, systemd drop-in, active config (2 tls_spoof keys) and CapEff 0x3400 (NET_RAW+NET_ADMIN) present; doctor spoof capability check OK |
| 20 | Pilot account `friend-pilot-1` on fin1 | VERIFIED (server) | created once; existing four users' credential fingerprints identical before and after; fin1 doctor 0 FAIL; sing-box active. Subscription: 200 for all three formats, identical body on repeat fetches |
| 21 | Pilot credentials per transport | VERIFIED (server) | from fin1 loopback: REALITY 3/3, Hysteria2 3/3, YouTube TLS through the hairpin 3/3 each. From ru1 (Russian datacenter): REALITY 0/3 (and YouTube 0/3), Hysteria2 3/3 (YouTube 3/3) |
| 22 | Anonymous YouTube through the real chain on rc.4 | VERIFIED (browser) | headless browser, signed out, fin1 REALITY to hairpin to ru1 with spoof: 2 runs x 10 items: 18 played, 2 flagged FROZE (the same ~17 s clip in both runs, ended at 17.0 s with no stall: probable end of clip, not proven), 0 bot challenges, 0 refused. Earlier dev-build runs: 16/20 and 17/20. Not a device test |
| 23 | Release gate | FAILED once, then passed | Release run 40175153386 attempt 1 failed (singbox-validate, offline-full-verdict); attempt 2 passed. Documented in decision item 3 |

## Open decisions

A. Transport for friends. Evidence favours Hysteria2 from Russia in the datacenter test only (items 10, 11, 21). Not assumed for other providers. First device test (below) tests REALITY and Hysteria2 independently with the unmodified 2-route profile. No REALITY server name change, no default change.
B. Done (rc.4 on ru1).
C. Approved: ONE pilot account only (done). No other accounts until the pilot passes.

## Minimal device acceptance (per friend device, one pass per network)

Record: device model, OS version, Hiddify version, network (Wi-Fi or cellular, operator), profile format, transport selected. Never record cookies, account names or browsing history.

1. Import: paste the personal URL, the profile loads, no error. Result: pass/fail + message.
2. Connect: tap Connect, internet works within 30 s. Record which transport.
3. Public IP: open an IP-check site: expect the fin1 address for ordinary sites (YouTube/Google egress via Russia by design).
4. DNS: a DNS-leak site shows no resolver of the local operator.
5. Ordinary browsing: 3 sites load.
6. YouTube signed out (Incognito or logged out): 5 distinct Shorts, 30 s each advancing; 1 long video of 5 min including a seek.
7. YouTube signed in: the same.
8. Telegram: text, a photo, a voice call connects.
9. Interruptions: airplane mode 10 s then back; Wi-Fi to cellular switch; Connect recovers without re-import.
10. App restart: force-close Hiddify, reopen; the profile is still listed; Connect works.
11. Refresh: tap update on the profile; still works; credentials unchanged (no re-import needed).
12. Admin disables the user: refresh fails (404) and the tunnel stops after reconnect; admin re-enables: refresh works.

Any failure is recorded with the exact message and keeps the rollout NO-GO until fixed and re-run.

## First real-device test (pilot, Russian network)

Pilot account: `friend-pilot-1` (link delivered privately by the administrator; never in a ticket, chat log or screenshot).
Goal: one friend, one link, no help. Run once on Russian mobile data and once on Russian Wi-Fi, ideally with different operators. Record per run: phone model, OS, Hiddify version, network, operator.

1. Phone with no other VPN. Install Hiddify, copy the link, Add from Clipboard (docs/FRIENDS_ONBOARDING_RU.md). Record: profile loads, which entries appear, and which entry or group (single route, Auto, balancer) Hiddify selects by default.
2. Tap Connect with the default selection. Wait 30 s. Record: connected or not, exact message, which transport.
3. Test each transport independently. Select Reality, Connect, run steps 4-5; disconnect; select Hysteria2, Connect, run steps 4-5. Do not rely on Auto for this comparison.
4. Per transport: an IP-check site (expect the fin1 address), 3 ordinary sites, a DNS-leak site.
5. Per transport: YouTube signed out (Incognito): 5 distinct Shorts, 30 s each, then one 5-minute normal video with a seek. YouTube signed in: the same. Telegram: message, photo, call.
6. Reconnect: airplane mode 10 s; switch Wi-Fi/cellular; force-close and reopen Hiddify (profile still present, Connect works); tap profile refresh (still works, nothing to re-import).
7. Report to the administrator: pass/fail per line with exact messages, no link, no screenshots of the link.

Pass for the pilot means every line passes on at least one transport on both network types, with no technical intervention. A failure on one transport only is recorded as that transport's FAILED result, not hidden by the other.

## Addendum 2026-10-25 (late): Russian-entry relay link form

| # | Item | Status | Evidence |
|---|------|--------|----------|
| 24 | Relay profile forms in the Hiddify engine (hiddify-core v5.0.3, workstation, not an app) | VERIFIED (engine) / UNVERIFIED (apps) | `?format=hiddify` on a relay = direct route only, silently bypasses the Russian entry (Evolus saw the client's address); unpinned `?format=singbox` gets a Hiddify `balance` default; `?format=singbox&compat=hiddify-pinned` keeps the two-hop (Evolus saw only the relay). See `RU_ENTRY_TWO_HOP_BASELINE.md` section C |
| 25 | Relay chain in a browser | FAILED intermittently, FIELD-ONLY cause | Chromium HTTP/2 hung on `m.youtube.com` through the Vision/Vision relay chain; Vision off on the nested exit hop only fixed it (live A/B). Not reproducible in the loopback lab. Fix is opt-in `vless_flow = none` in PR #208, not in rc.6, not yet validated on hardware. **Do not give friends the relay link until validated** |
| 26 | Google/YouTube egress on the relay chain | VERIFIED | exits from ru1 (Russia), not Europe; ordinary traffic exits from Evolus (DE, IPv4 and IPv6) |
