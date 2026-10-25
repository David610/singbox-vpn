# Russian-entry two-hop: verified baseline, client compatibility, egress, drift (2026-10-25)

Secret-free. Evidence classes: VERIFIED (observed, environment stated), UNVERIFIED, FIELD-ONLY (observed live, not reproduced in isolation).
All three servers were LIVE; this phase was read-only except two reversible test settings that were reverted
(see "Live changes made earlier today" at the end).

## A. Verified baseline

| Item | Value |
|---|---|
| main | `35d4a3f0d9ac15c1f474f80d072cde3e15dbf525` (CI green after one re-run: first attempt failed on a `groupadd` "Text file busy" race in a test helper; recorded) |
| latest published candidate | `v1.3.1-rc.6` = `35d4a3f`. Contains PR #206 (relay TLS spoof) and #207 (relay Hysteria2 self-test). Does **not** contain the Vision control in this branch |
| open PRs at start | #205 (friends onboarding docs) only |
| ru1 `135.106.178.167` | Ubuntu 24.04, 1 vCPU/1 GB, **relay** `node_id=ru1`, rc.6 (install-state `source_commit` 35d4a3f, source tree sha matches the tag, acceptance `accepted`), doctor 0 FAIL |
| Evolus `91.244.71.165` | Ubuntu 24.04, 2 vCPU, **exit** `node_id=91-244-71-165`, rc.6, doctor 0 FAIL, Google hairpin -> ru1 |
| fin1 `62.238.46.190` | Ubuntu 22.04, **exit**, still **rc.3** (executables `5b7943d5…`, source tree sha differs from rc.6), Google hairpin -> ru1, doctor 0 FAIL |
| sing-box | 1.14.1 everywhere, same executable hash `9334a1c1…` |

Confirmed active paths (packet captures and client tests, 2026-10-25):

1. Relay chain (pinned profile): client -> ru1:443 (REALITY, Vision) -> Evolus:443 (REALITY) -> Internet. Evolus saw inbound connections only from ru1, none from the client.
2. Google/YouTube from the relay chain: Evolus -> hairpin (REALITY) -> **ru1 -> Google with `tls_spoof`**. 266 packets left ru1 toward Google ranges, **0 left Evolus**.
3. fin1 users: client -> fin1 -> (Google set only) hairpin -> ru1 -> Google; everything else leaves fin1.

## B. Nested Vision

See `docs/RELAY_EXIT_VISION.md`. Summary: FIELD-ONLY failure, narrowed by live A/B to Vision on the detour-nested exit hop;
not reproducible in a loopback lab (curl and Chromium, four nodes, latency/segmentation injected); cause unclassified.
Implemented: opt-in per-route `vless_flow = "none"` (default unchanged) with validation, doctor note and tests.
Needs a new candidate and live re-validation.

## C. Client compatibility (Hiddify)

Decisive evidence = which host sees the client. Test: hiddify-core v5.0.3 (the Hiddify engine, `hiddify-core-linux-amd64-glibc.tar.gz`,
sha256 prefix `0d22f7b6aa23fdb1`, released 2026-10-18) `build` and `run` on a Linux workstation, with packet captures on the servers.
This is the engine inside the Hiddify apps, **not** an Android/iOS app run; app UI behaviour is UNVERIFIED.

| Subscription form | What it contains on this relay | What Hiddify's engine does | Verdict |
|---|---|---|---|
| `?format=hiddify` (share-link URIs, 254 B) | **one** `vless://` URI: the **direct** route to Evolus (share links cannot express `detour`) | connects straight to Evolus; captures on Evolus show the client's own address; exit IP is Evolus, so it *looks* fine | **silently bypasses the Russian entry. Do not use for a relay.** |
| `?format=singbox` / bare URL (2283 B) | first hop + direct + via route + selector/urltest | `build` wraps it in `select` with default **`balance`** over [first hop alone, direct, via]. The first hop alone is rejected by the relay, direct bypasses Russia | **unusable** |
| `?format=singbox&compat=hiddify-pinned` (1679 B) | hidden first hop (`Reality §hide§`, Vision) + one via route with `detour` | `build` keeps `detour`, one selectable route, `select` default = the via route, no balancer; `run`: exit IP Evolus 3/3, YouTube 200; Evolus saw only ru1 | **works as a two-hop in the engine** |

