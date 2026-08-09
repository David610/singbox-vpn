# PRODUCTION_HARDENING_PLAN.md

Fix plan for the Hiddify/VLESS+REALITY/Hysteria2 compatibility stack,
written against the actual code (see audit notes inline — this is not a
generic checklist). Scope: make the AlmaLinux + sing-box + Hiddify
deployment safe, reproducible, and likely to work on a real VPS. No new
protocols, no GUI, no rewrite of REALITY/Hysteria2/QUIC.

Status column: `[x]` implemented+tested this session, `[~]` implemented
but not externally verified (no real VPS/AlmaLinux/Android available in
this sandbox), `[ ]` documented only / deferred with reason.

---

## 1. Filesystem ownership (`sing-box` cannot read its own secrets)

- **Issue**: `install.sh` puts `reality/` and `hysteria/` under
  `root:vpn-subscription` / `root:root` respectively. `sing-box.service`
  runs as `User=sing-box`. `sing-box` is never added to either group, so
  it cannot read `reality/private.key` or `hysteria/{cert,key}.pem` —
  the service as shipped cannot actually start against a real REALITY
  config.
- **Root cause**: `create_directories()` (`install.sh`) chose
  `vpn-subscription`/`root` group ownership for directories that
  `sing-box`, not `vpn-subscription`, needs to read.
- **Impact**: sing-box fails to start (`ExecStartPre=sing-box check`
  fails with a permission error) — the deployment is dead on arrival for
  any config that isn't a placeholder.
- **Files**: `deploy/almalinux/install.sh`.
- **Fix**: `reality/` and `hysteria/` become `root:sing-box`, mode
  `0750`; `reality/private.key` stays `root:root 0600` (sing-box runs as
  `sing-box`, not root, so make `private.key` `root:sing-box 0640`
  instead — sing-box must read it); `reality/public.key`,
  `short_id.txt` become `root:vpn-subscription 0640` (only the
  subscription service needs those, never the sing-box process);
  `hysteria/{cert,key}.pem` become `root:sing-box 0640`. `users/` stays
  `root:vpn-subscription`. `sing-box` group membership is not needed —
  every file sing-box must read is directly group-owned `sing-box`.
- **Test**: `deploy/almalinux/acceptance-test.sh` §"file ownership"
  runs `sudo -u sing-box test -r <path>` / `sudo -u vpn-subscription
  test -r <path>` / `test ! -r <path>` for every secret file per the
  Definition-of-Done matrix in this plan.

## 2. `config.json` written with no explicit mode (defaults to umask, often 0644)

- **Issue**: `apply_config_atomically` (`crates/compat-config/src/server.rs`)
  writes the temp file via plain `std::fs::write`, with no `chmod`. The
  live file, the `.bak` backup, and the rollback copy in `update.sh` all
  inherit whatever the process umask happens to be — `root`'s default
  umask (0022) yields **0644**, world-readable, exposing the REALITY
  private key + every VLESS UUID + every Hysteria2 password to any local
  user.
- **Root cause**: only `store.rs` (`users.json`) set an explicit mode;
  `server.rs` (`config.json`, which is *more* sensitive — it contains
  the REALITY private key in cleartext) never did.
- **Impact**: local privilege escalation / credential theft by any
  unprivileged local account.
- **Files**: `crates/compat-config/src/server.rs`, `deploy/almalinux/update.sh`.
- **Fix**: `apply_config_atomically` opens the temp file with
  `OpenOptions::mode(0o640)`, `fsync`s it before validation, sets the
  same mode on the `.bak` copy, and `fsync`s the parent directory after
  the rename. `update.sh`'s rollback path now uses `cp -a` (preserves
  mode from the backup, which was itself written 0640) instead of
  `install -m 0644`.
- **Test**: new `#[cfg(unix)]` test in `server.rs` asserting the
  applied file (and its `.bak`) have mode `0640`; new
  `deploy/almalinux/acceptance-test.sh` check `stat -c %a config.json`
  == 640.

## 3. Config writes not crash-safe end-to-end

