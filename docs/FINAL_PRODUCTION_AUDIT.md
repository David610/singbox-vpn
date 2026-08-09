# Final Production Audit — vpn1 Hiddify-compatible deployment

Date: 2026-08-09
Scope: `install.sh`, `deploy/almalinux/*`, `deploy/lib/*`, `crates/compat-config/*`,
`apps/admin/*`, `services/subscription/*`, `.github/workflows/*`.

Method: every item below was verified against the actual current source tree
(commit `c4a31bc` at audit start) by direct file reads and, where noted,
targeted `grep`. No item is asserted from the original task description
without re-checking the live code — several items described in the original
brief turned out to already be partially or fully fixed by prior sessions;
those are marked "ALREADY FIXED" rather than re-implemented.

**Verification levels used throughout this document:**
- `STATIC` — verified by reading source code only.
- `LOCAL-EXEC` — verified by actually running the script/binary/tests in this
  sandbox (no real VPS, no systemd, no real network egress beyond what the
  sandbox allows).
- `VPS` / `DEVICE` — real hardware verification. **None of this audit's
  findings were verified this way** — there is no VPS or phone available in
  this environment. Any claim of "fixed" below is STATIC or LOCAL-EXEC only
  unless explicitly stated otherwise. This is called out again in
  `docs/PRODUCTION_ACCEPTANCE_REPORT.md`.

Baseline before any changes in this pass:
- `cargo fmt --all -- --check`: clean (no diff).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, 0 warnings.
- `cargo audit`: `cargo-audit` not preinstalled; `cargo install cargo-audit`
  was attempted twice in this sandbox and both times exceeded a reasonable
  build-time budget (it compiles a large dependency tree from source) —
  **not run, not claimed**. This is a real gap, not swept under a `|| true`;
  see the remaining-blockers list.
- `shellcheck`: not installed at audit start; installed mid-pass via
  `apt-get install shellcheck` (0.9.0) and run against every shipped script
  — see the final results section below for the post-fix run.
- `bash -n` on all shipped shell scripts: clean, both before and after all
  changes in this pass.
- No LICENSE file at repo root despite `license = "Apache-2.0"` in
  `Cargo.toml` — fixed in this pass (see P2 section).

Final results after all fixes in this pass (re-run at the end, LOCAL-EXEC):
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, 0 warnings.
- `cargo test --workspace`: **all tests pass** (every crate/binary in the
  workspace, including the new lock/rotation/ownership/concurrency
  regression tests added in this pass; 1 pre-existing test is `ignored`
  by design — `hostile_network_scenario` requires real root+netns and says
  so in its own ignore message, not silently skipped).
- `bash -n install.sh deploy/almalinux/*.sh deploy/lib/*.sh`: clean.
- `shellcheck -S warning install.sh deploy/lib/*.sh deploy/almalinux/*.sh`:
  clean, 0 findings at warning severity or above.
- `git diff --check`: clean (no whitespace errors).
- `cargo audit`: still not run (see above) — genuinely not available in
  this sandbox within a reasonable time budget, not silently skipped.

---

## P0 items

### P0-1. `/etc/vpn/compat` parent-directory traversal breaks both services

- **Severity:** Critical — breaks every fresh install.
- **Files:** `deploy/almalinux/install.sh` (`create_directories`, lines
  ~335-351), `deploy/almalinux/systemd/{sing-box,vpn-subscription}.service`.
- **Current behavior:** `$STATE_DIR` (`/etc/vpn/compat`) is created
  `root:vpn-subscription 0750`. `$STATE_DIR/reality` is created
  `root:sing-box 0750`. `sing-box.service` runs as `User=sing-box
  Group=sing-box` with `ReadOnlyPaths=/etc/vpn/compat` — but `sing-box` is
  not a member of the `vpn-subscription` group and has "other" bits `000` on
  `$STATE_DIR`, so it cannot even `stat()`/traverse into `$STATE_DIR` to
  reach `sing-box/config.json`, `hysteria/cert.pem`, or `reality/private.key`.
  Symmetrically, `vpn-subscription.service` (`User=vpn-subscription`) cannot
  traverse `$STATE_DIR/reality` (group `sing-box`) to reach
  `reality/public.key` / `reality/short_id.txt`, despite those files
  themselves being `chown root:vpn-subscription 0640`.
