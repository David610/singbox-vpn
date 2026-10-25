# Next-session handoff: fresh two-VPS validation (secret-free)

## Source baseline

- Base: `main` = `35d4a3f0d9ac15c1f474f80d072cde3e15dbf525`, published candidate `v1.3.1-rc.6`.
- Add: this branch (opt-in `vless_flow = "none"` for a relayed exit route, lab, docs). It is **not** in rc.6; cut a new candidate
  (`v1.3.1-rc.7`) from the merged result and say so; never describe rc.6 as containing it.
- Related: PR #205 (friends onboarding docs; corrected for the relay link form).

## Target configuration

Two VPSs, one real domain per host (avoid `sslip.io`), no extra infrastructure.

- Entry (Russia): `install.sh --role relay --node-id ru1 --domain <entry-domain> --reality-handshake-server <decoy>`.
  `[[access_paths]] relay-ingress` (installer-written). Peer exit declared once as a **relayed** route with its **own** credential and
  `vless_flow = "none"` (only if the Vision problem is still reproduced on the fresh hosts; otherwise keep Vision). No direct-route alias for friends.
- Exit (Europe): ordinary exit install. One dedicated user per relayed route/friend, `vision-off-experiment` set on that user only when the route is Vision-less.
- Google/YouTube: the exit hairpins the Google set to the entry (`google-egress-hairpin set --uuid-stdin`, UUID of a hairpin user flagged on the entry);
  the entry carries `[google_egress_tls_spoof]` plus the `sing-box.service.d` capability drop-in (CAP_NET_RAW, CAP_NET_ADMIN). Both are needed for YouTube from Russia.
  Product decision pending: Google/YouTube therefore **exit from Russia** (see baseline section D).
- Clients: Hiddify with the `?format=singbox&compat=hiddify-pinned` link only (one server). `?format=hiddify` bypasses the entry; bare/unpinned gets a Hiddify balancer.

## Verified so far

Relay + exit + hairpin + spoof functioning on live rc.6 hosts; Hiddify engine (hiddify-core v5.0.3) keeps the pinned two-hop and bypasses it for share-links;
egress classification; Vision A/B on the live chain; config-level tests for the Vision control (94 relay tests, full compat-config suite, clippy).

## Not verified / still to do

- Vision control on real hardware after this change; root cause of the nested-Vision failure (unreproducible in the loopback lab).
- Hiddify mobile apps (import, persistence, refresh, restart), native iPhone/Android YouTube, DNS leaks, Telegram, Russian consumer ISPs and mobile operators.
- UDP/QUIC behaviour of the relay path (design says TCP only).
- Installer support for the spoof drop-in, a written two-host hairpin pairing runbook, real-domain TLS, SSH hardening, cleanup of `/root` leftovers and debug-log drop-in.

## Fresh-install test matrix (run on two blank VPSs)

1. Role install (relay, exit) from the published candidate with checksum and attestation verification; doctor 0 FAIL on both.
2. Pair: exit user -> entry peer credential; relayed profile fetch (pinned) returns 200 with one server; unpaired relay returns 503 `no_selectable_route`.
3. Traffic: client via entry -> exit IP is the exit; Evolus-style capture shows the exit sees only the entry; Google set exits from the entry (or per the owner decision).
4. Vision: Chromium HTTP/2 (`m.youtube.com`, 16 parallel streams) with Vision on; if it hangs, repeat with the Vision-less route; record which config passes.
5. Lifecycle: create/rotate/disable/expire/renew credentials, subscription refresh (body stable), `render-config`, reconcile, `repair`, upgrade, downgrade refusal, reboot, backup/restore (restore refuses a role mismatch).
6. Fail-closed: stop the exit -> relayed profile fails visibly, no direct fallback; stop the entry hairpin -> Google fails, no leak from the exit address.
7. Security: no secrets in logs/diagnostics (`RUST_LOG` default), SSH key-only login after the owner procedure, firewall allows only 22/443/8443.
8. Device: Hiddify import of the single link, connect, browse, YouTube Shorts signed in/out, long video, Telegram, reconnect, refresh, app restart; Russian mobile and Wi-Fi.

## Minimum prerequisites

Two VPSs (Ubuntu 22.04/24.04 acceptable, AlmaLinux 9 is the supported gate), root key access with a verified recovery path, one DNS name per host
(A record pointing at the host; owner-side), outbound HTTPS to GitHub/Sigstore/Let's Encrypt from both hosts (the updater fails closed otherwise; Sigstore TUF was unreachable from a Russian host at times),
ports 22/443/8443 open at the provider, a phone on a Russian network for the device matrix.
