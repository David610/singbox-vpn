# Production evidence ledger and device acceptance tests

This is the **one canonical current evidence ledger** for production and
device claims. Other audit and investigation documents are historical or
narrative records and must link here rather than independently upgrading a
claim. A code path existing, a config rendering, or a server-side probe passing
never proves that a real application works on a real Russian device.

## Evidence vocabulary

Use exactly these labels for behavioral claims:

- **USER-REPORTED** — preserved report without the complete reproducible test
  record required below. Useful evidence, never a production guarantee.
- **REPRODUCED** — repeated in a described environment, but not necessarily at
  the server/device layer required by the claim.
- **CODE-VERIFIED** — source inspection plus an executable code test.
- **SERVER-VERIFIED** — executed on a named real VPS with dated evidence.
- **DEVICE-VERIFIED** — executed on a named real device/network with dated
  version and commit evidence.
- **UNVERIFIED** — the required evidence does not exist in this ledger.
- **HYPOTHESIS** — a proposed explanation, not an observed capability.

CI-VERIFIED is used below as a more specific CODE-VERIFIED status for tests
executed in CI. `STATICALLY INSPECTED` means source/prose inspection only and
is not equivalent to any runtime verification.

## Canonical evidence ledger

Current as of 2026-08-22 for commit
`5370a30325bf9d95ab1d5250648809699b6eefff` (the audit baseline). Entries with
missing device/VPS/version data stay USER-REPORTED or UNVERIFIED.

| Claim | Scope | Status | Evidence | Date | Commit | Environment |
|---|---|---|---|---|---|---|
| Subscription JSON contains transport connection fields and selector/default hints, but no DNS, inbounds/TUN, MTU, IP-family policy, or `route.rules` | Renderer output | CODE-VERIFIED | `compat-config` renderer regression tests, including every compatibility mode | 2026-08-22 | `5370a30325bf9d95ab1d5250648809699b6eefff` | Local Rust tests; serialization layer only |
| VLESS server/client rendering agrees on REALITY public connection material and is accepted by the pinned sing-box configuration checks | Config generation/schema | CI-VERIFIED | `crates/compat-config/tests/reality_interop.rs`; CI sing-box validation | 2026-08-22 | `5370a30325bf9d95ab1d5250648809699b6eefff` | CI/config validation, not a public network |
| Hysteria2 loopback handshake/transfer interoperability | Protocol loopback | CI-VERIFIED | Existing compatibility/benchmark tests and CI | 2026-08-22 | `5370a30325bf9d95ab1d5250648809699b6eefff` | CI loopback, not a real device/network |
| Fresh AlmaLinux 9 installation of current HEAD | Full deployment lifecycle | UNVERIFIED | No dated disposable-host run for current HEAD | — | — | Required: fresh public AlmaLinux 9 x86-64 VPS |
| AlmaLinux 9 install and Hiddify connection succeeded | Prior deployment smoke observation | USER-REPORTED | Owner report retained in the dated entry below; required versions/transport/commit absent | 2026-08-16 | unknown | Reported real VPS + iPhone; incomplete record |
| Current subscription imports and tunnels on a real iOS/Android device | Current device compatibility | UNVERIFIED | No complete matrix entry | — | — | Required: named device/client/network/current commit |
| YouTube native playback over a Russian network | Application/device behavior | UNVERIFIED | Prior narrative reports lack device, network, client version, transport, date, and commit evidence | — | — | Required: Russian cellular and Wi-Fi runs |
| TikTok native playback over Russian Wi-Fi or cellular | Application/device behavior | UNVERIFIED | No completed TikTok matrix entry | — | — | Required: Russian Android/iOS device runs |
| DNS, IPv4/IPv6 leak prevention, MTU, kill switch, per-app routing, handover, or failover behavior | Client-owned runtime behavior | UNVERIFIED | Subscription intentionally does not control these fields; no complete device entry | — | — | Required per client/OS/network combination |
| Complete supported AlmaLinux 9 production lifecycle at the current audit baseline | Install, ACME/TLS/firewall, user mutations, rollback, backup/restore, update, reboot, uninstall/reinstall, and public transport egress | UNVERIFIED | No SSH credentials, key, target hostname, provisioning API, or disposable VPS was available in the 2026-08-22 execution environment; the destructive harness was therefore not invoked against a real host | 2026-08-22 | `212f830160f2dffff932d9fa099e699a912f551a` | Local Ubuntu container only; no systemd/SELinux/firewalld/ACME simulation counted |