- **Issue**: sequence was write→validate→backup(copy)→rename, no
  `fsync` anywhere — a crash between rename and the next read could
  leave the directory entry pointing at data not yet flushed to disk on
  some filesystems/power-loss scenarios.
- **Fix**: `fsync` the temp file after write (before validate — no
  point validating unflushed data on some exotic setups, but more
  importantly this guarantees the bytes are durable before they become
  "the config"), and `fsync` the parent directory handle after the
  rename (this is what actually makes the rename itself durable on
  Linux — renames are not implicitly fsynced). Not over-engineered:
  no O_DIRECT, no double-buffering, no WAL — a single fsync-on-write +
  fsync-on-dir-after-rename is the standard "atomic file replace" recipe
  and sing-box's own config directory is small enough that this is
  cheap.
- **Files**: `crates/compat-config/src/server.rs`.
- **Test**: existing `apply_atomically_*` tests still pass (fsync is a
  correctness-neutral addition on the happy/failure paths already
  covered); a crash-mid-write scenario can't be deterministically
  tested without fault injection, so this is `[~]` for the "provably
  survives a real crash" claim — the fsync calls are real, but no
  power-loss test harness exists.

## 4. User mutations never reload sing-box — disable/remove don't take effect

- **Issue**: `vpn-admin user disable/enable/remove/create` in
  `apps/admin/src/main.rs` call `regenerate_singbox_config`, which
  writes+validates `config.json`, but **never signals sing-box**. The
  running process keeps using its already-loaded config in memory until
  something else restarts it. `vpn-admin user disable USER` currently
  prints success while the disabled user's credentials are still
  accepted by the live server.
- **Root cause**: no reload/restart call exists anywhere in
  `apps/admin`; the only place that ever calls
  `systemctl reload-or-restart sing-box` is `deploy/almalinux/update.sh`
  (a separate, manually-invoked script), not any `user` subcommand.
- **Impact**: this is the single most serious correctness bug in the
  stack — access revocation is silently a no-op from the operator's
  point of view.
- **Files**: `apps/admin/src/main.rs`.
- **Fix**: new `apps/admin/src/service.rs` module,
  `CompatibilityServiceManager`, wrapping
  `systemctl reload-or-restart sing-box` (chosen over `reload` alone:
  sing-box 1.13's reload story for in-place config swap is not
  guaranteed reliable for all field changes across versions — the
  pinned-version docs describe SIGHUP/`systemctl reload` support but
  `reload-or-restart` degrades safely to a full restart, which is
  always correct, at the cost of a short connection drop that's
  acceptable for a single-VPS deployment) plus
  `systemctl is-active --quiet sing-box` health verification.
  `regenerate_singbox_config` now: renders → validates (existing) →
  applies atomically (existing) → **reloads** → **verifies health**; on
  reload/health failure it restores the previous `config.json.bak`,
  reloads again, and returns an error — the CLI command then reports
  failure, not success. This satisfies §6 and §7 together.
  Reload is skipped (with a clear warning, not silently) when running
  outside systemd (e.g. local dev / tests) — detected via `systemctl`
  binary presence, so unit tests aren't broken.
- **Test**: `apps/admin/tests/` gets a new test exercising
  `CompatibilityServiceManager` against a fake `systemctl`-shaped script
  on `$PATH` (integration test, no real systemd needed) proving: reload
  is invoked after config apply, and a reload failure triggers restore
  of the previous config. Real end-to-end (`disable` -> real sing-box
  rejects old creds) is `[~]` — needs a real systemd host.

## 5. Hysteria2 can start without TLS material — installer doesn't verify