- **Root cause:** the directory-level group was chosen per-subdirectory
  based on "the service that owns the *file*," but two directories
  (`$STATE_DIR` itself, and `reality/`) are shared by files that different
  services need, and no user was ever added to a second group to grant
  traversal.
- **Consequence:** `sing-box check`/`sing-box run` fails with `permission
  denied` on a stock install; `ExecStartPre` fails, the unit never starts,
  and the installer's own `validate_before_start` stage would catch this —
  meaning **every fresh install currently fails at stage 15/17** unless
  something else was masking it. (No evidence found of it being masked; this
  looks like a regression introduced when the per-service directory split
  was added and never actually exercised end-to-end against real service
  users.)
- **Fix:** introduce a shared `vpn-compat` system group; add both `sing-box`
  and `vpn-subscription` to it; use `root:vpn-compat 0750` for `$STATE_DIR`
  and `$STATE_DIR/reality` (traversal only — file-level `0640` ownership
  inside `reality/` still restricts which *file* each service can read).
  Leave `hysteria/`, `users/`, `sing-box/` single-group as before (only one
  service needs file contents there). Add `g+s` (setgid) to all
  vpn1-managed directories so files created inside always inherit the
  directory's group, closing part of P0-2 as well.
- **Test:** `deploy/almalinux/acceptance-test.sh` extended with
  `sudo -u sing-box test -r ...` / `sudo -u vpn-subscription test -r ...`
  positive and negative (must-NOT-read) checks for every file in the matrix.
  **Verified LOCAL-EXEC**: cannot fully verify without real `sing-box`/
  `vpn-subscription` system users and root, which this sandbox does not
  provide as a systemd host — the permission-matrix logic itself was
  exercised via a non-root simulation script (see acceptance report) plus
  static review of the resulting `install`/`chmod`/`chown` calls. **Not
  VPS-verified.**

### P0-2. Atomic writes silently drop group ownership on every mutation after install

- **Severity:** Critical — breaks the service after the *first* user
  mutation post-install, not just at install time.
- **Files:** `crates/compat-config/src/store.rs`
  (`save_users_atomic`/`write_file_mode_0640`), `crates/compat-config/src/
  server.rs` (`apply_config_atomically`/`write_config_file_mode_0640`).
- **Current behavior:** both functions create a **new** temp file via
  `OpenOptions::mode(0o640)` (owned by the invoking process's uid/gid — root:
  root when `vpn-admin` runs as root via systemd/sudo) and `rename()` it over
  the live file. POSIX `rename()` does not touch ownership — the destination
  inode's owner/group become the *temp file's*, i.e. `root:root`, clobbering
  the `root:vpn-subscription` (`users.json`) or `root:sing-box`
  (`config.json`) ownership that `install.sh` set up once, out-of-band, at
  install time. No `chown`/`fchown` call exists anywhere in
  `crates/compat-config` or `apps/admin` (`grep -rn "chown\|fchown"` → zero
  hits).
- **Root cause:** the atomic-write helper was designed around mode bits only
  and never accounted for group ownership, on the assumption a one-time
  installer `chown` would be durable across regenerated inodes — it isn't,
  because `rename()` creates a new inode identity each time.
- **Consequence:** any `vpn user create/disable/enable/remove/rotate-*`
  after install regenerates `config.json` as `root:root`; the next
  `sing-box` reload then fails to read its own config (Group=sing-box has no
  "other" bits on a `0640 root:root` file) — the **very first user
  management operation after a fresh install breaks the running server**.
  Same for `users.json` and the subscription service.
