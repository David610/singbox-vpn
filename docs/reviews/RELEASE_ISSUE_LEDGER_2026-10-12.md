# Release issue ledger — v1.3.0 preparation (2026-10-12)

Scope: the owner's single-VPS deployment (VLESS+REALITY, Hysteria2, HTTPS
subscription, native iPhone YouTube). Base: `main` at
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
| P-1…P-11 | Product gates: Shorts, long-form, TikTok app/web, website host, website journeys, provisioning, ordinary use, grant renewal/soak, lifecycle + hybrid transition, owner cutover | EXTERNAL / EVIDENCE | see "Product acceptance gates"; **none has native-device or live-site evidence** |

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

## Product acceptance gates (second pass)

The release target is the whole product, not one fix: the website, its customer
journeys, provisioning to the node, VPN usage, YouTube **and TikTok**, and the
lifecycle operations that must keep all of it working. Nothing below is removed
from scope; each item states what is verified, what is not, and what blocks it.
Evidence levels as above. "Candidate" means the exact immutable RC, which does
not exist yet (no tag has been created); every device/server cell therefore says
NOT RUN unless a dated trial entry is cited.

### Component manifest (what is known, and what is not)

| Component | Identity found | Deployed identity |
|---|---|---|
| singbox-vpn backend | PR #152, branch `release/1.3.0-prep`; proposed `v1.3.0-rc.1` (not tagged) | none yet — no RC exists |
| Latest published backend | `v1.2.0` | the owner's single VPS runs v1.2.0 plus hand-copied `main` (`ceb751c`) executables and `reject_google` (a hybrid, see P-10) |
| Website / frontend | repo `David610/vpn-web`, `origin/main` = `55277ed` (PR #82 merged 2026-10-08); Next.js static export (`arcana-web` 0.1.0) on Cloudflare Pages, Pages Functions, Supabase (Postgres, RLS), Stripe, Telegram bot | **UNKNOWN.** No GitHub deployment records; the repo's own runbook still calls the build an "offline-qualified release candidate"; the Pages project was reported blocked by Cloudflare (code 8000119) on 2026-10-08/09 and its production deployment state has not been read |
| Database | latest migration in `main`: `20261017000000_link_access_link_and_location_mode.sql` | **UNKNOWN** whether it is applied (it was written "apply before deploying the build"); the Supabase project is the production DB with test rows mixed in (owner decision 2026-10-06) |
| Provisioning agent | `apps/provisioning-agent` in this repo (v1.3.0 in this PR) | not installed on the single VPS; deliberately stopped and disabled on the earlier relay node (owner decision, preserved) |
| Client / core | Hiddify (iOS) with its embedded sing-box fork; native YouTube and TikTok apps | versions not recorded in any acceptance entry; must be recorded per run |
| sing-box | `1.14.1`, checksum-pinned in `deploy/lib/versions.env` | `1.14.1` on the single VPS |
| Website hostnames | `vpn.у-тамары.com` = `vpn.xn----8sba1cvaif4e.com` | see P-5; note it is a **different** name from the VPN host `vpn.утамары.com` = `vpn.xn--80aa9argf5d.com` used in the trial |

A singbox-vpn release does not deploy the website, the Pages Functions or the
database. Their accepted versions must be named in the stable acceptance record
and deployed through their own supported procedure; until then the product
cannot be called complete.

### P-1 — YouTube Shorts (native iPhone)
- **Reproduction:** native YouTube app through Hiddify; open `sAcElROnYIE`, then ≥ 10 consecutive Shorts.
- **Expected:** playback starts and continues on REALITY and Hysteria2, Wi-Fi and cellular.
- **Root cause:** per-family refusal by YouTube of the exit's IPv6 range (measured with browser emulation); the native retry behaviour after a proxy reject is UNKNOWN.
- **Implementation:** `[egress] ipv6_literal` (#151); explicit persisted policy for the owner's host at cutover (P-11); accepted-IP hairpin unchanged for exits whose IPv4 is refused.
- **Automated:** policy tests, 88-assertion real-core lab. **Server:** trial 2026-10-12 (browser emulation, 6/6 on IPv4/domain, IPv6 refused in ≈19 ms). **Device:** owner-reported "looks like it works" (provisional; version/transport/network not recorded).
- **Blockers:** exact-RC native run on the 4-cell matrix.

### P-2 — YouTube long-form
- **Reproduction:** a long video, seek forward/back, 15 minutes sustained, reconnect mid-play.
- **Expected:** no stall beyond normal buffering; plays on both transports and networks.
- **Root cause:** n/a (not reported failing). **Implementation:** none specific. **Automated/Server/Device:** none recorded for the candidate; the owner's 2026-10-11 report covered a normal video once.
- **Blockers:** exact-RC native run.

### P-3 — TikTok (native app)
- **Reproduction:** cold start, feed load, thumbnails, video start, 10 consecutive videos, 15 minutes of scrolling, comments and profile views, account access, on each transport/network; with a VPN-off baseline.
- **Expected:** all of the above work, or the failing hosts and cause are identified.
- **Root cause: UNKNOWN.** `docs/TIKTOK_INVESTIGATION.md` holds hypotheses only (QUIC/UDP preference, DNS, IPv6, routing/sniffing, exit-IP or region restriction) — all UNKNOWN; the baseline "TikTok fails" is user-reported with no device, client, network or transport record. The Google IPv6 policy is **not** assumed to apply and no TikTok domain list or rule is added.
- **Implementation:** none. Existing read-only server probe `vpn-investigate.sh tiktok` (root domains only). A TCP-only profile (`?compat=tcp-only`) is available as a controlled diagnostic experiment and is not a proposed default.
- **Automated/Server/Device:** NOT RUN.
- **Blockers (device, owner):** run the matrix in "TikTok diagnostic procedure" below; a deliberate service refusal of the exit IP, if that is the answer, is an external restriction to be reported, not fixed in code.

### P-4 — TikTok web
- **Reproduction:** tiktok.com in a mobile and a desktop browser through the VPN: home feed, one video, search. **Expected:** loads and plays; **Root cause:** UNKNOWN. **Evidence:** NOT RUN. **Blockers:** needs a browser on the accepted network; HTTP 200 on the root page is not success — a video must play.

### P-5 — The website host `vpn.у-тамары.com`
- **Reproduction (read-only, from the investigation machine, 2026-10-12):** `vpn.xn----8sba1cvaif4e.com` resolves (Cloudflare nameservers, DNS-only) to **91.244.71.165 — the earlier de1 VPN exit**, not to a Pages deployment. TCP 80, 443 and 8443 to it all timed out from this network; the VPN host `vpn.xn--80aa9argf5d.com` (157.173.27.46) answered on 443 and 8443 from the same network.
- **Expected:** the customer website answers on its public hostname with valid TLS.
- **Root cause: UNKNOWN, with a hard architectural conflict.** On a node, port 443 belongs to the REALITY listener (and UDP 443 to Hysteria2); a static Pages site cannot be served from that address. Either the hostname is not the website's real address (the site may live at an apex/other name or a `*.pages.dev` URL), or the record is stale/mis-pointed. Whether de1 is down, firewalled or blocked from this network cannot be told from one vantage point. This is **not** a website outage finding and **not** a fix: no DNS was changed.
- **Implementation:** none possible without the owner naming the real deployment (Pages project/custom domain) and authorising a probe from a second network. The IDN round-trip (`у-тамары` ↔ `xn----8sba1cvaif4e`) is consistent; the two names `у-тамары.com` and `утамары.com` must not be conflated in configuration.
- **Evidence:** DNS only. **Blockers:** owner confirms the intended production hostname for the website vs the VPN; authorised public smoke after any cutover.

### P-6 — Website customer journeys
- **Inventory (from the repo, not from a running site):** landing/navigation; registration and login (Supabase auth) with session handling; `/account` dashboard; `/account/devices`, `/account/links` (+ new, detail), `/account/subscription`, `/account/billing` (Stripe checkout and portal), `/account/settings`, `/account/help`; Telegram Mini App with Telegram-authenticated routes; admin console (AAL2/RBAC); subscription retrieval `/sub/<token>`; node bootstrap/agent API (`functions/v1`).
- **Expected:** each journey works end to end in a sandbox: verification email, login/logout/session renewal, password reset, sandbox checkout → entitlement → provisioning, QR/import presentation, subscription refresh, expiry/disable/reactivate, clear recoverable errors when API, payment provider or node is unavailable. Negative cases: no cross-account access to links; webhook/job replays create no duplicate users or credentials; client-side checkout success grants nothing by itself; expired access rejected; sensitive responses uncached and unlogged; a job is not "succeeded" before the node operation succeeded; delivered credentials equal the node's rendered ones.
- **Root cause / Implementation:** n/a — nothing has been observed failing; nothing was changed in the website. The repo already ships real-stack e2e suites (`scripts/e2e/portal.mjs`, `admin.mjs`, `telegram.mjs`, `docs/runbooks/LOCAL_E2E.md`).
- **Evidence:** NOT RUN. This pass had no sandbox Supabase/Stripe project, no approved test inbox, and the only database known is the production one (rules in the owner's note: prefix test rows, never touch existing rows, treat Stripe/RLS/rotation tests as production-data operations).
- **Blockers:** owner supplies/approves a staging or sandbox target (or explicitly authorises prefixed test rows in production) and a test inbox; then run the e2e suites and the manual journeys above, record versions per the manifest, and repeat a public smoke after cutover.

### P-7 — Website → agent → node provisioning
- **Reproduction:** sandbox purchase → control-plane job → provisioning agent claims it → node renders the user → client receives credentials that authenticate.
- **Expected:** at-most-once effect per job, credentials identical to the node's, job reports success only after the node operation, grants renew before expiry.
- **Facts established:** operator-created users (`vpn user create`) carry static legacy credentials with no lease; native grants (30-minute leases) exist only for authorisation-backed principals and are renewed by the agent/lease pool; the agent is a separate service (`vpn-provisioning-agent`), not installed on the single VPS, and deliberately disabled on the earlier relay while its control plane was unreachable.
- **Root cause:** n/a. **Implementation:** none. **Evidence:** NOT RUN (needs a live control plane, P-6). **Blockers:** if the journey requires the agent on a node, the exact change (install/enable on named host, its control-plane URL, claim-token handling) is prepared for approval — it is **not** enabled and manual user creation is **not** substituted for it.

### P-8 — Ordinary VPN use
- **Reproduction:** browsing, representative video/large-asset sites, a sustained download and a controlled upload, non-Google IPv6 and IPv6-only destinations, DNS/ISP-IPv6/WebRTC leak checks, subscription refresh, disconnect/reconnect, idle/resume, Wi-Fi↔cellular handover, client reboot, authorised server reboot — on all four transport/network cells.
- **Evidence:** server side: trial showed non-Google literal IPv6 reachable under `reject_google`; REALITY handshake passes after policy changes. Device: NOT RUN. **Blockers:** exact-RC device run.

### P-9 — Credential/grant renewal and expiry
- **Reproduction:** keep a client connected across ≥ 3 grant lifetimes and ≥ 8 reconcile cycles; observe disconnects, renewal errors, restarts, resource growth.
- **Implementation:** `deploy/lib/soak-sampler.sh` (read-only, private CSV, no secrets) and `deploy/lib/soak_evaluate.py` with criteria fixed in advance (≥ 5400 s; 100 % availability; zero unplanned sing-box restarts; RSS ≤ +25 % and ≤ +64 MiB; fds ≤ +50 % and ≤ +64; every reconcile successful and ≥ 8 runs; watchdog active; every self-test passed and ≥ 1 ran; missing data fails). Evaluator and sampler are unit/fixture tested.
- **Evidence:** tooling verified locally; **no soak has been run** (no authorised host/time window). **Blockers:** run on the disposable host and on the owner's host after cutover, attach the CSV and verdict.

### P-10 — Supported lifecycle operations and the hybrid transition
- **Transitions (all use the verified updater of the host's own installed tree, never a generic repair toward an older release):** normal v1.2.0 → candidate; v1.2.0 with development executables copied over → candidate; v1.2.0 hybrid with `reject_google` → candidate. Scripted and mock-tested in `deploy/almalinux/hybrid-upgrade-acceptance.sh`; it never uninstalls or reinstalls before the transition, fingerprints credentials, checks the installed executable against the published release independently of the manifest, runs `--repair` from the new tree only to record identity, and proves a downgrade is refused while `[egress]` is set.
- **Guard coverage:** the `[egress]` detector now follows the production parser (header, spaced, quoted, single-quoted, trailing-comment, dotted-key and inline-table spellings), is exercised against a **real** v1.2.0 binary (which loads the file and ignores the policy) and the real current build, and runs update.sh's own pre-switch block, proving the live tree is byte-identical after a refusal.
- **Old-updater limit:** the v1.2.0 installed updater cannot be changed and has neither the egress guard nor identity recording. The supported transition therefore runs the **candidate's own updater from its checksum-verified source archive** (documented path); it records executable hashes at commit, so the identity line reads "recorded" immediately, and a repeated repair from the new tree is idempotent. **Never** run `--repair` or `--version v1.2.0` from the v1.2.0 tree on a host with an `[egress]` policy or hand-copied executables — it re-installs v1.2.0 and drops the policy.
- **Evidence:** automated only; the disposable-host runs are PENDING (R-19).

### P-11 — The owner's cutover leaves the verified policy explicit and persistent
- The policy is stored in `deployment.toml` (`[egress] ipv6_literal = "reject_google"`), is re-rendered by the single production renderer on every reconcile/update/repair, and survives them (tested). The owner's single VPS currently has it applied by hand with hand-copied executables. Cutover (needs authorisation): supported update to the accepted stable, then verify `vpn-admin egress show`, the "release identity" doctor line, REALITY/Hysteria2 handshakes, and a native playback test. Unrelated deployments keep the `allow` default.

## Corrections to earlier dispositions

- **R-3 was marked FIXED on a detector that only recognised the literal `[egress]` header.** Valid TOML spellings (`[ egress ]`, `["egress"]`, dotted keys, inline tables) escaped it. Re-fixed and re-verified (see P-10); the earlier "static grep" evidence is superseded.
- **R-6 covered only a read-only command.** A closed stdout/stderr during a *mutating* command could panic between "install configuration" and "reload" and skip rollback. Output now goes through a best-effort layer that cannot abort a transaction; tests close the streams during mutations, assert desired = runtime configuration, exactly one reload, unchanged credentials, a free lock, and that genuine failures still exit 1 with rollback. One residual: a one-time secret printed to a closed stdout (`user create`, `rotate-token`) is lost by definition; the account is still created whole and `rotate-token` mints a replacement.
- **R-2's wording over-claimed.** The identity line said "published stable release"; it now says what is true (the installer's recorded hashes match; not independent provenance; units/helpers not hashed), also checks the **running** executable, and no longer advises `--repair` toward an older release.
- **Test totals:** the earlier "66 fixtures" figure was wrong — the 67 passing items were 65 shell fixtures + the egress tools test + shellcheck. Final counts are recorded with the qualification run below.
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

## Automated qualification (final tree of this PR)

Environment: Ubuntu 24.04 (WSL2) on a Windows 10 host, Rust 1.94.1 (pinned),
sing-box 1.14.1 downloaded and **checksum-verified against
`deploy/lib/versions.env`**, aioquic 1.3.0, `jq` 1.7. The privileged tier ran as
root (CAP_NET_ADMIN, `/run` and ownership fixtures available), which the earlier
cloud run lacked. IPv4 and IPv6 loopback available; outbound network available.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | PASS |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --no-deps` | PASS |
| `cargo test --locked --workspace --all-targets --no-fail-fast` (root, `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`, pinned core) | PASS — 34 suites, **1019 passed, 0 failed, 0 ignored**; includes `two_hop_system` (22) and the real-core lab |
| Real-core egress-family lab (`egress_family_interop`, unprivileged) | PASS — **88 assertions** (REALITY + Hysteria2 × allow/reject_google/reject_all) |
| `deploy/lib/tests/*.sh` (66 fixtures) + `test-egress-tools.py` (7) + `shellcheck -S warning` | PASS — 67 of 67, run as root on an exported (non-git) tree |
| `check-release-version.sh v1.3.0-rc.1` and `v1.3.0` | PASS; a wrong core (`v1.2.1-rc.1`) is refused |

Failures seen on the way and what they were: (1) my two broken-pipe tests
launched the binary outside the repository's host-isolation helper — caught by
`every_binary_invocation_is_guarded`, fixed; (2) the lifecycle-harness fixture
fails if its tree is an *unborn* git repository (`git rev-parse` prints `HEAD`
and exits non-zero, so the harness derives a two-line branch) — a property of the
scratch checkout I used, not of a release (CI has real commits; archives have no
`.git`); (3) `test-uninstall-idempotency` failed once while the cargo suite was
creating system accounts concurrently — it passes alone; (4) two benchmark
fixtures need `jq` (a prerequisite, now installed in the test VM).

**Not run here (CI or external):** the GitHub CI graph, CodeQL, cargo-audit,
the release build in the pinned AlmaLinux 8.10 container with the glibc ≤ 2.28
gate, SBOM/attestation generation, packaged-archive execution in AlmaLinux 8/9
containers (`release-container-smoke`, `os-matrix`), `idn`/pcap-tool branches,
and everything in R-18…R-20. A passing local run is not release evidence for
those. Fragmented-TLS and ECH flows are not exercised by the lab (the policy
cannot match them: no SNI is recovered), and UDP rejection is observed only as
response absence, not as an ICMP error reaching a client.

## Final decision

**BLOCKED** for release approval until R-19 and R-20 have PASS evidence on the
exact RC (and the GitHub CI graph, which has not yet run on this branch, is
green). The repository work for R-2…R-10 and R-15 is code-verified locally.