- **Issue**: nothing in `install.sh` checks that
  `$STATE_DIR/hysteria/{cert,key}.pem` exist before
  `enable_and_start_services`. `sing-box.service`'s own
  `ExecStartPre=sing-box check` will catch a missing file and refuse to
  start, but `install.sh` still prints "Install complete." regardless,
  because the `render-config` step is itself masked with `|| true`
  (see #7 below) and nothing downstream checks service state.
- **Fix (chosen: "Preferred" option — explicit failure, no auto-ACME)**:
  auto-ACME was considered (`certbot`/`acme.sh`) but rejected for this
  pass: Hysteria2's cert is served directly by sing-box (not by a
  webserver ACME clients assume), so wiring real ACME (HTTP-01 needs
  port 80 free, DNS-01 needs a provider API) is a second production
  subsystem, not a one-line addition, and the task explicitly says "do
  not implement ACME yourself" and "select one simple approach" — for a
  single-VPS boring-V1 deployment, requiring the operator to run
  `certbot certonly --standalone` once (documented, exact command
  given) and pointing sing-box at the resulting files is simpler and
  has fewer moving parts than scripting a second ACME client
  integration blind. `install.sh` now has a `[6] certificates` stage
  that **fails installation** with the exact missing-file paths and the
  exact `certbot`/`openssl` commands to fix it, rather than silently
  starting a broken Hysteria2 listener.
- **Files**: `deploy/almalinux/install.sh`, `docs/ALMALINUX_DEPLOYMENT.md`.
- **Test**: `bash -n`/shellcheck clean; acceptance-test.sh validates
  `openssl x509 -checkend` on the installed cert.

## 6. Subscription-service TLS vs Hysteria2 TLS conflated in docs

- **Issue**: docs referred to "TLS" generically enough that a reader
  could conflate the subscription reverse-proxy's HTTPS cert
  (`sub.example.com`, terminated by nginx) with the Hysteria2 TLS cert
  (`vpn.example.com`, consumed directly by sing-box, no reverse proxy
  involved).
- **Fix**: `docs/ALMALINUX_DEPLOYMENT.md` now has an explicit "Two
  independent TLS certificates" section naming both hostnames and
  stating they may reuse one cert/domain only if the operator
  deliberately chooses that, never assumed.
- **Files**: `docs/ALMALINUX_DEPLOYMENT.md`.

## 7. No automated reverse proxy — manual nginx step

- **Issue**: `install.sh` told the operator to add nginx manually after
  install; subscription HTTPS was never actually reachable out of the
  box.