Open: app-level import and persistence, subscription refresh in the app (the served body is byte-identical on refetch, VERIFIED server side),
whether the mobile apps embed the same engine version. A physical-device test is the only decisive one.
Also observed: the engine's DNS lookups of `*.sslip.io` names timed out intermittently from the test host, so using sslip.io names as
relay/exit hosts adds a third-party DNS dependency for every friend (UNVERIFIED impact in Russia).

PR #205 correction: its onboarding text tells friends to import a `?format=hiddify` link and to choose between "Reality" and
"Hysteria2". That is right for the fin1 (single-hop) deployment and **wrong for the Russian-entry relay**, whose link is the pinned
singbox form with exactly one server. The PR has been edited accordingly.

## D. Egress classification (pinned relay profile, 2026-10-25)

| Traffic | Last egress | Evidence |
|---|---|---|
| ordinary HTTPS (IPv4) | **Evolus, DE** | `api.ipify.org` = 91.244.71.165; Cloudflare trace `loc=DE` |
| ordinary HTTPS (dual stack) | Evolus **IPv6** `2a12:bec4:19a5:9aa::` | `api64.ipify.org` |
| IPv6-only site | Evolus IPv6 | `ipv6.icanhazip.com` |
| Google/YouTube domain set (TCP) | **ru1 (Russia)** | 266 packets toward Google from ru1, 0 from Evolus |
| DNS for proxied names | resolved by the exit (name sent to the exit); client-side stub leaks are device-dependent | UNVERIFIED on a device |
| UDP/QUIC | not carried by the relay path (`capabilities=["tcp"]`, forwarding rule `network=tcp`); apps must fall back to TCP | by configuration; not traffic-tested (UNVERIFIED) |

Conflict for the owner (not decided here): Google/YouTube **do not exit from Europe** because YouTube refuses the European addresses
(fin1, Evolus) and accepts ru1. Strict European egress would break the working YouTube/Shorts behaviour. Alternatives to discuss:
(1) keep the hairpin and label Google as "exits via Russia"; (2) an additional accepted European address (not available: no extra VPS);
(3) accept broken Shorts. No live routing was changed.

## E. Configuration drift (live vs GitHub vs intent)

Rendered sing-box configs match what the source renders for each node (doctor L4 "exact match" passes, 0 FAIL on all three).
Differences are therefore outside the rendered file:

| # | Where | Difference | Category | Source of truth / action |
|---|---|---|---|---|
| 1 | ru1 | `/etc/systemd/system/sing-box.service.d/tls-spoof.conf` (adds CAP_NET_RAW, CAP_NET_ADMIN) | REQUIRED_BUT_UNDOCUMENTED (documented in `docs/TLS_SPOOF.md`, **not installed by the installer**; a fresh install/uninstall removes it and the spoof silently does nothing until recreated) | installer/`vpn-admin` should create it when `[google_egress_tls_spoof]` is set, or doctor already FAILs (it does) |
| 2 | ru1 | `[google_egress_tls_spoof]` in deployment.toml | INTENTIONAL_ENVIRONMENT_SETTING | supported field |
| 3 | ru1 | `[[peer_endpoints]]` evolus-direct / evolus-via-ru1 sharing one credential | REQUIRED_BUT_UNDOCUMENTED order of steps; becomes SOFTWARE_DEFECT-adjacent with Vision-less route (shared credential forces one flow) | `docs/RELAY_EXIT_VISION.md`; use a dedicated credential per relayed route |
| 4 | ru1 | users `fin1-hairpin`, `evolus-hairpin` flagged `google_egress_hairpin`; their UUIDs were transported to the exits with `google-egress-hairpin set --uuid-stdin` | REQUIRED_BUT_UNDOCUMENTED as a **two-host procedure** (state lives in users.json, not in a file) | needs a written pairing runbook in the next session |
| 5 | ru1/Evolus | hostnames are `*.sslip.io` | INTENTIONAL_ENVIRONMENT_SETTING + SECURITY_RISK (third-party DNS dependency; Let's Encrypt limits) | real domain for the final deployment |
| 6 | Evolus | `vpn-subscription.service.d/20-sub-request-log.conf` sets `RUST_LOG=debug` (shipped default `warn`) | OBSOLETE_WORKAROUND / SECURITY_RISK. Journal checked: 0 lines contain a `/sub/<token>` path | remove after owner approval |
| 7 | Evolus | `vpn-subscription.service.d/10-hardening.conf` (empty capability sets, `MemoryDenyWriteExecute`, syscall filter, …) not in the shipped unit | UNKNOWN origin; possibly worth upstreaming | evaluate, then fold into the shipped unit or delete |
| 8 | Evolus, fin1 | `[egress] ipv6_literal = reject_google` and a public IPv6 address | INTENTIONAL_ENVIRONMENT_SETTING (matches fresh-install default) | none |
| 9 | fin1 | still rc.3; ufw `22/tcp LIMIT`; several `*.pre-migration-*.bak` | DEPLOYMENT_DEFECT (version skew), INTENTIONAL (LIMIT) | upgrade or retire fin1 in the fresh build |
| 10 | all | sshd `PasswordAuthentication yes`, `PermitRootLogin yes`; 2-3 keys in `authorized_keys` | SECURITY_RISK (historical root passwords were exposed; rotation not verified) | owner procedure below |
| 11 | all | `/root` holds backups (`*.tar`, mode 0600, contain credentials), old logs and scripts; ru1 also holds Sep-Oct leftovers (`de1-pair.env`, `.claude-server-commander`, `arcana-backup-*`); Evolus and fin1 contain `.claude/` and `.claude.json` from 2026-10-25 | SECURITY_RISK / UNKNOWN | inventory with the owner, then remove or move off-host |
| 12 | ru1 | provider resolvers (`188.93.16.19/17.19`) vs stub on the others | INTENTIONAL_ENVIRONMENT_SETTING | document |
| 13 | all | `ip_forward=0`, no NAT rules | correct for this proxy-only design | none |

Would be lost on routine operations: #1 on a reinstall (not on `update.sh`; it survived the rc.3 -> rc.4 update on ru1), #4 and the
peer credentials on a reinstall or restore to a different role (`restore` refuses a role/node-id mismatch), nothing in #2/#3/#8 (stored in `deployment.toml`,
preserved by `render-config`, `update.sh` and `--repair`).

SSH: the working key login used in this session is the only access path verified. Do not disable password login until the owner has
(a) added a second owner-controlled key and (b) verified provider console access for recovery. Owner steps: add the key to `authorized_keys` on each host,
open a **second** session with the key, then set `PasswordAuthentication no` and `PermitRootLogin prohibit-password` in a drop-in, reload sshd, test a new login
before closing the old one; rotate the Cloudflare token in the Cloudflare dashboard (no value should be sent to chat).

## F. Live changes made earlier today (all authorised, for the record)

ru1 upgraded to rc.4 then wiped and reinstalled as a relay (rc.6); Evolus upgraded to rc.5 and rc.6; fin1 hairpin re-pointed to the new ru1;
pilot users created (`friend-pilot-1` on fin1, `friend-pilot-ru1` on ru1, dedicated exit credential `ru1-pilot-b` on Evolus, hairpin users on ru1).
Two Vision experiments (`vision-off-experiment` on one ru1 user and on `ru1-pilot-b`) were turned off again; verified `vision_off_experiment` is unset.
Test clients were removed. This phase made no live change.
