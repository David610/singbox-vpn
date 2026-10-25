# Friends-only rollout: acceptance procedure and evidence register (2026-10-25)

Scope: a small group of friends in Russia. One personal subscription URL per friend, imported once in Hiddify (Android/iPhone), then Connect.
No secrets (tokens, UUIDs, keys, full URLs) appear in this document. Status words: VERIFIED (observed by a test, environment stated), USER-REPORTED (owner statement, not reproduced), FAILED, BLOCKED (cannot be run with current authorisation or without a device), UNVERIFIED (no evidence).
Server tests are never proof of real-device success.

## Decision: NO-GO for the friends rollout (as of 2026-10-25)

1. FAILED: the default REALITY profile (SNI `www.cloudflare.com` to fin1:443) does not connect from the Russian datacenter vantage (ru1): 0/5 tunnels, and 0/4 for each alternative SNI tested. Hysteria2 to fin1 worked 5/5 from the same host. No friend profile prefers Hysteria2 today.
2. BLOCKED: no test on a real device in a Russian cellular or Wi-Fi network has been run. Datacenter results do not predict consumer ISPs or mobile operators.
3. The production exit that serves Google/YouTube (ru1) runs a development build, not a published release. It must be replaced by a tagged release candidate (PRs #203 and #204 merged, rc.4 built) before friends use it.
4. Friend accounts do not exist yet: creating each user reloads sing-box (a brief disconnect for everyone), which needs explicit approval.

## Evidence register

| # | Item | Status | Environment / evidence |
|---|------|--------|------------------------|
| 1 | fin1 runs published rc.3 binaries | VERIFIED | executable hashes match rc.3 (vpn-admin 5b7943d5, sing-box 9334a1c1, vpn-subscription-svc b6f37e36); doctor 0 FAIL |
| 2 | ru1 runs a release | FAILED | ru1 runs a development build of vpn/vpn-admin (958a05f2, PR #204 code) with the opt-in TLS spoof; doctor reports "release identity INCONSISTENT" as designed |
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
| 17 | Fresh friend user end to end on production | BLOCKED | needs approval for user creation (sing-box reload) |

## Open decisions

A. Transport for friends. Evidence favours Hysteria2 from Russia (items 10 and 11). Options: (i) test the plain 2-route import on a Russian device and see what Hiddify selects; (ii) add a Hysteria2-first or Hysteria2-only profile (small renderer change, new RC); (iii) change the REALITY decoy name to one that works from Russia (server change, restart, every profile re-imported). None is applied.
B. Replace the ru1 development build with a tagged rc.4 (merge #203 and #204, tag, install).
C. Approve creation of one user per friend on fin1 (one reload each; do them in one batch).

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
