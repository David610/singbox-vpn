# iPhone Shorts acceptance — eight-cell runbook (REALITY / Hysteria2 × Wi-Fi / cellular × signed-in / Incognito)

**Status of every cell below: BLOCKED — not performed.** Nothing in this file is evidence. A cell becomes PASS only
when its row in the results table is filled in from a real run on the exact release candidate and the record validates
with `deploy/lib/release_evidence.py` (fields: `device_matrix[]`, see `CELL_FIELDS` / `PLAYBACK_GATES` there). A browser
run, `curl`, an emulator, `playabilityStatus: OK`, or green CI is not acceptance (CLAUDE.md).

This extends [IPHONE_ACCEPTANCE_PROCEDURE.md](IPHONE_ACCEPTANCE_PROCEDURE.md) (general steps 0–9) with the Shorts matrix
and a way to tell *which layer* failed. Do one cell at a time and write the result down before starting the next.

## 0. Before you start (once)

| # | Check | How | Record |
|---|---|---|---|
| 0.1 | Exact candidate is installed on the server | on the VPS: `vpn status` and the release tag; take the SHA-256 of the running sing-box binary (`sha256sum "$(command -v sing-box)"` or the path `vpn status` reports) | tag, commit, `binary_sha256` |
| 0.2 | The same value goes in **every** cell | a cell with a different digest is rejected by the validator | one digest |
| 0.3 | Dedicated QA user, not your real account | `vpn user create --name qa-<date>`; remove it afterwards with `vpn user remove <id>` | user id (not the token) |
| 0.4 | Three QA profiles, terminal-only | `sudo bash deploy/lib/qa_profile_pack.sh <qa-user-id>` — prints (1) REALITY-only, (2) Hysteria2-only, (3) both. **It mints a new token**; do not screenshot or paste it | — |
| 0.5 | Device facts | iPhone model, iOS version, Hiddify version/build and App Store vs TestFlight, YouTube app version | `iphone_model`, `ios_version`, `hiddify_version`, `youtube_version` |
| 0.6 | Control run, VPN **off** | In the YouTube app (signed in) and in a private session, play 10 Shorts in a row on the same network. Pick one Short that failed before and note how to find it again | confirms the Short is playable at all |

Use profile (1) for the REALITY cells and (2) for the Hysteria2 cells so the transport under test is certain. Profile
(3) lets Hiddify balance across routes and **cannot** attribute a result to one transport.

**"Incognito" on iPhone.** The YouTube app has an Incognito mode (profile menu → *Turn on Incognito*). Safari/Chrome private
tabs are a *different* application path and do **not** count for the app cells. If you also test the browser, record it as
a separate, non-gating observation.

## 1. One cell (repeat for each row of the matrix)

Set the cell: transport profile imported and selected, network forced (Wi-Fi: cellular data off for the app or airplane
mode + Wi-Fi on; cellular: Wi-Fi off), YouTube mode (signed in, or Incognito on). Then, in order:

| Step | Action | Expected | Write down |
|---|---|---|---|
| 1 | Connect in Hiddify; open a "what is my IP" page in Safari | VPN indicator on; address is **62.238.46.190** (the node's own address). IPv6, if shown, is the node's, never your carrier's/ISP's | IPv4/IPv6 shown |
| 2 | Fully quit and reopen YouTube (swipe away); set the mode for this cell | — | — |
| 3 | Play **10 consecutive genuine Shorts** by swiping (no skipping back, no reopening). Include the known-failing Short from 0.6 | every one plays with picture and sound within a few seconds | count played (`shorts_consecutive_played`), and `known_failing_short_played` yes/no |
| 4 | Keep playing (Shorts or a long video) for **at least 900 s** without the player stalling out | continuous playback | seconds (`continuous_playback_seconds`) |
| 5 | Play a normal long video; seek forward and back | plays; seeks complete | `long_form_playback`, `seeking` |
| 6 | Quit and reopen YouTube; play 3 Shorts | works | `app_restart` |
| 7 | Hiddify: disconnect, reconnect; play 3 Shorts | works | `reconnect` |
| 8 | Network transition: Wi-Fi → cellular (or back) while connected; wait ≤ 30 s (Hysteria2 may need longer); play 3 Shorts | recovers and plays | seconds to recover |
| 9 | After a server reconcile (the agent/reconcile timer, or `systemctl restart` of the reconcile unit if the runbook allows it — **do not** hand-edit configs) play 3 Shorts | works | `post_reconcile_playback` |
| 10 | On the server, while a Short plays, confirm the traffic left via the intended Google egress (see §3) | matches the approved target | `google_egress_verified` |

Any step that does not meet "Expected" is a **FAIL for the cell**; record the step number and symptom, then go to §2. Do not
retry silently and do not reuse a lucky second attempt as a pass.

## 2. If Shorts fail: which layer?

Decide by *what still works* and run the minimum diagnostic for the matching row. Capture only what is listed; never capture
cookies, tokens, account names, watch history, URLs of videos, or packet payloads.

| Symptom | Likely layer | Minimal safe diagnostic | Reading |
|---|---|---|---|
| No site loads at all after connect | transport / tunnel | Hiddify log line count + the first error text (copy only the error, not the profile) ; server: `vpn status`, `systemctl status sing-box --no-pager` | profile wrong, port blocked, REALITY/Hysteria2 handshake failing |
| IP page shows your carrier/ISP address or IPv6 | leak / routing | screenshot of the IP page only (no other data) | **FAIL, stop**: traffic is not going through the tunnel |
| Ordinary sites work, YouTube **home and long videos** also fail | DNS or Google routing | in Safari open a page that shows the resolver (DNS-leak test) ; note resolver owner | resolver belongs to the node's network → DNS is tunnelled; otherwise DNS leak |
| Home page/long video works, **only Shorts** fail (spinner / grey) | egress reputation, family selection, QUIC, or app-mode | do §2.1 comparison | proceed to the table below |
| Works for ~N Shorts then stalls | connection reuse / session-dependent CDN decision | note N; repeat once after app restart only to see whether N changes (record both) | session-dependent rather than a hard block |

### 2.1 Comparison that separates the hypotheses (do on the same network, same Short)

| Run | Result to record |
|---|---|
| A. VPN off, signed in | plays? |
| B. VPN off, Incognito | plays? (the 2026-10-22 baseline: yes) |
| C. REALITY, signed in | plays? |
| D. REALITY, Incognito | plays? |
| E. Hysteria2, signed in / Incognito | plays? |

Interpretation (a table of *what to test next*, not conclusions):

- **A/B yes, C/D/E all no** → the problem follows the tunnel egress, independent of mode and transport (IP/family/Google routing).
- **C yes, D no, E mixed** (the historical 2026-10-22 pattern) → **session-mode-dependent** server-side decision *or* a
  client-side path that differs by mode. Transport-only defects are less likely because both transports failed anonymously.
  Next: server-side per-family byte counts for the window (`deploy/lib/diagnostics/af_capture.py`, aggregate only — it
  correlates traffic, it cannot name an app), and the egress-family check in §3.
- **REALITY works, Hysteria2 fails in the same mode** → transport-specific: UDP/443 reachability on that network, QUIC
  handling, or the Hysteria2 profile (obfuscation/SNI). Check the carrier/Wi-Fi allows UDP/443 (a test on another network).
- **Wi-Fi works, cellular fails (same transport/mode)** → network-specific (carrier filtering, IPv6-only path/NAT64, MTU).

### 2.2 Distinguishing DNS / IPv6 / transport / app-mode / egress

| Layer | Positive sign | Negative sign |
|---|---|---|
| DNS | DNS-leak page shows the node's resolver | shows carrier/ISP resolver → leak |
| IPv6 | IPv6 test shows the node's v6 or none | shows your own v6 → leak; or only-v6 destination fails |
| Transport | other apps/sites work through the same profile | nothing works, or only one transport fails |
| Application mode | same Short plays in one YouTube mode and not the other on the *same* tunnel | identical in both modes |
| Egress | VPN-off plays and tunnel doesn't, in both modes, both transports | tunnel plays in at least one mode |

Do not conclude "IP ban" or "IPv6 problem" from one cell. The 2026-10-22 IPv6 observation was never shown to be causal.

## 3. Server-side evidence (run by you in your own SSH terminal, nothing stored in chat)

For the window of the cell (note start/end times):

- `vpn status` before and after (node healthy, same release tag/commit and binary digest as 0.1).
- Aggregate per-family bytes for the window (IPv4 vs IPv6) with `af_capture.py` — aggregate counters only.
- Google egress check: that the Google/YouTube domain set left through the approved target (see
  [HAIRPIN_TARGET_QUALIFICATION.md](HAIRPIN_TARGET_QUALIFICATION.md) and
  [../YOUTUBE_FIX_INVARIANTS.md](../YOUTUBE_FIX_INVARIANTS.md)); record yes/no for `google_egress_verified` only if you
  actually observed it, otherwise leave it false.

Do **not** change routing, sniffing, the IPv6 reject, UDP/443 handling or the hairpin during a test. Those are protected
invariants; a changed configuration invalidates the cell and every cell after it.

## 4. Results table (copy per attempt; all rows start BLOCKED)

| Transport | Network | YouTube mode | Status | Shorts in a row | Known-failing Short | Playback s | Long/seek | App restart | Reconnect | Net switch | Post-reconcile | Egress verified | Evidence ref |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| REALITY | Wi-Fi | signed in | BLOCKED | | | | | | | | | | |
| REALITY | Wi-Fi | Incognito | BLOCKED | | | | | | | | | | |
| REALITY | cellular | signed in | BLOCKED | | | | | | | | | | |
| REALITY | cellular | Incognito | BLOCKED | | | | | | | | | | |
| Hysteria2 | Wi-Fi | signed in | BLOCKED | | | | | | | | | | |
| Hysteria2 | Wi-Fi | Incognito | BLOCKED | | | | | | | | | | |
| Hysteria2 | cellular | signed in | BLOCKED | | | | | | | | | | |
| Hysteria2 | cellular | Incognito | BLOCKED | | | | | | | | | | |

`Evidence ref` is a file under `docs/release-acceptance/` or private storage — screenshots of the IP page and the
counts are enough. Never attach a screen recording that shows your watch history, account, or subscription URL.

## 5. After the test

1. `vpn user remove <qa-user-id>`; the minted subscription URL stops working.
2. Delete screenshots that show anything beyond the IP page.
3. Put the results into `docs/release-acceptance/<tag>.evidence.json` only for cells you actually ran, then run
   `python3 deploy/lib/release_evidence.py` on it; the stable gate must continue to refuse anything short of all eight cells.

## 6. Other clients

Hiddify is the qualified client. INCY and Shadowrocket are **not** accepted on this matrix: see
[../CLIENT_COMPATIBILITY.md](../CLIENT_COMPATIBILITY.md) for what each can import. Run them only as separate, clearly-labelled
observations (they do not count toward `DEVICE_VERIFIED`).