- **Fix (Option A, as preferred)**: `install.sh` gains an `[9]
  nginx/subscription HTTPS` stage: installs `nginx`, writes
  `/etc/nginx/conf.d/vpn-subscription.conf` (proxies
  `https://sub.example.com:8443` → `http://127.0.0.1:9100`, `nginx -t`
  validated before reload, `no-store` on `/sub/`, its own
  request-rate-limit zone for `/sub/`), and documents that the operator
  must supply/renew the subscription cert (certbot webroot/standalone —
  same reasoning as #5: not reinventing ACME).
- **Files**: `deploy/almalinux/install.sh`,
  `deploy/almalinux/templates/nginx-vpn-subscription.conf.template`,
  `docs/ALMALINUX_DEPLOYMENT.md`.
- **Test**: `nginx -t` run in install.sh before reload (fails loudly,
  doesn't mask); acceptance-test.sh curls the HTTPS endpoint.

## 8. Subscription rate limiting collapses to global once behind nginx

- **Issue**: `services/subscription` keys its token bucket off
  `ConnectInfo`'s raw TCP peer address. Once nginx proxies to
  `127.0.0.1:9100`, every request's peer address is nginx's loopback
  socket — the per-client bucket becomes one shared bucket for every
  real client, so one abusive subscriber can exhaust it for everyone.
- **Fix (chosen: rate-limit at nginx, per the task's stated preference
  for a single VPS)**: the generated nginx vhost adds a
  `limit_req_zone $binary_remote_addr zone=sub:10m rate=<n>r/s` +
  `limit_req zone=sub burst=... nodelay` on the `/sub/` location, using
  nginx's own view of the real client IP (it terminates the actual
  client TCP connection, so this is trustworthy without needing
  `X-Forwarded-For` at all). The Rust service's in-process limiter is
  left in place unchanged as defense-in-depth against direct-to-9100
  access (which firewalld/`IPAddressAllow` already block from the
  public network) — it is **not** modified to trust `X-Forwarded-For`,
  since that header is still not read anywhere in `lib.rs` and adding
  header-trust would introduce exactly the spoofing risk the task warns
  against. This is documented as a deliberate trust-boundary choice.
- **Files**: `deploy/almalinux/templates/nginx-vpn-subscription.conf.template`,
  `docs/ALMALINUX_DEPLOYMENT.md`.
- **Test**: nginx config includes the limit directives; documented
  manual verification (`ab`/`curl` loop) in acceptance-test.sh comments
  since load-testing a live nginx isn't meaningful in unit tests.

## 9. Hysteria2 masquerade not configured

- **Issue**: `render_singbox_server_config` never sets `masquerade` on
  the `hysteria2` inbound — flagged already in
  `docs/COMPATIBILITY_SECURITY_REVIEW.md` as a known gap.
- **Fix**: per current sing-box 1.13.x docs
  (`https://sing-box.sagernet.org/configuration/inbound/hysteria2/#masquerade`),
  the inbound gains a static-file masquerade default:
  `{"type": "file", "path": "/etc/vpn/compat/hysteria/masquerade"}`
  serving an innocuous placeholder directory (installer creates a
  minimal static HTML file there), so unauthenticated/invalid Hysteria2
  connections receive a plausible HTTP response instead of a
  distinctive failure. **What this does and does not protect against**
  documented inline: it makes a passive/active *unauthenticated probe*
  see ordinary-looking HTTP instead of an obvious auth-reject signature;
  it does **not** hide the fact that QUIC/UDP:443 is open, does not
  defeat active fingerprinting of the QUIC handshake itself, and is not
  a substitute for REALITY-style full protocol mimicry (Hysteria2 has no
  equivalent of REALITY's live-relay disguise).
- **Files**: `crates/compat-config/src/server.rs`,
  `deploy/almalinux/install.sh`, `docs/COMPATIBILITY_SECURITY_REVIEW.md`.
- **Test**: unit test asserting rendered config contains
  `masquerade.type == "file"`.

## 10. REALITY/Hysteria2 field correctness vs upstream

- Reviewed against `https://sing-box.sagernet.org/configuration/` for
  1.13.x: `flow`, `server_name`, `handshake.server`/`server_port`,
  `private_key`, `short_id` (array server-side, singular client-side),
  client `public_key`, `fingerprint` all match current schema per the
  audit (§1 of the audit notes above) and `docs/COMPATIBILITY_VERSIONS.md`'s
  existing citations. No renderer bug found beyond the missing
  `masquerade` key (#9). This plan does **not** claim independent
  verification against a live `sing-box check` run in this sandbox
  (no network-installable binary here) — that's covered by #11 (CI).

## 11. No real sing-box validation in CI

- **Issue**: CI only compiles/tests Rust; `sing-box check` never runs
  against generated config anywhere in the pipeline.
- **Fix**: new `ci.yml` job `singbox-validate`: downloads the pinned
  `sing-box 1.13.14` release tarball (same URL pattern as
  `install.sh`), verifies against upstream `checksums.txt` when
  published (same logic as the installer, not duplicated logic that can
  drift — both scripts hash-log if none is published), builds
  `vpn-admin`, runs `vpn-admin init` + `render-config` against a
  disposable temp state dir with a self-signed test cert for Hysteria2,
  then `sing-box check -c config.json`. Failure is blocking (no `|| true`).
- **Files**: `.github/workflows/ci.yml`.
- **Test**: the job itself *is* the test; running it in this sandbox is
  `[~]` (no outbound network to GitHub releases from here) — CI will
  exercise it on the next real push.

## 12. `cargo audit || true`

- **Issue**: `ci.yml`'s audit job always exits 0.
- **Fix**: `cargo audit` runs without the `|| true` suppression. No
  current advisory required a scoped ignore at audit time in this
  session (see test results below) — if one appears later, the fix is a
  narrowly-scoped `--ignore RUSTSEC-XXXX-YYYY` with a comment citing the
  advisory and why it's not fixable yet, not a blanket suppression.
- **Files**: `.github/workflows/ci.yml`.
- **Test**: `cargo audit` run locally this session — see Test Results.

## 13. sing-box license mis-stated as MIT

- **Issue**: `docs/COMPATIBILITY_VERSIONS.md` claimed "MIT-licensed".
  sing-box's actual upstream license (SagerNet/sing-box, `LICENSE` file
  at the pinned `v1.13.14` tag) is **GPL-3.0-only** for the open-source
  core.
- **Impact**: incorrect legal claim in project docs; no code-level
  impact since this project never links against sing-box (it's invoked
  as an external subprocess/binary, downloaded as a release artifact —
  arm's-length process invocation, not a compiled-in dependency), but
  the license text/notice should still be preserved when redistributing
  the binary.
- **Fix**: `docs/COMPATIBILITY_VERSIONS.md` corrected to GPL-3.0-only
  with a citation, plus a new paragraph clarifying: this Rust project's
  own code is under its own license (see root `Cargo.toml` /
  workspace license), sing-box is a separate, unmodified upstream
  binary obtained via its official release artifacts and invoked as a
  subprocess (not statically or dynamically linked into any Rust
  binary here), and `install.sh` is updated to fetch and keep sing-box's
  `LICENSE`/`NOTICE` file alongside the installed binary so redistribution
  obligations are met if the installed system image is itself
  redistributed. No legal conclusion beyond what the license text
  itself states is asserted.
- **Files**: `docs/COMPATIBILITY_VERSIONS.md`, `README.md`,
  `deploy/almalinux/install.sh`.

## 14. User IDs are only 32 bits (`user_<8 hex>`)

- **Issue**: `apps/admin/src/main.rs` `cmd_user_create` builds the id as
  `format!("user_{}", credentials::generate_short_id())` —
  `generate_short_id` is 4 random bytes (32 bits), *and* is the same
  function used for the REALITY `short_id` (a general-purpose ID
  generator reused for an unrelated purpose, against the task's
  explicit guidance not to reuse REALITY short IDs as user IDs).
- **Fix**: new `credentials::generate_user_id()` (128-bit UUIDv4-based,
  `user_<uuid>`), used by `cmd_user_create`, plus explicit collision
  detection against the existing store before insert (belt-and-suspenders
  even at 128 bits).
- **Files**: `crates/compat-config/src/credentials.rs`,
  `apps/admin/src/main.rs`.
- **Test**: new unit tests — id format, no collision across 10k
  generations, collision-detection path exercised with a forced
  duplicate.

## 15. No credential rotation for VLESS UUID / Hysteria2 password

- **Issue**: only `rotate-token` existed; `docs/COMPATIBILITY_SECURITY_REVIEW.md`
  already flagged this gap.
- **Fix**: new subcommands `user rotate-vless`, `user rotate-hysteria`,
  `user rotate-credentials` (both). Each: generate → save → render →
  validate → reload → verify, with the same rollback-on-failure path as
  #4. REALITY server keys are never touched by any rotate command
  (only `vpn-admin init --rotate` can do that, unchanged, deliberately
  separate and more disruptive).
- **Files**: `apps/admin/src/main.rs`.
- **Test**: CLI integration tests in `apps/admin/tests/`.

## 16. `user subscription` UX / accidental secret exposure

- **Issue**: mostly fine already (task's own audit vs. actual code:
  it does *not* print UUID/password, matching the spec) but didn't
  print expiry or the public subscription host, and the "cannot be
  recovered" messaging can be tightened.
- **Fix**: output now includes `id`, `enabled`, `expiry`, and the
  public subscription host, plus the exact `rotate-token` command to
  run — still never prints UUID/password/token.
- **Files**: `apps/admin/src/main.rs`.

## 17. Subscription response caching / headers

- **Issue**: no `Cache-Control`/`Pragma`/`X-Content-Type-Options`
  headers on `/sub/*` responses; nginx (new, #7) also needs explicit
  no-cache for the same path.
- **Fix**: `services/subscription/src/lib.rs` adds
  `Cache-Control: no-store`, `Pragma: no-cache`,
  `X-Content-Type-Options: nosniff` to every `/sub/*` response (success
  and error paths). nginx template adds `proxy_no_cache 1;
  proxy_cache_bypass 1; add_header Cache-Control "no-store" always;` on
  the `/sub/` location.
- **Files**: `services/subscription/src/lib.rs`,
  `deploy/almalinux/templates/nginx-vpn-subscription.conf.template`.
- **Test**: new axum test asserting the header is present on both the
  200 and 404 paths.

## 18. Logging review

- **Issue**: audited every `tracing::`/`log::` call across
  `services/subscription`, `apps/admin`, `crates/compat-config` for
  token/password/uuid/private_key content.
- **Finding**: no log statement in the current code interpolates a raw
  token, password, or private key — `lib.rs:120` logs `user.id` +
  requested format only, matching the task's allowed fields (user ID,
  status, error category). No change required beyond keeping this
  invariant; added a doc comment at the one call site plus a redaction
  regression test (`grep`-based CI-friendly check job, see #19).
- **Files**: none functionally; `docs/COMPATIBILITY_SECURITY_REVIEW.md` updated to record this as reviewed rather than unreviewed.

## 19. Health checks — deeper than "port open"

- **Issue**: `health-check.sh` already does process/config/listener/
  subscription-endpoint checks (levels 1-4 from the task's list) — the
  audit shows this is more complete than the task's framing assumed.
  Level 5 (real transport test through VLESS+REALITY / Hysteria2 to a
  controlled destination) does not exist.
- **Fix**: keep `vpn-health-check` (levels 1-4, fast, safe to run
  constantly) as-is aside from minor additions (nginx reachability,
  masquerade file presence); add a separate, explicitly-opt-in
  `deploy/almalinux/acceptance-test.sh --full` extension point for
  level 5 (documented as requiring a real sing-box client config +
  controlled destination — not safely automatable against arbitrary
  production traffic, so it's scripted but not wired into routine
  health checks).
- **Files**: `deploy/almalinux/health-check.sh`,
  `deploy/almalinux/acceptance-test.sh` (new).

## 20. Network failure-independence tests

- **Status**: `[ ]` not executed this session — same constraint already
  documented in `TASKS.md`/`docs/COMPATIBILITY_SECURITY_REVIEW.md`:
  this sandbox has no `iproute2`/root network-namespace capability
  (re-verified: `which ip tc` fail). Not re-claimed as fixed. The
  acceptance-test.sh script (new) includes a clearly-marked,
  documented-but-unexecuted section with the exact `nft`/`tc netns`
  commands and mandatory cleanup traps, for a privileged runner to
  execute.

## 21. AlmaLinux deployment test

- **Fix**: new `deploy/almalinux/acceptance-test.sh` — runs on a real
  host (container smoke-test acceptable for package/script syntax,
  explicitly distinguished in the script's own header comment from a
  full VM test that can check SELinux-enforcing/firewalld/systemd/
  low-port-capability behavior, which a plain container cannot
  provide). Not executed against a live AlmaLinux 9 host or VM in this
  session (`[ ]`) — no such host is available here; the script is
  written, `bash -n`/shellcheck-clean, and ready to run.

## 22. Installer stage structure / `|| true` on critical steps

- **Fix**: `install.sh` restructured into the 15 explicit numbered
  stages from the task, each logged (`[N/15] ...`), `set -euo pipefail`
  already present so any unhandled failure aborts before
  `print_next_steps` — meaning "Install complete." is now only ever
  printed if every required stage succeeded. The one `|| true` on a
  critical step (`render-config || true`, install.sh:140 pre-fix) is
  removed — a failed initial config render now aborts install with a
  clear error instead of silently continuing to start a service with
  no valid config. The two remaining `|| true` uses in the script are
  on genuinely optional SELinux labeling steps and are now commented
  explaining why they're safe to ignore (semanage/restorecon absence on
  a non-SELinux host, or `restorecon` no-op when the context is already
  correct) — not "everything is fine, ignore it" but individually
  justified.
- **Files**: `deploy/almalinux/install.sh`.

## 23. Update/rollback hardening + permission preservation

- **Fix**: `update.sh`'s rollback now (a) restores binaries as before,
  (b) restores `config.json` via `cp -a` from the backup instead of a
  hardcoded `install -m 0644` (closes #2's rollback gap), (c) only
  prints `ROLLBACK SUCCESS`-equivalent messaging after the post-rollback
  health check itself passes (previously it printed a rollback message
  unconditionally after issuing the restart, without re-checking
  health) — if the rollback restart *also* fails health, it now says so
  loudly rather than claiming success.
- **Files**: `deploy/almalinux/update.sh`.
- **Test**: new unit test on the underlying mode-preservation logic is
  not directly testable from bash in CI without a VM; verified by code
  review + `bash -n`/shellcheck; acceptance-test.sh includes a
  post-rollback mode assertion for real-host runs.

## 24. Uninstall semantics

- **Status**: already correct on audit — `uninstall.sh` already
  distinguishes binary/service removal (default) from `--purge-state`
  (destroys REALITY key/users.json/certs) and `--purge-firewall`
  (separate flag), and never deletes state without the explicit flag.
  No change required; `docs/ALMALINUX_DEPLOYMENT.md` updated to
  describe this accurately (it undersold what was already implemented).

## 25. SELinux

- **Fix**: `configure_selinux()` gains explicit `fcontext` labeling for
  the two secret-serving directories in addition to the binary
  (`$STATE_DIR/sing-box`, `$STATE_DIR/hysteria`) and documents
  `ausearch -m AVC -ts recent` as the standard troubleshooting command
  in `docs/ALMALINUX_DEPLOYMENT.md`. `setenforce 0` is explicitly called
  out in the docs as **not** an acceptable production fix. No custom
  policy module is generated (out of scope/untestable without a real
  AlmaLinux SELinux host) — documented as the fallback if `fcontext`
  labeling alone proves insufficient on a real host, with a pointer to
  `audit2allow` for building the smallest possible policy.

## 26. systemd hardening review

- **Finding**: both units already carry the requested directive set
  (`NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `ProtectKernelTunables`, `ProtectKernelModules`,
  `ProtectControlGroups`) per the audit. `ReadOnlyPaths=/etc/vpn/compat`
  on `sing-box.service` needs no change now that #1's ownership fix
  makes every file under it group-`sing-box`-readable. No functional
  regression expected; `systemd-analyze security` output can't be
  captured in this sandbox (no systemd) — `[ ]` for that specific
  verification, left as a manual acceptance step.

## 27. Client-parser / Hiddify validation honesty

- **Fix**: `docs/CLIENT_COMPATIBILITY.md` gets the repeatable manual
  test template (task §35) and a HONOR MagicOS-specific subsection
  (task §36), and the compatibility matrix is corrected to show
  "not yet validated against a real client" for every row that hasn't
  had an actual device test — removing any "Hiddify = validated" framing
  that wasn't earned.

## 28. Documentation truthfulness pass

- **Fix**: `README.md`, `TASKS.md`,
  `docs/ALMALINUX_DEPLOYMENT.md`, `docs/CLIENT_COMPATIBILITY.md`,
  `docs/COMPATIBILITY_SECURITY_REVIEW.md`,
  `docs/COMPATIBILITY_VERSIONS.md` updated at the end of this session
  to remove/never introduce "production-ready", "Hiddify validated",
  "AlmaLinux validated", "MagicOS validated" claims — replaced with
  explicit implemented-vs-verified language, matching this plan's
  status markers.

---

## Priority order actually followed

1. Deployment actually starts (#1, #5, #7, #22)
2. Secret protection (#2, #3)
3. Credential revocation takes effect (#4, #15)
4. TLS correctness (#5, #6, #9)
5. Rollback (#4, #23)
6. Real sing-box validation (#11)
7. Android usability (#16, #17, #27)
8. Censorship/failure-independence tests (#20 — documented, not executed)
9. Polish (#12, #13, #14, #18, #19, #24, #25, #26, #28)