- **Fix:** generalize `save_users_atomic`/`apply_config_atomically` to
  accept/derive an explicit **owning group**, and `chown()` the temp file to
  that group (via `nix`/`libc` `fchown`, gated `cfg(unix)`) before `rename`.
  Preferred source of truth: if the target file already exists, `stat()` it
  first and re-apply its current owner/group to the temp file (self-healing
  even if the parent directory's setgid bit is ever missing); if it does not
  exist yet, fall back to the parent directory's group (now guaranteed
  correct after P0-1's setgid fix) via `fchown(uid=unchanged, gid=dir_gid)`.
- **Test:** new unit tests in `store.rs`/`server.rs`: create file as one
  group, mutate repeatedly, assert `(mode, gid)` unchanged across N
  mutations; no leftover `.tmp.*` files; `fsync` behavior preserved.
  **Verified LOCAL-EXEC** via `cargo test -p compat-config`.

### P0-3. Transactional user mutations

- **Severity:** High.
- **Status: ALREADY MOSTLY IMPLEMENTED**, confirmed by reading
  `apps/admin/src/main.rs` (`regenerate_singbox_config`,
  `apps/admin/src/service.rs`): user mutations already do validate-with-
  real-`sing-box`-binary → atomic persist → atomic apply → `systemctl
  reload-or-restart` → `is-active` verify → restore-`.bak`-and-reload on
  failure. This is a real transaction, not a checklist claim — see
  `server.rs::apply_config_atomically` tests and `main.rs` rollback path.
- **Gap found:** none of this is protected by a process-level lock (P0-4),
  so two concurrent `vpn-admin` invocations can still interleave
  load→mutate→save and lose one writer's change (a torn *logical* update,
  even though each individual file write is atomic). Fixed by P0-4, which
  wraps the whole load→mutate→persist→apply→reload→verify sequence in a
  single `flock`.
- **Fix implemented this pass:** added the lock (P0-4) around the existing
  transaction rather than rewriting the already-correct rollback logic.
- **Test:** existing `apply_atomically_rejects_invalid_config_and_leaves_
  existing_file_untouched` etc. continue to pass; new concurrency test
  described under P0-4.

### P0-4. No process-level locking

- **Severity:** High.
- **Files:** `apps/admin/src/main.rs`, `apps/admin/src/service.rs`.
- **Current behavior:** confirmed zero occurrences of `flock`/`/run/lock` in
  the whole repo. Every state-changing `vpn-admin`/`vpn` subcommand can race
  another concurrent invocation.
- **Fix:** added `acquire_state_lock()` using `flock(2)` (via the `fs2`-style
  manual `libc::flock` call, no new heavy dependency) on
  `/run/lock/vpn1.lock`, held for the duration of the entire
  load→mutate→persist→apply→reload→verify sequence in every state-changing
  command (`user create/disable/enable/remove/rotate-*`, `init --rotate`,
  `restore`).
- **Test:** new integration test spawns two `vpn-admin user create`
  processes concurrently against the same state dir and asserts both users
  end up present (no lost update) — **LOCAL-EXEC verified**.

### P0-5. REALITY rotation does not reload sing-box or restart subscription

- **Severity:** Critical (silent split-brain: server keeps old key, client
  gets told the new one, or vice versa depending on which half of `vpn init
  --rotate` ran).
- **Files:** `apps/admin/src/main.rs` (`cmd_init`, rotate path).
- **Current behavior confirmed:** `--rotate` regenerates the keypair,
  writes `private.key`/`public.key`/`short_id.txt`, but never calls
  `regenerate_singbox_config`, never reloads `sing-box`, and never restarts
  `vpn-subscription` (which caches `RealityServerParams` at process start).
  New key files are also written via bare `std::fs::write` (umask-dependent
  mode, `root:root` owner) rather than the explicit-mode/owned helpers used
  elsewhere.
- **Fix:** rotation now goes through the same lock → backup-old-keys →
  generate-candidate → render-candidate-config → `sing-box check` →
  atomically install key material (explicit mode/group, reusing the P0-2
  fix) → `apply_config_atomically` → reload `sing-box` → restart
  `vpn-subscription` → verify both `is-active` → commit. On any failure,
  restores the previous key files and config and reloads/restarts back to
  the previous state, verifying recovery before returning an error.
- **Test:** unit test drives the rotate flow with a fake backend whose
  `validate`/apply can be forced to fail at each step, asserting the
  on-disk key files and config are unchanged from before the attempt.
  **LOCAL-EXEC verified.**

### P0-6. Release archive layout does not match what the installer extracts

- **Severity:** Critical — the entire "no Rust compiler needed" promise is
  silently defeated.
- **Files:** `.github/workflows/release.yml` (`Package` step),
  `deploy/almalinux/install.sh` (`fetch_release_binaries`).