The commit column identifies what was tested, not what is currently checked
out forever. When code changes, retain historical rows and add a new row; do
not silently carry a result forward.

Automated CI cannot validate a real Hiddify/v2rayNG import on a real
iOS/Android/MagicOS/Linux/Windows/macOS device against a real VPS — this
document is the explicit manual test matrix for that, per the honesty
rule already established in `docs/CLIENT_COMPATIBILITY.md`: a cell only
ever changes to PASS after a dated, filled-in entry exists below, never
from spec conformance or code review alone.

## Matrix

| Platform | Client | VLESS+REALITY | Hysteria2 | Subscription refresh | Network switch |
|---|---|---|---|---|---|
| iOS | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| Android | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| HONOR MagicOS | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| Android | v2rayNG | not yet tested | N/A (unsupported/not guaranteed — see `docs/clients/V2RAYNG_ANDROID.md`) | not yet tested | not yet tested |
| Linux | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| Windows | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| macOS | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |

## What each column means

- **VLESS+REALITY** / **Hysteria2**: the client successfully connects
  through that transport specifically (switch to it manually if the
  client auto-selects the other one first) and traffic actually egresses
  through the VPS (e.g. `curl ifconfig.me` shows the VPS's IP).
- **Subscription refresh**: after `vpn-admin user rotate-token` or
  `vpn-admin user rotate-vless`/`rotate-hysteria`, re-importing/
  refreshing the subscription in the client picks up the new
  credentials and the old ones stop working.
- **Network switch**: the connection survives (or promptly reconnects
  after) switching from Wi-Fi to mobile data and back, and after a
  screen-off idle period (mobile platforms).

## How to actually run this

Prerequisites:

1. A real AlmaLinux 9 x86-64 VPS with a public IP and two
   DNS names pointed at it (`vpn.example.com`, `sub.example.com` — see
   `docs/ALMALINUX_DEPLOYMENT.md`).
2. `sudo ./deploy/almalinux/install.sh` run there, completing without
   error (a failed install must not have printed "Install complete" —
   see `docs/PRODUCTION_HARDENING_PLAN.md` #22).
3. `sudo vpn-admin doctor` (or `vpn doctor`) on the VPS reporting no
   `[FAIL]` lines. Also run `sudo vpn-admin doctor --protocol` — it adds
   a best-effort `[L5-6]` check that dials the server's own REALITY
   listener with a throwaway `sing-box` client. Passing `doctor`
   without `--protocol` only proves L1-L4 (process/config/listeners/
   subscription-render-coherence); it does **not** prove a real client
   can authenticate — that is exactly what this whole matrix exists to
   verify by hand, and a real device test below should still be run
   even if both `doctor` variants are fully green.
4. `sudo vpn-admin user create --name test --qr` to get a subscription
   QR code / URL.
5. The device under test, on a real network, with the relevant client
   installed per `docs/clients/`.

For each matrix row (see `docs/CLIENT_PROTOCOL_BEHAVIOR.md` for what
each of the DNS/IPv6/tunnel-drop fields below actually mean and why they
are not something server-side config can guarantee):

```
Date:
Platform:
Client + version:
Device model / OS version:
VPS region / provider:
sing-box version (from `vpn-admin version` on the VPS):

Profile import:          PASS/FAIL
VLESS+REALITY:            PASS/FAIL
Hysteria2:                 PASS/FAIL / NOT TESTED
Subscription refresh:      PASS/FAIL
Network switch:            PASS/FAIL

Observed public IP after connecting (must equal the VPS's public IP):
Required client-side setting (e.g. Hiddify Service Mode = VPN/TUN, not
  Proxy Only — record whatever was actually needed):

DNS leak result (e.g. https://dnsleaktest.com with VPN on vs. off —
  record which resolver/location showed, PASS if it matches the VPS's
  location/ISP, FAIL if your real ISP/location leaked through):
IPv6 result (does the device have IPv6 connectivity at all before
  connecting? If so, does an IPv6-specific leak test show the VPS or
  your real network? N/A if the device/network has no IPv6 at all):
Tunnel-drop behavior (kill the VPS's sing-box process or disable Wi-Fi/
  data briefly — does the client fail closed, fail open, or hang?
  Record what actually happens, do not assume a kill switch exists):

Steps to disable/revoke and prove it took effect:
  1. `vpn-admin user disable test` on the VPS.
  2. Client attempts to reconnect/use the existing session — confirm it
     is rejected (REALITY/Hysteria2 handshake fails, or the client shows
     a connection error) within a reasonable time.
  3. `vpn-admin user enable test`, `vpn-admin user rotate-token test`.
  4. Re-import the new subscription URL on the client and confirm it
     connects again.
Revocation actually took effect: PASS/FAIL

Notes:
```

Paste the filled-in block above as a new dated entry directly below this
line once a real test is run, and update the corresponding matrix cell.

## Performance sanity check (not a benchmark)

Run once per real device/VPS test above, connected via REALITY (repeat
for Hysteria2 if also testing it). This is meant to catch an obvious
stall, fragmentation problem, reconnect failure, or routing break — it
is not a throughput measurement (`vpn benchmark` on the VPS, see
`docs/PERFORMANCE_OPTIMIZATION_PLAN.md`, is the real benchmark tool).

```
Browsing a few ordinary HTTPS sites:        PASS/FAIL (note any that hang/fail)
Sustained download (a large file, 1+ min):   PASS/FAIL (note if it stalls or dies mid-transfer)
Sustained upload (a large file, 1+ min):     PASS/FAIL
Disconnect and reconnect the client:         PASS/FAIL (note how long reconnect took)
Idle 5-10 minutes, then reuse the connection: PASS/FAIL (note if it needed a manual reconnect)

Notes (any stall, unusually slow transfer, or unexpected disconnect):
```

## Streaming / real-application matrix (P9)

The reported production symptom this section exists for: Safari plays YouTube
normally over the tunnel, the native YouTube iOS app does not, on both
REALITY and Hysteria2, with iCloud Private Relay ruled out as the
explanation. No single server-side root cause was established by the
investigation this section follows from — see that investigation's report
for the ranked hypotheses and evidence. `vpn doctor` and
`vpn benchmark` passing does **not** establish this matrix as PASS — this is
exactly the "healthy diagnostics, broken real application" gap those tools
cannot close (see `deploy/lib/vpn-investigate.sh streaming`/`youtube` for the
server-side diagnostics that narrow the search, but do not replace this real
on-device test).

As with every other matrix in this document: a cell only becomes PASS/FAIL
after it is actually exercised on a real device, never inferred. Run each
row twice — once with REALITY manually selected, once with Hysteria2
manually selected — so the two transports are never conflated.

| Test | Reality | Hysteria2 |
|---|---|---|
| YouTube — Safari playback | not yet tested | not yet tested |
| YouTube — native app playback | not yet tested | not yet tested |
| Large HTTPS download (1+ min) | not yet tested | not yet tested |
| Large HTTPS upload (1+ min) | not yet tested | not yet tested |
| Telegram media (photo/video) | not yet tested | not yet tested |
| Voice call (any app) | not yet tested | not yet tested |
| Video call (any app) | not yet tested | not yet tested |
| Idle 5-10 min, then resume | not yet tested | not yet tested |
| Wi-Fi -> cellular handover | not yet tested | not yet tested |
| Cellular -> Wi-Fi handover | not yet tested | not yet tested |

Row-filling template (paste as a new dated entry, one per transport
actually tested):

```
Date:
Client + version:
Device model / OS version:
VPS provider/region:
Transport under test: Reality / Hysteria2

YouTube Safari playback:            PASS/FAIL
YouTube native app playback:        PASS/FAIL
Large HTTPS download (1+ min):      PASS/FAIL
Large HTTPS upload (1+ min):        PASS/FAIL
Telegram media:                     PASS/FAIL
Voice call:                         PASS/FAIL
Video call:                         PASS/FAIL
Idle -> resume:                     PASS/FAIL
Wi-Fi -> cellular:                  PASS/FAIL
Cellular -> Wi-Fi:                  PASS/FAIL

Notes (exact failure mode — does playback never start, start then stall, or
  play at reduced quality? Any error shown in the app? Timestamps help):
```

### YouTube-specific record (fill in every field — do not mark PASS without a real test)

This is the specific record the reported production symptom needs. Every
field must be filled in from an actual observation on an actual device — an
empty or guessed field invalidates the entry.

```
Date:
Safari playback:              PASS/FAIL (note if playback starts, stalls, or never starts)
YouTube app playback:         PASS/FAIL (note if playback starts, stalls, or never starts)
Reality:                      PASS/FAIL/NOT TESTED (repeat this whole block once per transport)
Hysteria2:                    PASS/FAIL/NOT TESTED
IPv4/IPv6:                    (does the device have real IPv6 connectivity before connecting?
                                run `vpn-admin doctor` on the VPS and record its IPv6 posture line)
Hiddify version:               (Hiddify -> Settings -> About)
Bundled sing-box version:      (same screen — this is Hiddify's OWN core, not the VPS's)
iOS version:
VPS provider/region:

Server-side diagnostics run (paste the relevant PASS/WARN lines, not the full output):
  vpn doctor:
  vpn-investigate.sh streaming:
  vpn-investigate.sh youtube:

Notes:
```

### WARP control comparison (see `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §5)

Run on the SAME phone, SAME network, immediately before/after the
REALITY YouTube-app test above, following the same reset procedure
(§9.7 of that document — force-quit the app, verify public IP, fresh
launch). This is a control, not a fix: WARP working does not by itself
identify the cause, only which of the variables below actually differ.

```
Date:
Public IPv4 (singbox-vpn / WARP):
Public IPv6 (singbox-vpn / WARP, or "none" if unavailable):
DNS resolver observed (singbox-vpn / WARP, via a DNS-leak-test page):
YouTube app result — singbox-vpn (full breakdown per this doc's
  "Streaming / real-application matrix" row list):
YouTube app result — WARP (same breakdown):
Safari YouTube — singbox-vpn:
Safari YouTube — WARP:
TikTok result — singbox-vpn (see docs/TIKTOK_INVESTIGATION.md's breakdown):
TikTok result — WARP:

Which variables actually differed between singbox-vpn and WARP (not
just "WARP worked"):
```

## TikTok-specific matrix

Per `docs/TIKTOK_INVESTIGATION.md`, a prior tester USER-REPORTED that YouTube
worked (including native playback) and that TikTok was the application-specific
failure. Neither statement is DEVICE-VERIFIED. "TikTok
doesn't work" is not one test — a cell only becomes PASS/FAIL after it
is exercised on a real device on a real Russian network, broken down
exactly as `docs/TIKTOK_INVESTIGATION.md` section 4 specifies (app
opens, feed loads, thumbnails load, video starts, video stalls, comments
load, login works — never collapsed into a single verdict).

| Test | Reality | Hysteria2 | tcp-only |
|---|---|---|---|
| App opens | not yet tested | not yet tested | not yet tested |
| Feed metadata / thumbnails load | not yet tested | not yet tested | not yet tested |
| Video playback starts | not yet tested | not yet tested | not yet tested |
| Video playback sustains (10+ videos, scrolling) | not yet tested | not yet tested | not yet tested |
| Comments load | not yet tested | not yet tested | not yet tested |
| Login works | not yet tested | not yet tested | not yet tested |
| TikTok web (browser) — same breakdown | not yet tested | not yet tested | not yet tested |
| Mobile network, VPN off (baseline, no VPN involved) | n/a | n/a | n/a |
| Cross-VPN comparison (another VPN/Outline, same device) | not yet tested | n/a | n/a |

Row-filling template (paste as a new dated entry, one per profile
actually tested — see `docs/TIKTOK_INVESTIGATION.md` section 4 for full
methodology, including the mobile-vs-Wi-Fi and VPN-off baseline steps):

```
Date:
Location (country/network — "Russian residential" / "Russian mobile" /
  other, do NOT record exact GPS/address):
ISP / mobile carrier:
Wi-Fi or mobile data:
TikTok app version:
Client + version (Hiddify, etc.):
Device model / OS version:
Profile under test: Reality / Hysteria2 / tcp-only

Baseline — mobile network, VPN OFF:
  TikTok app opens/works at all:  PASS/FAIL (record exactly what happens)

With VPN on, this profile:
App opens:                        PASS/FAIL
Feed metadata / thumbnails:       PASS/FAIL
Video playback starts:            PASS/FAIL
Video playback sustains:          PASS/FAIL (note stall point if any)
Comments load:                    PASS/FAIL
Login:                            PASS/FAIL
TikTok web (browser):             PASS/FAIL (same breakdown)

Cross-VPN comparison (if run): other VPN/product used, and TikTok result
  through it, same device/SIM/network:

Exact error message(s), if any:
Does it fail immediately or time out? (note approx seconds)

Server-side diagnostics run (paste the relevant OK/WARN lines, not the
  full output):
  vpn-investigate.sh tiktok:

Notes:
```

## Telegram-specific matrix

Per `docs/TELEGRAM_RESILIENCE_PLAN.md`: "Telegram works" is not one
test. A cell in this matrix only becomes PASS/FAIL after it is actually
exercised on a real device on a real network — never inferred from the
general matrix above, from `vpn doctor`/`vpn doctor --telegram`, or from
YouTube/Instagram working. See `docs/TELEGRAM_TROUBLESHOOTING.md` for
exact per-row steps (disabling Telegram's own proxy first, switching
transports manually, etc).

| Test                          | Reality | Hysteria2 | Auto |
| ------------------------------ | ------- | --------- | ---- |
| App startup / connects at all  | not yet tested | not yet tested | not yet tested |
| Text messages send/receive     | not yet tested | not yet tested | not yet tested |
| Image download                 | not yet tested | not yet tested | not yet tested |
| Video/media download           | not yet tested | not yet tested | not yet tested |
| Media upload                   | not yet tested | not yet tested | not yet tested |
| Channels (large/high-traffic)  | not yet tested | not yet tested | not yet tested |
| Notifications / background reconnect | not yet tested | not yet tested | not yet tested |
| Voice call                     | not yet tested | not yet tested | not yet tested |
| Video call                     | not yet tested | not yet tested | not yet tested |
| Wi-Fi -> cellular handover      | not yet tested | not yet tested | not yet tested |
| Cellular -> Wi-Fi handover      | not yet tested | not yet tested | not yet tested |
| IPv4-only network               | not yet tested | not yet tested | not yet tested |
| IPv6-preferring network          | not yet tested | not yet tested | not yet tested |

Row-filling template (paste as a new dated entry, one per transport
actually tested):

```
Date:
Location (country/network — do NOT record exact GPS/address, only
  enough to know "Russian residential" vs "Russian mobile" vs "not
  Russia" etc):
ISP / mobile carrier:
Wi-Fi or mobile data:
Client + version:
Device model / OS version:
Hiddify version / sing-box core version (Hiddify -> Settings -> About):
Transport under test: Reality / Hysteria2 / Auto
Telegram internal proxy: confirmed DISABLED before testing (yes/no)

App startup:                    PASS/FAIL
Text messages:                  PASS/FAIL
Image download:                 PASS/FAIL
Video/media download:           PASS/FAIL
Media upload:                   PASS/FAIL
Channels:                       PASS/FAIL
Notifications/background:       PASS/FAIL
Voice call:                     PASS/FAIL
Video call:                     PASS/FAIL

Notes (exact failure mode, timestamps, anything unusual):
```

**This matrix cannot be filled in from outside Russia.** Development
performed on this repository has no way to reproduce Russian
residential/mobile ISP DPI behavior — see
`docs/TELEGRAM_RESILIENCE_PLAN.md` §"Remaining limitations". Every row
above stays "not yet tested" until a real family member/friend on a
real Russian network runs it and reports back.

## Entries

### 2026-08-16 — owner-reported release-readiness smoke test

- A real AlmaLinux 9 VPS installation was reported working.
- Hiddify on a real iPhone was reported to import/connect successfully and
  pass traffic through the VPS.

This is useful evidence that the basic supported path works, but it is not a
completed matrix entry: the iOS/Hiddify version, device/OS version, selected
transport, subscription refresh, credential revocation, network switching,
DNS/IPv6 behavior, and sustained-transfer results were not recorded. The
matrix above therefore remains unchanged rather than inferring PASS for tests
that were not individually reported.

### 2026-08-22 — Russia resilience architecture decision

| Claim | Scope | Status | Evidence | Date | Commit | Environment |
|---|---|---|---|---|---|---|
| A second provider/ASN endpoint improves availability compared with a third transport on the current IP | Proposed controlled A/B | HYPOTHESIS | Experiment design and failure-domain analysis in `docs/RUSSIA_PRODUCTION_INVESTIGATION.md`; no endpoint was provisioned and no Russian run occurred | 2026-08-22 | `f90b64adc97c38b111392e67b0f75219fd6a8a82` | Local source inspection only; external research access unavailable |
| VLESS+REALITY or Hysteria2 works on a current Russian fixed/mobile network | Russia data-plane availability | UNVERIFIED | No dated ISP/carrier/device/server-log/public-IP record | — | — | Requires the A/B matrix in the Russia investigation |
| HTTPUpgrade, HTTP, gRPC, WebSocket, or xHTTP improves Russia availability | Candidate transport behavior | UNVERIFIED | No current primary measurement or controlled deployment test; no transport added | — | — | Requires current-source research and one-variable field testing |

These entries do not alter earlier device rows. In particular, they do not
upgrade YouTube, TikTok, Telegram, handover, DNS, IPv6, or failover status.

### 2026-08-22 — PR #13 final release-acceptance audit

| Claim | Scope | Status | Evidence | Date | Commit | Environment |
|---|---|---|---|---|---|---|
| Attestation migration preserves historical `v0.1.2` installation while requiring provenance for `v0.1.3-rc.1` and later | Local release-policy behavior | CODE-VERIFIED | Bootstrap and updater regression tests cover the finite boundary, missing/wrong identity, modified archive, wrong package version, and checksum mismatch | 2026-08-22 | candidate derived from `441cb3d61f9834a005d5770583b897b18ff42352` | Local Ubuntu container; public GitHub unavailable; CI result not observed |
| A public RC was produced by the new workflow and its downloaded assets independently verified | Published release pipeline | UNVERIFIED | GitHub API, releases, Actions and attestations were inaccessible; no tag was created or release claimed | — | — | Requires authenticated GitHub access and an immutable RC workflow run |
| Public RC bootstrap on stock AlmaLinux 9 without preinstalled `gh` | Supported-host bootstrap | UNVERIFIED | No disposable AlmaLinux host or public RC was available | — | — | Requires stock AlmaLinux 9 x86-64 VPS |
| Complete supported AlmaLinux 9 production lifecycle for a provenance-required RC | Full server lifecycle | UNVERIFIED | No disposable VPS credentials or provisioning interface was available | — | — | Requires destructive lifecycle harness against a disposable public host |
| Compatible-device smoke test against the RC | Device interoperability | UNVERIFIED | No RC deployment or test device was available | — | — | Requires recorded client/device/network test |

These results are release-policy tests, not published-release, server, device,
Russia, YouTube, or TikTok acceptance evidence. PR #13 remains blocked until the
public RC and supported-host gates run successfully.

### 2026-08-22 — supply-chain hardening pass and production-readiness review

| Claim | Scope | Status | Evidence | Date | Commit | Environment |
|---|---|---|---|---|---|---|
| Every third-party GitHub Actions ref is pinned to an immutable full commit SHA with the mutable ref preserved as a trailing comment | CI supply-chain integrity | CI-VERIFIED | 32/32 `uses:` pinned in ci.yml/release.yml; `test-release-archive-contract.sh`/`test-release-reproducibility.sh` updated to enforce the SHA+comment invariant; full CI green on the result (run `32598898645`) | 2026-08-22 | `58ded16` (pins introduced in `a6a9cfd`) | GitHub Actions ubuntu-latest |
| Pinned upstream sing-box 1.13.19 carries Go-level vulnerabilities; attacker-reachable class given the shipped config is limited to pre-auth crypto/tls DoS/info-leak via the REALITY/Hysteria2 handshake paths; no fixed upstream stable exists (1.13.19 is latest stable, built with go1.24.7; fixes require go >= 1.25.8-1.25.13) | Data-plane dependency posture | CODE-VERIFIED | govulncheck v1.7.0 (DB 2026-08-21) symbol-level scan of pristine v1.13.19 source: 43 symbol-reachable findings triaged against the deployed config (masquerade type=file, no clashapi/debug/libbox/v2ray transports/ssh/tor/grpc enabled); remediation = bump `deploy/lib/versions.env` when a stable built with go >= 1.25.13 ships | 2026-08-22 | `12427f3` (pin unchanged since) | Local static/toolchain analysis; not a runtime exploit |
| First-party client (`David610/singbox-client`, Karing-based Flutter fork) declares Android/iOS support and parser/config-generation parity with the server renderer | Client-server compatibility surface | CODE-VERIFIED (client side) / DEVICE-UNVERIFIED | Client README states real-device end-to-end validation has not been performed; client commit inspected: `b24ba8b` | 2026-08-22 | client `b24ba8b` | Source inspection only |
| Fresh AlmaLinux 9 install of current HEAD (post-hardening) | Full deployment lifecycle | UNVERIFIED | No disposable VPS was available or provisionable in the 2026-08-22 evening execution environment (no provider credentials accessible; local virtualization disabled, so container simulation unavailable); `deploy/almalinux/lifecycle-acceptance.sh` remains ready and unrun against a real host | — | `58ded16` | None; supersedes nothing, upgrades nothing |
| Production-readiness decisions for <=10 trusted users, public unattended deployment, and Russian-network reliability | Overall verdicts | NO-GO / NO-GO / UNVERIFIED | Derived strictly from the rows above per the evidence rules of this ledger; private-owner use rated CONDITIONAL GO pending one dated harness run | 2026-08-22 | `58ded16` | Decision record |

These entries do not alter earlier device/Russia rows. No SERVER-VERIFIED or
DEVICE-VERIFIED status was created by this pass.
