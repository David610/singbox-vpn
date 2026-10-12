# Release issue ledger — v1.3.0 preparation (2026-10-12)

Scope: the owner's standalone single-VPS deployment (VLESS+REALITY, Hysteria2,
subscription/direct links, native iPhone YouTube **and TikTok**). Website, billing,
Supabase, Telegram and Tamara are DEFERRED BY OWNER (see "Scope" below). Base: `main` at
`ceb751c4d737b12be1bb01efb39e13c7743fe0eb` (PR #151 merged); latest published
stable `v1.2.0`; open PRs #121, #147, #148 are dependency updates and are
deliberately **not** part of this release. No tag, release, merge, production
deployment, DNS, firewall, sysctl or credential change is authorised by this
document or its PR.

Evidence classes follow `docs/DEVICE_ACCEPTANCE_TESTS.md`: **code-verified**
(unit/integration/fixture), **CI-verified**, **server-verified** (real host),
**browser-verified**, **native-device verified**, **user-reported**, and
**UNVERIFIED**. Nothing here is promoted above the evidence it has. Real
credentials, tokens, subscription URLs and host addresses are intentionally
absent; the trial host is called "the trial VPS".

## How items are classified

| Class | Meaning |
|---|---|
| BUG | A defect in repository code/scripts/workflows; fixed or open |
| POLICY | Expected behaviour of a deliberate rule (e.g. approval gates, `reject_all` breaking IPv6-only literals) |
| PREREQ | A test that needs a privilege/tool/network the environment lacked; not an application failure |
| UNSUPPORTED | A configuration outside the supported product boundary |
| EVIDENCE | A claim whose proof is missing; carried as pending, never as PASS |
| HISTORY | A past report or measurement preserved as a dated record |
| EXTERNAL | Needs an owner action or authorisation (infrastructure, device, release) |

## Summary

| ID | Item | Class | Disposition |
|---|---|---|---|
| R-1 | Shorts fail on a plain exit for literal-IPv6 Google destinations | BUG (policy shipped in #151) | Server behaviour verified; native acceptance **PENDING** (owner reports it works — provisional) |
| R-2 | Manual executable copy: build identity not distinguishable | BUG | FIXED (this PR) — recorded hashes + doctor classification |
| R-3 | Older updater/downgrade would silently ignore `[egress]` | BUG | FIXED (this PR) — capability gate refuses the switch |
| R-4 | VPS acceptance workflow never ran the egress-policy stage | BUG | FIXED (this PR) |
| R-5 | Acceptance SSH trusts the first host key seen | BUG | FIXED (this PR) — verified host keys required for release runs |
| R-6 | `vpn-admin … \| head` panics on a closed pipe (exit 101) | BUG | FIXED (this PR) |
| R-7 | Loopback stub resolver reported as an unexpected public listener | BUG | FIXED (this PR) |
| R-8 | "firewall-cmd not available — firewall check skipped" on ufw hosts | BUG | FIXED (this PR) |
| R-9 | nginx stock site leaves TCP/80 open on Debian-family hosts | BUG | FIXED in #151 (D-19); live renewal **PENDING** |
| R-10 | `vpn user links <name>` fails; only the ID works | BUG | FIXED (this PR) — unique names resolve, ambiguity rejected |
| R-11 | Probe credentials stopped matching the rendered user | EVIDENCE | Cause **UNKNOWN**; policy changes proven non-rotating |
| R-12 | New hostname "Failed host lookup" on the phone (router NXDOMAIN cache) | UNSUPPORTED/ENV | Documented (D-21) |
| R-13 | Direct URI described as DNS-independent | BUG (docs) | FIXED in #151 (D-23) |
| R-14 | Subscription-token minting is approval-gated | POLICY | Documented; never bypassed |
| R-15 | Broad `pkill -f` / leftover temp files in ad-hoc probing | HISTORY + guard | Repository contains none; static guard added |
| R-16 | Local test limits (root/CAP_NET_ADMIN, external gstatic, idn/pcap) | PREREQ | See "Automated qualification" |
| R-17 | Accepted-IP hairpin must not regress | — | Unchanged; existing tests green |
| R-18 | Ubuntu 24.04 support tier | UNSUPPORTED/EVIDENCE | AlmaLinux 9 supported; Ubuntu best-effort — lifecycle evidence **PENDING** |
| R-19 | Exact-RC disposable lifecycle (fresh, v1.2.0→RC, reboot, renewal, rollback) | EXTERNAL | **PENDING** |
| R-20 | Exact-RC native iPhone matrix (2 transports × 2 networks) | EXTERNAL | **PENDING** |
| R-21 | Passwords/tokens pasted into chat during investigation | EXTERNAL | Owner must rotate; none copied here |
| R-22 | Live inventory of the earlier two-node deployment | HISTORY | UNVERIFIED; not inferred |
| R-23 | D-1…D-25 consistency audit | — | Carried from the audit; not reopened |
| P-1…P-11 | Mandatory VPN-only gates: Shorts, long-form, TikTok app/web, subscription/direct links, ordinary use, soak/expiry, TLS renewal, backup/rollback, lifecycle + hybrid transition, owner cutover | EXTERNAL / EVIDENCE | see "Product acceptance gates (VPN-only)"; **none has native-device or real-host evidence yet** |
| P-D1…P-D3 | Website, provisioning journey/Supabase/billing/Telegram, Tamara | DEFERRED BY OWNER | not PASS, not FIXED, not a dependency |

## Detail

### R-1 — Shorts on a plain exit with a literal-IPv6 Google destination
- **Observed:** On a single-VPS exit whose IPv4 address YouTube accepts, Shorts
  show "Video unavailable" on an iPhone while long videos play.
- **Evidence:** *Browser-verified* (Chrome iPhone emulation through the real
  REALITY tunnel, 2026-10-12): literal IPv4 6/6, domain name 6/6, literal IPv6
  0/6; plain-TCP controls agree. Re-measured with `main` (`ceb751c`) on the trial
  VPS: policy `allow` → literal IPv4 6/6, literal IPv6 0/6; policy
  `reject_google` → literal IPv6 refused in ≈19 ms (curl 000, browser
  `ERR_SSL_PROTOCOL_ERROR`), literal IPv4 6/6, domain 6/6, a non-Google literal
  IPv6 destination still reachable (it leaves from the host's IPv6 address).
  *User-reported* afterwards: playback "looks like it works" (transport, network
  and duration not recorded). Phone socket sampling never attributed a failed
  Short to an IPv6 flow; the earlier "138 IPv6 destinations" figure is
  withdrawn (it counted IPv4-mapped inbound peers).
- **Root cause (measured, not native-proven):** YouTube refuses Shorts media to
  this host's IPv6 range; an iOS TUN client resolves DNS itself and sends
  literal IPs of both families, and a plain exit routes literal IPv6 directly.
- **Required behaviour:** Literal-IPv6 Google/YouTube flows must not leave from
  the refused range, without breaking other IPv6, REALITY/Hysteria2, or the
  accepted-IP hairpin.
- **Implementation:** `[egress] ipv6_literal = allow | reject_google | reject_all`
  (PR #151; exact rules in `docs/EXIT_IPV6_POLICY.md`). Default stays `allow`
  for a plain exit; the hairpin keeps its reject.
- **Verification:** 79 policy tests, 88-assertion real-core lab, browser matrix
  above. **Not established:** that the native app retries over IPv4 after the
  proxy closes the flow (a reject on a proxy inbound is a close, not a proven
  ICMP to the TUN), ECH/absent-SNI flows, Hysteria2/QUIC literal-IPv6 on a
  device.
- **Disposition:** server behaviour VERIFIED; whole fix **BLOCKED_ON_DEVICE_EVIDENCE**
  until R-20. No "Shorts fixed" claim is made.

### R-2 — Manual executable copy cannot be told from a release
- **Observed:** `main`'s `vpn`/`vpn-admin` were copied over v1.2.0; both report
  `1.2.0`, so nothing showed the host was not a release deployment.
- **Evidence:** server-verified on the trial VPS; code-verified here.
- **Root cause:** the binary embeds only the Cargo version; the install-state
  manifest recorded the release tag but no executable identity.
- **Required:** operators can distinguish published stable, exact RC,
  development build, and partial/inconsistent deployments, with no new wire
  schema and no change to release binaries' bytes.
- **Implementation:** installer and updater record `vpn_admin_sha256` and
  `subscription_sha256` in `install-state.json`; `doctor` (L1) compares them and
  the version core (`apps/admin/src/release_identity.rs`). A manifest without
  hashes is reported UNKNOWN, never as a match. `update.sh` marks its pre-commit
  verification as update-in-progress.
- **Verification:** 11 unit tests (stable/RC/development, same-version swap,
  core mismatch, missing file, unrecorded, update-in-progress, hash vector);
  manifest-writer functional test; static test of update.sh commit/doctor wiring.
- **Disposition:** FIXED, code-verified. Limitation: a host installed before this
  release has no recorded hashes until `update.sh --repair` (reported UNKNOWN).
  Live behaviour on a disposable host is R-19.

### R-3 — Policy silently lost on an older binary
- **Observed (by inspection):** v1.2.0's `DeploymentConfig` does not deny unknown
  top-level tables, so v1.2.0 ignores `[egress]` and re-renders a plain exit.
- **Required:** never silently lose a security-relevant setting on update,
  repair or downgrade.
- **Implementation:** `vpn-admin config validate` prints
  `capability: exit-egress-policy`; `update.sh`'s pre-switch check refuses to
  switch a deployment that declares `[egress]` to a `vpn-admin` that does not,
  before any live change, naming `vpn-admin egress clear` as the explicit way to
  drop the policy. The deployment schema version is intentionally unchanged
  (bumping it would make *every* rollback to v1.2.0 fail even without a policy).
- **Verification:** shell/Rust marker agreement, static wiring, helper behaviour
  (comment/look-alike/missing-file cases) in `test-node-identity.sh`; Rust test
  that `config validate` prints the marker.
- **Disposition:** FIXED, code-verified. Limitation: v1.2.0 itself cannot be
  changed; the protection applies when switching *from* a build with this gate.

### R-4 / R-5 — Acceptance workflow gaps
- **Observed:** `vps-acceptance.yml` never passed `--verify-egress-policy`; the
  harness trusted the first SSH host key (`accept-new`).
- **Implementation:** inputs `verify_egress_policy` (default true) and
  `ssh_known_hosts`; a run with a release `version` must request the stage, must
  not skip the reboot and must supply a verified host key; the harness fails if
  a requested stage did not run and supports `--known-hosts-file`
  (`StrictHostKeyChecking=yes`); the workflow independently greps the log for
  stage 16c PASS and the gate PASS and writes the result to the job summary.
- **Verification:** mocked-ssh harness scenarios (pass / failing stage / opt-in /
  strict-key / missing file) and a workflow contract test that executes the
  workflow's own prerequisite and evidence snippets.
- **Disposition:** FIXED, code-verified; never run against a real host here
  (R-19).

### R-6 — Broken pipe
- **Observed:** `vpn-admin egress show | head -1` printed a Rust panic (exit 101).
- **Implementation:** a panic hook recognises only a stdout/stderr EPIPE, lets
  the panic unwind (locks and rollback guards still drop) and maps it to 141;
  `main` prints `Error: …` and exits 1 for every real failure as before.
- **Verification:** failing-first integration test (25 iterations with the pipe
  closed) + a test that a real failure stays non-zero with its message.
- **Disposition:** FIXED, code-verified.

### R-7 / R-8 — Doctor listener and firewall diagnostics
- **Observed (trial VPS):** WARN for `127.0.0.53%lo:53` "unexpected listener";
  WARN "firewall-cmd not available — firewall check skipped".
- **Root cause:** `ss` prints a zone suffix (`%lo`) that defeated IP parsing, so
  a loopback-only listener looked public; the firewall check assumed firewalld.
- **Implementation:** zone-stripped loopback detection; the host firewall is
  judged by the backend the installer recorded (ufw: `ufw status`; firewalld:
  unchanged behaviour); wildcard listeners are described as "non-loopback,
  reachability UNKNOWN", provider firewalls are always reported UNKNOWN.
- **Verification:** unit tests (stub resolver yields no finding, wildcard
  wording, backend matrix incl. Ubuntu with no firewall-cmd).
- **Disposition:** FIXED, code-verified. The nginx `:80` listener seen on the
  trial VPS is R-9.

### R-9 — nginx stock site on TCP/80
- Fixed in #151 (ownership-aware disable of the checksum-matching package link,
  rollback, uninstall; D-19). The trial VPS still shows it because it was
  installed from v1.2.0, which predates the fix. **Disposition:** FIXED locally;
  live renewal/firewall restoration **PENDING** (R-19). Never blanket-disable
  TCP/80; certbot HTTP-01 handling is unchanged.

### R-10 — `user links` by name
- **Observed:** `vpn user links david` → "no such user"; the ID worked.
- **Implementation:** the stable ID always wins; otherwise a name matches only
  when exactly one non-reserved account has it; duplicates are rejected with the
  candidate IDs; reserved probe principals resolve by ID only. Read-only command
  only (mutating commands still take IDs).
- **Verification:** 5 unit tests. **Disposition:** FIXED.

### R-11 — Credential mismatch during probing
- **Observed:** a temporary test client with a saved UUID was rejected until it
  was rebuilt from the current user.
- **Evidence:** server-verified read-only: the user store's modification time
  (11:10 UTC) predates every policy operation of that session (≈13:30 UTC); the
  user was created 11:09 UTC; no rotate/lease/renew messages in the journal;
  the provisioning agent is not installed. Policy `set`/`clear` therefore did
  not change it.
- **Root cause:** **UNKNOWN.** Most likely a stale saved client configuration
  from an earlier session's user; not reproducible as an application defect.
- **Required:** an egress-policy change must never rotate credentials.
- **Verification:** new automated test — `user create`, then `egress set`
  (two modes), `render-config`, `egress clear`: the user store bytes, the
  `user links` URIs and the key material are identical after every step.
- **Disposition:** policy operations PROVEN non-rotating; the original cause
  stays UNKNOWN (test-harness hygiene). Operator-created users use static
  legacy credentials with no lease; native grants/leases apply only to
  authorisation-backed principals. After any `rotate-*`, clients must re-import
  (see `docs/clients/HIDDIFY_IOS.md`, token-vs-credential table).

### R-12 / R-13 — DNS and "DNS-independent" claims
- Router NXDOMAIN caching for a new hostname and the wording that a hostname URI
  is DNS-independent: documented/corrected in #151 (D-21, D-23). A direct URI
  still resolves the endpoint hostname; subscription fetch independence is not
  DNS independence. A TLS-valid raw-IP fallback is **not** offered (hostname-only
  certificate). **Disposition:** documented; client address override DEFERRED.

### R-14 — Token minting is approval-gated
- `user rotate-token` and `user qr` mint a new token and invalidate the previous
  subscription URL (transport credentials are unaffected); `user links` is
  read-only. The approval layer rejected a mint as a secret-store write during
  the trial; that is a control, not a bug, and was not bypassed. Token
  invalidation vs credential revocation: `docs/clients/HIDDIFY_IOS.md`.

### R-15 — Process and temp-file hygiene
- The repository's tools clean up only processes they own (`egress_family_lab.py`
  tracks its `Popen` objects, terminates then kills in `finally`; private
  `0700`/`0600` credential files); no `pkill -f`/`killall` exists in
  `deploy/` or `.github/`. A static test now forbids them. The ad-hoc scripts
  used during the live trial are outside the repository.

### R-16 — Test prerequisites
- See "Automated qualification" below for exact results and what could not
  run. Filtered or skipped tests are not complete evidence.

### R-17 — Accepted-IP hairpin
- Target, dedicated credential, `sniff → reject(no_drop) → route`, no direct
  fallback and the full `GOOGLE_EGRESS_DOMAINS` set are untouched
  (`youtube_invariants_doc`, relay/exit policy tests). The single-VPS policy does
  not claim to fix exits whose IPv4 address is also refused.

### R-18 … R-20 — External gates (PENDING)
- **R-18:** AlmaLinux 9 is the supported target; Ubuntu 24.04 is best-effort. The
  owner's host is Ubuntu, so Ubuntu 24.04 lifecycle evidence is required before
  any support statement beyond best-effort.
- **R-19:** exact-RC disposable lifecycle on AlmaLinux 9 **and** Ubuntu 24.04:
  fresh install; v1.2.0 → RC; reproduce-the-manual-copy then supported upgrade;
  repeated repair/reconcile; reboot; certificate renewal with firewall
  restoration; injected-failure rollback; backup/restore; uninstall/reinstall.
  Requires authorised disposable hosts and verified host keys.
- **R-20:** exact-RC native iPhone matrix — REALITY and Hysteria2 × Wi-Fi and
  cellular — per `docs/NATIVE_IPHONE_IPV6_ACCEPTANCE.md`. The owner's
  "looks like it works" report is recorded as provisional only.

### R-21 / R-22 — Owner actions and history
- Passwords and tokens pasted during the investigation (several VPS root
  passwords, a Cloudflare token, subscription URIs) must be rotated by the
  owner; none appears in the repository. The earlier two-node inventory and the
  decision about it are dated history, UNVERIFIED today.

### R-23 — D-1…D-25
- Dispositions are in
  `docs/reviews/YOUTUBE_IPV6_CONSISTENCY_AUDIT_2026-10-12.md`; none reopened.
  Still pending there: live renewal after the nginx change (R-9) and the
  general client address-override follow-up (R-12).

## Scope (owner decision, 2026-10-12 — supersedes the earlier product scope)

The owner wants **singbox-vpn now**. The Arcana website (`vpn-web`), billing,
the Supabase deployment, the Telegram Mini App and the Tamara app are **deferred
tasks**. They are **DEFERRED BY OWNER** in this ledger — not PASS, not FIXED, not
accepted — and none of them is a dependency of this release or of its decision.
Their earlier observations (P-D1…P-D3 below) are kept as dated context for the
future task, not as findings against this release.

Deferring those consumers does not permit breaking the backend contracts they
rely on: the subscription format and profile title, the provisioning contract
and `apps/provisioning-agent`, and their regression tests stay as they are.
Standalone operation must not depend on any of them: an operator-created
account (`vpn user create`) is usable with no control plane and no provisioning
agent, and the soak/lifecycle gates are written for that mode.

**Mandatory for this release:** native YouTube Shorts and long-form, native
TikTok and TikTok web, ordinary VPN use (browsing, transfers, legitimate IPv6),
subscription import/refresh and direct links, REALITY and Hysteria2, update /
repair / reconcile / reboot persistence, TLS renewal, tested upgrade and
rollback, and the owner's standalone cutover. **Device-test client:** Hiddify
(iOS) with its embedded sing-box core — the client the owner actually uses — via
the normal subscription URL or `vpn user links`; no website purchase, Tamara
build or custom client is a prerequisite.

## Product acceptance gates (VPN-only)

Evidence levels as above. "Candidate" means the exact immutable RC, which does
not exist yet (no tag has been created); every device/server cell therefore says
NOT RUN unless a dated trial entry is cited.

### Component manifest (backend only)

| Component | Identity | Deployed identity |
|---|---|---|
| singbox-vpn backend | PR #152, branch `release/1.3.0-prep`; proposed `v1.3.0-rc.1` (not tagged) | none yet — no RC exists |
| Latest published backend | `v1.2.0` | the owner's single Ubuntu VPS runs v1.2.0 plus hand-copied `main` (`ceb751c`) executables and `reject_google` (a hybrid, see P-10) |
| sing-box | `1.14.1`, checksum-pinned in `deploy/lib/versions.env` | `1.14.1` on the single VPS |
| Provisioning agent | `apps/provisioning-agent` in this repo (contract and tests preserved) | not installed on the single VPS; deliberately stopped and disabled on the earlier relay node (owner decision, preserved); **not required** for standalone accounts |
| Client / core | Hiddify (iOS) and its embedded sing-box fork; native YouTube and TikTok apps | versions not recorded in any acceptance entry; must be recorded per run |

The VPN hostname used for the standalone server is the one in its
`deployment.toml`; nothing in this release changes DNS.

### P-1 — YouTube Shorts (native iPhone)
- **Reproduction:** native YouTube app through Hiddify; open `sAcElROnYIE`, then ≥ 10 consecutive Shorts.
- **Expected:** playback starts and continues on REALITY and Hysteria2, Wi-Fi and cellular; when `reject_google` closes a literal-IPv6 flow the app recovers over IPv4 (a correct rejection alone is not success).
- **Root cause:** per-family refusal by YouTube of the exit's IPv6 range (measured with browser emulation); the native retry behaviour after a proxy reject is UNKNOWN.
- **Implementation:** `[egress] ipv6_literal` (#151); explicit persisted policy for the owner's host at cutover (P-11); accepted-IP hairpin unchanged for exits whose IPv4 is refused.
- **Automated:** policy tests, 88-assertion real-core lab. **Server:** trial 2026-10-12 (browser emulation, 6/6 on IPv4/domain, IPv6 refused in ≈19 ms). **Device:** owner-reported "looks like it works" (provisional; version/transport/network not recorded).
- **Blockers:** exact-RC native run on the 4-cell matrix (smoke first, then full).

### P-2 — YouTube long-form
- **Reproduction:** a long video, seek forward/back, 15 minutes sustained, reconnect and idle/resume mid-play.
- **Expected:** no stall beyond normal buffering; plays on both transports and networks.
- **Root cause:** n/a (not reported failing). **Implementation:** none specific. **Evidence:** none for the candidate; the owner's 2026-10-11 report covered a normal video once.
- **Blockers:** exact-RC native run.

### P-3 — TikTok (native app) — mandatory
- **Reproduction:** cold start, feed load, thumbnails, video start, 10 consecutive videos, 15 minutes of scrolling, comments and profile views, applicable account access, on each transport/network; with a VPN-off baseline.
- **Expected:** all of the above work, or the failing operation and hosts are identified and the cause classified.
- **Root cause: UNKNOWN.** `docs/TIKTOK_INVESTIGATION.md` holds hypotheses only (QUIC/UDP preference, DNS, IPv6, routing/sniffing, exit-IP or region restriction) — all UNKNOWN; the baseline "TikTok fails" is user-reported with no device, client, network or transport record. The Google IPv6 policy is **not** assumed to apply, and no TikTok domain list, global UDP block or host IPv6 shutdown is introduced.
- **Implementation:** none yet. Existing read-only server probe `vpn-investigate.sh tiktok` (root domains only). A TCP-only profile (`?compat=tcp-only`) is available as a controlled diagnostic experiment, not a proposed default.
- **Evidence:** NOT RUN.
- **Blockers (device, owner):** run the smoke then the matrix in "TikTok diagnostic procedure"; diagnosis alone is not a PASS. If an external restriction of the exit (IP/region) is the answer, report it precisely with the options that need authorisation.

### P-4 — TikTok web — mandatory
- **Reproduction:** tiktok.com in a mobile browser and a desktop browser through the VPN: home feed, one video, search. **Expected:** loads and plays. **Root cause:** UNKNOWN. **Evidence:** NOT RUN. **Blockers:** a browser on the accepted network; HTTP 200 on the root page is not success — a video must play.

### P-5 — Subscription import/refresh and direct links — mandatory
- **Reproduction:** import the normal HTTPS subscription URL into Hiddify, refresh it, then import a direct `vless://` / `hysteria2://` link from `vpn user links`; reconnect with each.
- **Expected:** both import paths connect on both transports; after `rotate-token` the old subscription URL stops and the new one works while connected sessions are unaffected; a direct link needs the endpoint hostname resolved (it bypasses subscription fetching, not DNS).
- **Evidence:** automated contract and CLI tests; server trial 2026-10-12 verified direct links by name. Device: NOT RUN. **Blockers:** exact-RC device run.

### P-6 — Ordinary VPN use — mandatory
- **Reproduction:** browsing, representative video/large-asset sites, a sustained download and a controlled upload, non-Google IPv6 and IPv6-only destinations, DNS/ISP-IPv6/WebRTC leak checks, disconnect/reconnect, idle/resume, Wi-Fi↔cellular handover, client reboot, authorised server reboot — on all four transport/network cells.
- **Evidence:** server side: trial showed non-Google literal IPv6 reachable under `reject_google`; REALITY handshake passes after policy changes. Device: NOT RUN. **Blockers:** exact-RC device run.

### P-7 — Stability soak and account expiry (standalone mode) — mandatory
- **What it exercises:** the actual standalone configuration — operator-created accounts with static credentials, reconcile and watchdog timers, TLS and the services really installed. It does **not** require the provisioning agent or native 30-minute grants (not used in this mode, and the agent is intentionally disabled elsewhere).
- **Reproduction:** keep a client connected for the soak; across it observe disconnects, restarts, resource growth and desired/runtime drift; separately create a test account with a short `--expires-at`, confirm it is rejected at expiry and works again after `user set-expiry`.
- **Implementation:** `deploy/lib/soak-sampler.sh` (read-only, private CSV, no secrets) and `deploy/lib/soak_evaluate.py` with criteria fixed in advance (≥ 5400 s; 100 % availability; zero unplanned sing-box restarts; RSS ≤ +25 % and ≤ +64 MiB; fds ≤ +50 % and ≤ +64; every reconcile successful and ≥ 8 runs; watchdog active; every self-test passed and ≥ 1 ran; missing data fails). Evaluator and sampler are unit/fixture tested.
- **Evidence:** tooling verified locally; **no soak has been run** (no authorised host/time window). **Blockers:** run on the disposable host and on the owner's host after cutover; attach the CSV and verdict.

### P-8 — TLS certificate renewal and firewall restoration — mandatory
- **Reproduction:** `certbot renew --dry-run` through the supported hooks on the disposable hosts, with the TCP/80 pre/post firewall hooks and nginx ownership as installed; verify services healthy after.
- **Evidence:** fixtures green (renewal recovery, firewall hooks, nginx ownership); disposable-host renewal PENDING (R-19).

### P-9 — Backup/restore and rollback — mandatory
- **Reproduction:** `vpn-admin backup` then restore under the documented current-host deployment-setting policy (archived network policy is not applied silently); injected validation, reload and update failures roll back both desired and runtime state.
- **Evidence:** CLI/fixture tests green; disposable-host run PENDING (R-19).

### P-10 — Supported lifecycle operations and the hybrid transition — mandatory
- **Transitions (each runs the candidate's own checksum-verified updater, never a repair toward an older release):** normal v1.2.0 → candidate; v1.2.0 with development executables copied over → candidate; v1.2.0 hybrid with `reject_google` → candidate. Scripted and mock-tested in `deploy/almalinux/hybrid-upgrade-acceptance.sh`; it never uninstalls or reinstalls before the transition, fingerprints credentials, checks the installed executable against the published release independently of the manifest, repeats a repair from the new tree, and proves a downgrade is refused while `[egress]` is set.
- **Guard coverage:** the `[egress]` detector follows the production parser (header, spaced, quoted, single-quoted, trailing-comment, dotted-key and inline-table spellings), is exercised against a **real** v1.2.0 binary (which loads the file and ignores the policy) and the real current build, and runs update.sh's own pre-switch block, proving the live tree is byte-identical after a refusal.
- **Old-updater limit:** the v1.2.0 installed updater cannot be changed and has neither the egress guard nor identity recording. The supported transition therefore runs the **candidate's own updater from its checksum-verified source archive** (documented path); it records executable hashes at commit, so the identity line reads "recorded" immediately, and a repeated repair from the new tree is idempotent. **Never** run `--repair` or `--version v1.2.0` from the v1.2.0 tree on a host with an `[egress]` policy or hand-copied executables — it re-installs v1.2.0 and drops the policy.
- **Evidence:** automated only; the disposable-host runs are PENDING (R-19).

### P-11 — The owner's standalone cutover leaves the verified policy explicit and persistent — mandatory
- The policy is stored in `deployment.toml` (`[egress] ipv6_literal = "reject_google"`), is re-rendered by the single production renderer on every reconcile/update/repair, and survives them (tested). The owner's single VPS currently has it applied by hand with hand-copied executables. Cutover (needs authorisation): supported update to the accepted stable, then verify `vpn-admin egress show`, the "release identity" doctor line, REALITY/Hysteria2 handshakes, and a native playback test (Shorts **and** TikTok). Unrelated deployments keep the `allow` default.

### Deferred by owner (context only — not a gate, not a blocker)

| ID | Item | Status | Dated context kept for the future task |
|---|---|---|---|
| P-D1 | Arcana website host and journeys (registration, login, dashboard, billing, links) | **DEFERRED BY OWNER** | 2026-10-12, read-only: the name `vpn.xn----8sba1cvaif4e.com` resolved to the old de1 VPN exit and 80/443/8443 timed out from the investigation network, so it cannot be a Pages site as configured; deployed frontend/Functions/migration level unknown; `vpn-web` `origin/main` = `55277ed` passes its offline unit tests/lint/build. No DNS was changed; nothing was deployed or modified. |
| P-D2 | Website → provisioning agent → node journey, Supabase, Stripe, Telegram Mini App | **DEFERRED BY OWNER** | The agent stays disabled where the owner disabled it; the provisioning contract and `apps/provisioning-agent` tests are unchanged and still run. |
| P-D3 | Tamara app / custom client | **DEFERRED BY OWNER** | Not a test client for this release (Hiddify is). |

These are neither PASS nor FIXED, and an unresolved state in any of them does not
block this release.

## Corrections to earlier dispositions

- **R-3 was marked FIXED on a detector that only recognised the literal `[egress]` header.** Valid TOML spellings (`[ egress ]`, `["egress"]`, dotted keys, inline tables) escaped it. Re-fixed and re-verified (see P-10); the earlier "static grep" evidence is superseded.
- **R-6 covered only a read-only command.** A closed stdout/stderr during a *mutating* command could panic between "install configuration" and "reload" and skip rollback. Output now goes through a best-effort layer that cannot abort a transaction; tests close the streams during mutations, assert desired = runtime configuration, exactly one reload, unchanged credentials, a free lock, and that genuine failures still exit 1 with rollback. One residual: a one-time secret printed to a closed stdout (`user create`, `rotate-token`) is lost by definition; the account is still created whole and `rotate-token` mints a replacement.
- **R-2's wording over-claimed.** The identity line said "published stable release"; it now says what is true (the installer's recorded hashes match; not independent provenance; units/helpers not hashed), also checks the **running** executable, and no longer advises `--repair` toward an older release.
- **Test totals:** the earlier "66 fixtures" figure was wrong — the 67 passing items were 65 shell fixtures + the egress tools test + shellcheck. Final counts come from the committed qualification script and are recorded below.
- **A committed `.pyc`** (`deploy/lib/__pycache__/…`) was removed and Python caches are ignored.

## TikTok diagnostic procedure (owner device; nothing here changes production)

Record per run: iPhone/iOS, TikTok version, Hiddify and embedded core versions,
transport, network, server RC, policy, date. For each of REALITY/Wi-Fi,
REALITY/cellular, Hysteria2/Wi-Fi, Hysteria2/cellular, and once with the VPN off:
1. cold start; 2. feed metadata; 3. thumbnails; 4. video start; 5. 10 consecutive
videos; 6. 15 minutes of scrolling; 7. comments/profile; 8. login/account access;
9. TikTok web. Mark each step PASS/FAIL/slow.

Then, only for a failing cell, on an owner-authorised **test** window:
1. raise the node's sing-box log level temporarily (per-flow logging is off by
   default and must be restored — record the restore), capture the failing
   hostnames (SNI) and the transport (TCP/UDP) from restricted logs, never
   keep them in the repository;
2. compare with `?compat=tcp-only` as a **controlled experiment** (if TCP-only
   fixes it, UDP/QUIC is implicated; if it does not, it is disproved);
3. check DNS A/AAAA for those hosts from the node and from the client, IPv4 vs
   IPv6 reachability, MTU (`vpn-investigate.sh mtu`), and whether the exit IP is
   refused (fetch the same media URL from another vantage already available);
4. classify: DNS / TCP / destination QUIC-UDP / IPv4-IPv6 / MTU / routing /
   exit-IP or region restriction.

No domain is added to any rule, no UDP is blocked globally and no profile is
made default unless the specific failure was reproduced and the change was
verified to fix it without breaking other journeys. If the cause is an external
restriction of the exit's IP or region, that is reported as the blocker; the
product is then **not** declared fixed.

## Automated qualification (exact commit `1bba1ff3a273dcc5531481519aa5b1916f83392f`)

Produced by the committed `deploy/lib/qualify-local.sh`, which takes every gate's
exit status directly (no pipe can hide a failure), keeps each complete log with
its SHA-256, records the commit and tool versions, and reports a gate whose
prerequisite is missing as NOT-RUN (which fails the run), never as a pass.
Environment: Ubuntu 24.04 (WSL2) on a Windows 10 host, root, Rust 1.94.1,
sing-box 1.14.1 downloaded and **checksum-verified against
`deploy/lib/versions.env`**, aioquic 1.3.0, `jq` 1.7, IPv4/IPv6 loopback, outbound
network. Evidence directory (not in the repository): `release-evidence/v1.3.0-prep-1bba1ff3a273/`
(`SUMMARY.tsv`, `ENVIRONMENT.txt`, one log per gate).

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | PASS |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --no-deps` | PASS |
| `cargo test --locked --workspace --all-targets --no-fail-fast`, real-core interop **required** | PASS — **1029 tests, 34 suites, 0 failed, 0 ignored** (includes `two_hop_system` and the 88-assertion real-core lab) |
| every `deploy/lib/tests/*.sh` fixture (68), as root | PASS 68/68 |
| `test-egress-tools.py` | PASS |
| `shellcheck -S warning` over installer/uninstaller/`deploy/**` | PASS |
| Overall | **QUALIFICATION: PASS** (74 gates, 0 FAIL, 0 NOT-RUN) |

Also, outside the script: the downgrade-guard fixture was run against the **real**
v1.2.0 `vpn-admin` (built from its tag) and this tree's build — PASS (7/7 TOML
spellings refused; the old build loads the file and says nothing about the
policy); `check-release-version.sh` accepts `v1.3.0-rc.1` and `v1.3.0` and refuses
a wrong core; the GitHub CI graph on the earlier head of PR #152 was fully green
(the head has since moved; CI must be re-read on the final head).

**A failed first run, kept.** The first execution of this script on this commit
FAILED (`cargo-test-workspace`, exit 101: 904 passed, 1 failed) while I was
running the website's unit tests and a production build on the same machine. The
failing test is `provisioning-agent`'s
`report_queue::tests::enqueue_never_blocks_regardless_of_endpoint_health`, which
asserts a 500 ms wall-clock bound; it is load-sensitive and unrelated to any
change here (the agent crate is untouched). The rerun on an idle host passed
(above). The failed run's logs are retained
(`…-run1-loaded-host-FAILED/`). Qualification must therefore be run on an idle
host; the test's bound was **not** loosened to obtain a pass.

**Corrections to numbers quoted earlier:** the previous section said "66 fixtures"
and "34 suites, 1019 passed"; the tree then had 65 fixtures and 1019 tests, and
now has 68 fixtures and 1029 tests because tests were added. The ledger no longer
quotes figures that no committed artifact can reproduce.

**Website (DEFERRED BY OWNER — dated context only).** On 2026-10-12 the separate
`David610/vpn-web` repository at `origin/main` = `55277ed74afefedc585f55c8bcd90ac2149fc73e`
passed its offline unit tests (122 files / 1303 tests), lint (0 errors), build and
log-secret check. That says nothing about this release and is not a gate for it.

**Not run here (CI or external):** the GitHub CI graph on the final head, CodeQL,
cargo-audit, the release build in the pinned AlmaLinux 8.10 container with the
glibc ≤ 2.28 gate, SBOM/attestation generation, packaged-archive execution in
AlmaLinux 8/9 containers, `idn`/pcap-tool branches, and everything in R-18…R-20 and
the mandatory P-1…P-11 gates. Fragmented-TLS and ECH flows are not exercised by the lab (the policy
cannot match them: no SNI is recovered), and UDP rejection is observed only as
response absence, not as an ICMP error reaching a client.

## Final decision (as of 2026-10-12, before any RC)

This is a dated pre-RC snapshot. RELEASE.md freezes everything except `README.md`,
`docs/DEVICE_ACCEPTANCE_TESTS.md` and the stable acceptance record once an RC is
accepted, so the **current** status of the release is recorded only in those
files, never here.


**BLOCKED — not ready for owner release approval, for VPN-only reasons.**
Done and verified locally on commit `1bba1ff3a273` (later commits are docs-only
or comment-only): the repository work for R-2…R-10 and R-15, the hardened
downgrade guard (proved against a real v1.2.0 build), mutation-safe output
handling, running-executable identity, the hybrid-transition script, the soak
tooling, and a clean 74-gate local qualification. Website, billing, Supabase,
Telegram and Tamara are DEFERRED BY OWNER and are **not** part of this decision.

What still blocks the release:
1. **Merge of PR #152 and an RC** — the owner's authorisation (the final head's
   CI must be read first). No RC exists.
2. **Disposable-host lifecycle** (AlmaLinux 9 and Ubuntu 24.04), the three
   hybrid-transition scenarios, TLS renewal, backup/restore, rollback and the
   soak — need authorised hosts and verified host keys (R-19, P-7…P-10).
3. **Native iPhone matrix** through Hiddify for YouTube Shorts/long-form **and
   TikTok** (app and web), both transports, Wi-Fi and cellular, with subscription
   and direct-link import (R-20, P-1…P-6). TikTok's cause is UNKNOWN and may be an
   external restriction of the exit; no claim is made.
4. **Owner cutover** of the standalone server through the supported package
   (P-11), after the stable record exists.

Smallest concrete owner action: authorise merging PR #152 and creating
`v1.3.0-rc.1` (approval package in `docs/reviews/RELEASE_1_3_0_PREPARATION.md`
§2), and name two disposable hosts (AlmaLinux 9, Ubuntu 24.04) with their SSH host
keys. Everything else I can then run; only the iPhone matrix and the publish/
cutover approvals remain the owner's.