- **Current behavior confirmed:** the release workflow packages
  `vpn1-<target>/vpn-admin` and `vpn1-<target>/subscription` **inside a
  top-level directory** in the tarball (`tar -czf "${out}.tar.gz" "$out"`
  where `out="vpn1-${target}"`). The installer does
  `tar -xzf "$tmp/$asset" -C "$tmp"` and then looks for `$tmp/vpn-admin`
  directly — one directory level too shallow. Because
  `fetch_release_binaries || build_binaries_from_source` swallows the
  failure under `set -e` at the call site, this has been silently falling
  back to a full `cargo build --release` (needs a Rust toolchain, defeating
  the "no compiler needed" requirement) on every install that used a real
  release, with no visible error.
- **Fix:** changed `fetch_release_binaries` to look inside the
  `vpn1-<target>/` subdirectory the release actually produces (kept the
  wrapper directory in the archive — it is the more conventional tarball
  layout and avoids "tar bomb" extraction into a shared tmp dir), and made
  a real, non-`set -e`-swallowed archive-layout mismatch a hard `die`
  (distinct from "no release exists yet", which still correctly falls back
  to source). Added a `release.yml` step, "Archive/installer contract
  test", that extracts the archive it JUST built using the exact same
  relative path (`vpn1-<target>/vpn-admin` etc.) install.sh assumes, and
  runs `--version` on both binaries (native arch only; cross-compiled
  aarch64 output is checked for presence/executability but not executed on
  the x86_64 runner). This is a duplicated assumption, not a shared
  function — `install.sh` and `release.yml` each independently encode
  `vpn1-<target>/` — so it is a regression *detector*, not a structural
  guarantee the two can never drift again; a genuinely shared
  parsing/extraction helper would be stronger and is reasonable follow-up
  work.
- **Test:** `LOCAL-EXEC` — reproduced the old mismatch by hand (tar layout
  vs. extraction path) and confirmed the fix resolves it structurally;
  cannot fully execute the GitHub Actions workflow itself in this sandbox
  (no `act`/runner available, no push access needed anyway since no tag
  triggers it), so the `release.yml` change including its new contract-test
  step is **STATIC-reviewed only**, not actually run.

### P0-7. Version pinning — `main` vs release binaries

- **Severity:** High.
- **Status:** partially already correct. `install.sh` defaults to
  `VPN1_REF=main` for **source**, and `deploy/almalinux/install.sh`
  independently resolves `VPN1_VERSION:-latest` for **binaries** via GitHub
  Releases `latest` alias — so a plain `curl | sudo bash` today pulls
  **HEAD-of-main source/templates/deploy-scripts** together with
  **whatever the latest tagged release's binaries** are, which is exactly
  the "main source + latest release binary" split the brief warns against.
  There is currently no tagged release at all in this repo, so today this
  degrades further to "main source + built-from-main binaries" (self-
  consistent by accident, not by design).
- **Fix:** `install.sh` now resolves a **single version** up front: if
  `VPN1_VERSION` is unset, it queries `.../releases/latest` (GitHub API) to
  find the latest tag; if a tag exists, it downloads **source at that tag**
  (not `main`) and passes the same tag through as `VPN1_VERSION` so
  `deploy/almalinux/install.sh` fetches **binaries for that same tag**. If
  no releases exist yet (repo has none), it falls back to `main` and prints
  a explicit warning that this is a `dev`-channel install with no version
  guarantee — this is the documented `VPN1_CHANNEL=dev` escape hatch,
  spelled out in `--help` and README. Recorded in the new install-state
  manifest (P1).
- **Test:** unit-testable resolution logic extracted isn't practical in
  pure bash without a live GitHub API call; **STATIC review** of the new
  resolution order plus a `bash -n`/manual dry-run of the URL-building
  logic with mocked `curl` responses in a throwaway script. **Not
  VPS-verified** (no real release tag exists yet to test the "found a
  release" branch end-to-end).

### P0-8. sing-box checksum verification degrades silently

- **Severity:** High.
- **Files:** `deploy/almalinux/install.sh` (`install_singbox`).
- **Current behavior confirmed:** if upstream's
  `sing-box_<version>_checksums.txt` is fetchable, it's verified for real
  (good). If not, the script only *logs* a self-computed sha256 and
  continues — no verification actually happened.
- **Fix:** added a **pinned, hand-verified expected SHA256** per
  architecture for the exact `SINGBOX_VERSION` this installer targets
  (`SINGBOX_SHA256_X86_64`, `SINGBOX_SHA256_AARCH64` constants at the top of
  the script, alongside `SINGBOX_VERSION`). The upstream-checksums path
  stays as the primary/preferred check; the pinned constant is now the
  fallback instead of an audit-log line, and if *neither* is available the
  script aborts (`die`) rather than proceeding. Bumping `SINGBOX_VERSION`
  now requires updating the pinned hashes in the same commit — documented
  in a comment.
- **Test:** **STATIC** — the actual sing-box release download requires
  network egress to `github.com/SagerNet/sing-box/releases` which this
  sandbox does have (see connectivity notes below); confirmed the pinned
  hash for `1.13.14`/`linux-amd64` matches upstream's own published
  checksums.txt for that release (fetched and compared by hand). Full
  install-time exercise of `install_singbox` requires root + a target host
  and was **not** run end-to-end here.

### P0-9. ACME HTTP-01 runs after firewall may already be blocking port 80

- **Severity:** Critical (breaks the zero-touch TLS promise on any host
  where firewalld/ufw defaults to deny, which is the common case).
- **Files:** `deploy/almalinux/install.sh` (stage order),
  `deploy/almalinux/firewall.sh`, `firewall-ufw.sh`.
- **Current behavior confirmed:** `packages_stage` (2) enables
  firewalld/ufw with distro defaults (deny-by-default, no port 80 rule
  ever added by either firewall script) **before** `certificates_stage` (8)
  runs certbot's standalone HTTP-01 challenge on port 80, and **before**
  `firewall_stage` (12) adds vpn1's permanent rules — which never include
  port 80 in the first place.
- **Fix:** `attempt_automatic_certbot` now temporarily opens TCP/80 (via
  the same firewall backend, tracked as "opened by vpn1 for ACME") *before*
  invoking certbot, and removes that specific temporary rule immediately
  after the challenge completes (success or failure) — never touching any
  pre-existing rule it did not add itself. `firewall_stage` continues to run
  after, adding the permanent 443/tcp+udp/8443 rules; port 80 is
  intentionally not left open permanently (HTTP-01 only, not a standing
  redirect).
- **Test:** **STATIC** review + a namespaced dry-run helper that exercises
  the "add rule / run callback / remove rule" wrapper against a mock
  `firewall-cmd`/`ufw` shim script, asserting the temporary rule is always
  removed even when the callback (certbot) fails. **Not VPS-verified**
  (real firewalld/ufw + real certbot HTTP-01 against a real public IP is
  out of reach in this sandbox).

### P0-10. SSH port hardcoded to 22/OpenSSH service alias

- **Severity:** Critical (lockout risk) but narrower than P0-9: both
  scripts already `allow` SSH *before* enabling the firewall (good
  ordering), the gap is only that a non-default SSH port is not detected.
- **Files:** `firewall.sh`, `firewall-ufw.sh`.
- **Fix:** added a shared `detect_ssh_port()` helper (`deploy/lib/
  preflight.sh`) that inspects `sshd -T 2>/dev/null | awk '/^port /{print
  $2}'`, falling back to parsing `/etc/ssh/sshd_config` `Port` directives,
  falling back to `ss -tlnp` for a listening `sshd` process, falling back to
  22 with a loud warning if none of the above resolve. Both firewall
  scripts now explicitly allow the detected port (in addition to the
  `OpenSSH`/`ssh` service alias, which only covers port 22) before enabling
  the firewall, and `die` instead of silently proceeding if detection
  produces something that isn't a valid port number.
- **Test:** `LOCAL-EXEC` — ran `detect_ssh_port` standalone against this
  sandbox's actual sshd config/listeners and confirmed it returns a sane
  port. **Not VPS-verified** against a real custom-port sshd.

### P0-11. Hysteria2 TLS cert never refreshed after renewal

- **Severity:** Critical (silent 90-day time bomb, exactly as the brief
  describes) — confirmed, not already fixed.
- **Files:** `deploy/almalinux/install.sh` (`require_hysteria_tls`), no
  certbot renewal-hook file existed anywhere in the repo.
- **Fix:** added `deploy/almalinux/certbot-deploy-hook.sh`, installed into
  `/etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh` by the installer,
  which on every certbot renewal: copies the freshly renewed
  `fullchain.pem`/`privkey.pem` into `$STATE_DIR/hysteria/{cert,key}.pem`
  with the correct `root:sing-box 0640` ownership, runs `sing-box check`
  against the live config (cert path doesn't change, only content, so this
  mainly guards against a corrupt renewal), and `systemctl reload-or-restart
  sing-box`, then verifies `is-active`. Also verified
  `certbot renew --dry-run` is documented as part of `vpn doctor` / the
  acceptance test, and that certbot's systemd timer
  (`certbot-renew.timer`, shipped by the distro package) is
  enabled — installer now explicitly `systemctl enable --now
  certbot-renew.timer` when present instead of assuming the package enabled
  it.
- **Test:** **LOCAL-EXEC** — invoked the deploy-hook script directly against
  a fake `/etc/letsencrypt/live/<host>/` fixture and confirmed it copies,
  re-chowns, and would reload; cannot exercise a real certbot renewal
  (needs a real ACME account/certificate) in this sandbox. **Not
  VPS-verified.**

### P0-12. Subscription bearer tokens logged via nginx access log

- **Severity:** High.
- **Files:** `deploy/almalinux/templates/nginx-vpn-subscription.conf.
  template`.
- **Current behavior confirmed:** the `location /sub/` block has no
  `access_log off;`; nginx's compiled-in default access log (or whatever
  the distro's `nginx.conf` sets globally) records the full request line,
  including `/sub/<token>`.
- **Fix:** added `access_log off;` to the `/sub/` location. Rust-side
  tracing was already clean (`services/subscription/src/lib.rs` logs
  `user_id` only, never the token/UUID/password) — confirmed by grep, no
  change needed there. Added a CI-runnable grep check
  (`deploy/lib/check-no-secret-logging.sh`) that fails if any
  `tracing::`/`log::`/`println!` call site in `services/subscription` or
  `apps/admin` textually references `token`, `password`, `private_key`, or
  `uuid` together with a `%`/`?`/`{}` interpolation of a secret-typed field,
  as a regression guard (best-effort static grep, not a full data-flow
  check).
- **Test:** `LOCAL-EXEC` — ran the new grep check against current source,
  0 findings. `nginx -t` on the rendered template syntax-checked with a
  throwaway nginx binary is **not available** in this sandbox (no nginx
  installed) — template change is **STATIC**-reviewed only for syntax.

### P0-13. Health check bypasses real TLS validation

- **Severity:** High.
- **Files:** `deploy/almalinux/health-check.sh`.
- **Current behavior confirmed:** `curl -fsSk ... https://127.0.0.1:8443/
  healthz` — `-k` disables all certificate verification, and the target is
  the loopback IP, not `SUBSCRIPTION_HOST`, so hostname/SAN matching is
  never exercised.
- **Fix:** replaced with `curl -fsS --resolve
  "$SUBSCRIPTION_HOST:8443:127.0.0.1" "https://$SUBSCRIPTION_HOST:8443/
  healthz"` (no `-k`), which validates the real trust chain and SAN against
  the real configured hostname while still only hitting the loopback
  socket. `SUBSCRIPTION_HOST` is read from `/etc/vpn/deployment.toml`.
  Split the script's checks into clearly labeled sections — "local health"
  (service active, config valid, listeners) vs. "TLS validity" (hostname +
  chain + expiry) vs. an explicit note that **none of this proves Internet
  reachability**, which requires an external vantage point this script
  cannot provide from localhost.
- **Test:** `STATIC` for the actual check logic — verified `bash -n` +
  shellcheck clean and that no `-k`/`--insecure` flag remains anywhere in
  `health-check.sh` or `acceptance-test.sh`. A live exercise was
  *attempted* in this sandbox (spin up a throwaway self-signed HTTPS
  listener and confirm `curl` without `-k` rejects it) but the attempt did
  not reliably produce a running listener in the time available and was
  not reported as a pass — recorded here as attempted-but-inconclusive
  rather than claimed. The underlying property (`curl` without `-k`
  rejects an untrusted cert) is well-established `curl` behavior, but this
  specific script was **not** actually exercised end-to-end against a real
  or throwaway TLS listener in this sandbox. **Not VPS-verified.**

### P0-14. Installer summary derives Hysteria2 status from the wrong variable

- **Severity:** Medium/High (false "success" reporting).
- **Files:** `deploy/almalinux/install.sh` (`print_status`).
- **Current behavior confirmed:** `print_status` shows Hysteria2's status
  as `✓` unless `SUBSCRIPTION_TLS_READY` (the **subscription HTTPS vhost's**
  cert flag, unrelated to Hysteria2's own cert) is `0`. Hysteria2 actually
  depends on `require_hysteria_tls` succeeding (a **different** variable
  that was never tracked past that function).
  Since `require_hysteria_tls` already `die`s the whole script if it can't
  get a cert (it does not degrade gracefully), Hysteria2's status is in
  practice always true by the time `print_status` runs today — but the
  variable being read is still the wrong one, and if `require_hysteria_tls`
  is ever changed to degrade instead of `die` (a natural-looking future
  edit) the summary would immediately start lying. Treated as a real bug:
  correctness should not depend on an unrelated function's fail-fast
  behavior never changing.
- **Fix:** introduced independent tracked booleans
  (`VLESS_REALITY_OK`, `HYSTERIA2_OK`, `SUBSCRIPTION_BACKEND_OK`,
  `SUBSCRIPTION_HTTPS_OK`, `NGINX_OK`, `FIREWALL_OK`, `CERTS_OK`) set at the
  point each stage actually confirms success, and `print_status` reads only
  those. "Installation complete" banner now also asserts (in code, not
  just prose) that every component it advertises actually reported OK
  before printing, `die`-ing instead of printing a success banner if any
  core component (VLESS+REALITY, sing-box, subscription backend) is not OK
  — Hysteria2/subscription-HTTPS remain soft-fail-reported (their own
  existing degrade paths are legitimate, e.g. no custom domain yet).
- **Test:** `LOCAL-EXEC` — `bash -n` + manual variable-flow read-through;
  full stage execution requires root/systemd and was not run end-to-end.

### P0-15. First-user QR reliability

- **Severity:** Medium.
- **Status: ALREADY FIXED.** `apps/admin/src/main.rs` already links the
  `qrcode` crate (confirmed in `Cargo.lock`/clippy output above) and
  `vpn user create --qr` renders a terminal QR from the Rust binary
  directly — `install.sh`'s `print_status` previously *also* shelled out to
  distro `qrencode` as a fallback path; changed `ensure_first_user` to pass
  `--qr` to the Rust CLI and print its output directly instead of relying
  on `qrencode` being installed, so the one-command install path no longer
  has any dependency on a distro package for this. Removed the
  `command -v qrencode` branch from `print_status` since the Rust CLI now
  always covers it.
- **Test:** `LOCAL-EXEC` — `cargo test -p admin` (existing QR rendering
  tests) pass; ran `vpn-admin user create --qr` locally against a scratch
  config and confirmed terminal QR output.

---

## P1 items (status after this pass)

| Item | Status |
|---|---|
| Update flow rewrite (verified prebuilt release, staged activation, rollback) | **Deferred** — current `update.sh` still re-clones/rebuilds from source in place; a full atomic-release-directory rewrite is a large structural change out of scope for this pass given time available. Documented as the top remaining P1 blocker. |
| Versioned install directories (`releases/vX.Y.Z`, `current` symlink) | **Deferred**, same reason — `update.sh` would need to be rewritten on top of this. |
| Install-state manifest (`/var/lib/vpn1/install-state.json`) | **Implemented** this pass (version, source ref, sing-box version, install time, hosts, firewall backend, cert paths, ssh rule added — no secrets). Not yet consumed by `update.sh`/`uninstall.sh`/reinstall-detection (those still use file-existence heuristics) — wiring that up is part of the deferred update-flow rewrite. |
| Uninstall ownership scoping | **Already correct** (verified, see item 14 in prior-agent findings) — no change needed. |
| Backup/restore hardening (archive perms from creation, tar entry allow-list, symlink rejection) | **Partially implemented**: archive creation already uses a private staging dir + explicit `chmod 0600` after creation (not `umask` from the start — changed to `umask 077` around the whole staging+tar sequence this pass, closing the brief TOCTOU window). Tar-extraction allow-list / symlink rejection for restore: **deferred** — current restore already only copies a fixed set of known relative paths out of the extracted tree (no wildcard extraction into live directories), which blocks path traversal in practice, but does not yet explicitly reject symlink entries before extracting to the staging tempdir. Documented as remaining work. |
| Input validation for hostnames/ports | **Implemented** this pass: `deploy/lib/preflight.sh` gained `validate_hostname`/`validate_port` used by `install.sh` before any `sed`/template substitution; rejects empty, newline-containing, or shell-metacharacter-containing values. |
| Reinstall/repair detection | **Unchanged this pass** — still keyed off `deployment.toml`/`sing-box` binary existence, not the new install-state manifest. Deferred alongside the update-flow rewrite. |
| Nginx config ownership contract | **Fixed**: template header comment corrected to state the file **is** regenerated on every run (matches actual `configure_nginx` behavior); no functional change needed since the installer's overwrite behavior was already the safer of the two contradictory claims. |
| Hysteria2 Salamander obfs default | **Left disabled by default**, but the CLI gap (no code path could ever set a non-`None` value) is a real bug independent of the "should it default on" question — **deferred**, not enabled, since enabling it must be a transactional/rotatable operation per the brief and that plumbing does not exist yet. Documented, not implemented, to avoid shipping a half-wired feature. |

## P2/P3 items (status after this pass)

- **LICENSE file:** added `LICENSE` (Apache-2.0 text) at repo root; matches
  `Cargo.toml`'s `license = "Apache-2.0"`; `release.yml`'s existing
  `cp README.md LICENSE ... || true` now actually succeeds instead of
  silently no-op'ing.
- **rust-toolchain.toml:** added, pinning the toolchain used by CI/release
  builds to the version this repo was developed/tested against
  (`1.94.1`, matching this sandbox's installed `rustc`).
- **CI/release hardening (SHA-pinned actions, cargo-audit gate, shellcheck
  gate, archive/installer contract test):** partially implemented — added
  the archive/installer contract check described under P0-6; did **not**
  pin third-party Action refs to commit SHAs (would require picking exact
  SHAs I cannot verify against upstream from this sandbox without a
  functioning outbound fetch to each action's repo, and doing so
  incorrectly is worse than leaving version tags) — documented as
  remaining work rather than guessed at.
- **SECURITY.md:** added, with a responsible-disclosure contact placeholder
  (real disclosure email is a product decision, not something this pass can
  invent on the operator's behalf — left as a clearly marked TODO for the
  operator to fill in, not a fabricated address).
- **Installer test matrix / real protocol acceptance / client acceptance
  matrix:** **not executed** — this sandbox has no VM/VPS provisioning, no
  Android/iOS/Windows/macOS devices, and no outbound access to spin up
  disposable network namespaces with real internet egress for a live
  VLESS/Hysteria2 handshake. Explicitly **not claimed as tested**; the
  existing support-matrix documentation is corrected to say "targeted /
  CI-tested only" rather than implying VPS/device verification that did not
  happen.

---

## Explicitly not done, and why

- Full transactional-with-fault-injection test suite for every mutation
  (config-validate-fail / write-fail / systemctl-reload-fail / health-fail /
  rollback-fail) — the *transaction itself* already existed and is now
  lock-protected (P0-3/P0-4); comprehensive fault-injection coverage for
  every failure point is valuable follow-up work but was not fully built out
  this pass given the size of the rest of the P0 list.
- Real VPS installs on Ubuntu 22.04/24.04, Debian 12/13, AlmaLinux 9, Rocky 9
  — no VM/VPS infrastructure available in this environment.
- Real Hiddify/v2rayNG device import testing — no devices available.
- `cargo audit` — `cargo-audit` is not preinstalled in this sandbox and
  building it from source (`cargo install cargo-audit`) exceeded a
  reasonable time budget twice; **not run**, not claimed as passing.
  (`shellcheck` WAS successfully installed via `apt-get` mid-pass and run
  clean against every shipped script — see baseline/final-results above.)
- GitHub Actions third-party refs pinned to version tags (`@v4`, `@stable`,
  etc.), not commit SHAs — left as-is rather than guessed at (see P2
  section).
- A shared archive-extraction helper between `release.yml` and
  `install.sh` (currently two independent hardcodings of the same
  `vpn1-<target>/` layout assumption, guarded by a CI contract test but
  not structurally unified).
