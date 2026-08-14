# Implementation Status (v1.0 baseline)

Compact engineering handoff. Full boundary definition: `docs/SUPPORTED_PRODUCT.md`.
Release-readiness evidence (PASS/FAIL/UNVERIFIED matrix): `docs/ACCEPTANCE.md`
if present, otherwise the latest PR final-response acceptance pass.
Read `docs/SUPPORTED_PRODUCT.md` first; do not re-audit the repo from scratch.

## Supported scope (summary)

AlmaLinux 9 x86_64, <=10 users, one VPS, upstream sing-box, VLESS+REALITY
TCP/443 primary, Hysteria2 UDP/443 optional, Hiddify/sing-box-compatible
clients only, custom domain default. Native adaptive stack
(`client-daemon`, `transport-native`, `policy`, `failure-classifier`,
`network-state`, `rendezvous-client`, `telemetry`, `services/rendezvous`,
`services/relay-agent`) is a separate, out-of-scope product surface for
v1.0 — see `deploy/local/run-dev-slice.sh` for its dev-only entry point.

## Supported file/test surface (avoid full-workspace runs when only this changed)

- Installer: `install.sh` (bootstrap, checksum-verifies the release
  source archive for any pinned `VPN1_VERSION`) -> `deploy/almalinux/
  install.sh` (real implementation, builds `-p admin -p subscription`
  only).
- Uninstall: `uninstall.sh` -> `deploy/almalinux/uninstall.sh` ->
  offline entry point `bin/vpn1-uninstall` (persisted to
  `/opt/vpn1/bin/vpn1-uninstall`).
- Update/repair/render: `deploy/almalinux/update.sh`, `render-config.sh`,
  `health-check.sh`, `acceptance-test.sh`, `certbot-deploy-hook.sh`.
- Release: `.github/workflows/release.yml` publishes per-arch binary
  tarballs + a checksum-manifested `vpn1-src.tar.gz` source archive +
  combined `SHA256SUMS`, all fetched/verified by `install.sh`.
- Shared shell libs: `deploy/lib/*.sh` plus `deploy/lib/versions.env`
  (one authoritative sing-box version/checksum/arch source).
- Rust crates in the supported dependency closure: `crates/common`,
  `crates/compat-config`, `apps/admin`, `services/subscription`.
- Rust tests: `crates/compat-config/tests/*` + `src/**` unit tests,
  `apps/admin/tests/*`, `services/subscription/tests/*`.
- Shell tests: `deploy/lib/tests/*.sh` (20 files).
- Recovery: `docs/RECOVERY.md` (manual disaster-recovery procedure),
  `vpn-admin user links <id>` (out-of-band profile delivery independent
  of the subscription hostname).
- CI gates (`.github/workflows/ci.yml`): `fmt`, `clippy`, `test`
  (workspace-wide — see Known CI scope note), `audit`, `docs`, `shell`,
  `secret-logging-check`, `license-check`, `singbox-validate` (real
  pinned `sing-box` + real REALITY/Hysteria2 handshake).
- **FAST GATE**: `bash deploy/lib/fast-gate.sh` — one canonical command
  for all of the above (supported-crate scope only).
- **DESTRUCTIVE lifecycle gate**: `deploy/almalinux/lifecycle-acceptance.sh`
  — disposable AlmaLinux 9 host only, over SSH; never run against production.

## Known CI scope note (documented, not changed)

`fmt`/`clippy --workspace`/`cargo build|test --workspace` build the whole
workspace including out-of-scope native adaptive crates. `singbox-validate`
and `deploy/lib/fast-gate.sh` already scope to the supported crates only.
Splitting the workspace-wide CI jobs would touch CI trust/release
plumbing for a runner-minutes-only win; deferred.

## Checkpoint 3 (transactional release-to-release updater) — completed this session

- **Old model (replaced)**: `deploy/almalinux/update.sh` rebuilt
  whatever source happened to be checked out at `/opt/vpn1` via
  `cargo build`, unconditionally required Cargo/Rust, never resolved or
  fetched a target release, never touched the sing-box binary, and
  never wrote the install-state manifest (so `vpn1_version` went stale
  after every update).
- **New model**: `deploy/almalinux/update.sh --version vX.Y.Z` (or
  `--latest` / `--repair`) is a real STAGE -> PREPARE -> SWITCH ->
  ACTIVATE -> VERIFY -> COMMIT transaction. Production path requires no
  Cargo/Rust: it downloads and SHA-256-verifies the target release's
  source archive (reusing `install.sh`'s bootstrap trust model) and
  prebuilt binaries (reusing `fetch_release_binaries()`'s asset/
  checksum contract) into a staging directory under `/opt`, and fails
  closed with "Nothing live has been changed" if either is missing or
  fails verification — never a silent fallback to a source build.
  sing-box is staged/verified and swapped too, only when the target
  release pins a different version. `--dev-rebuild` (or
  `VPN1_CHANNEL=dev`) is the explicit, structurally separate escape
  hatch that keeps the old Cargo-rebuild-from-local-source behavior for
  development/testing.
- Source tree swap is an atomic same-filesystem rename
  (`/opt/vpn1` <-> a staged/previous directory under `/opt`), not an
  in-place overwrite — the previous release stays fully intact until
  SWITCH, and rollback renames it straight back.
- `install-state.json` is only rewritten at COMMIT, strictly after
  `doctor --protocol --require-protocol` passes — a failed update never
  updates the authoritative version record.
- Rollback restores binaries, systemd units, helper scripts, the
  sing-box binary (if changed), and the previous `/opt/vpn1` source
  tree, then re-renders (never rewinds) `users.json`/REALITY material
  with the restored tooling, and reports exactly one of `UPDATE FAILED —
  PREVIOUS RELEASE RESTORED AND VERIFIED` or `UPDATE FAILED — ROLLBACK
  ALSO FAILED` (never a bare "update failed").
- Interrupted-transaction detection: a `TRANSACTION_MARKER` written in
  PREPARE (before the first live mutation) makes a subsequent
  invocation refuse with precise recovery instructions instead of
  starting a new update on top of unknown state; a stale
  `/opt/.vpn1-update-staging.*`/`/opt/.vpn1-prev-*` directory left by a
  killed prior run is detected the same way.
- Same-version requests exit cleanly ("Already at vX.Y.Z...") with zero
  mutation unless `--repair` is given; `--repair` re-fetches and
  re-verifies the *currently recorded* release's own material rather
  than resolving a different version.
- Transaction-only backups/staging directories are removed on
  successful commit (never a permanent backup product) and reused
  (never blindly overwritten) if a prior interrupted transaction is
  detected first.
- `deploy/almalinux/lifecycle-acceptance.sh`'s `--update-to-ref` path
  fixed to actually invoke `update.sh --dev-rebuild` (the old
  `VPN1_REF=...` env var it passed was silently ignored by update.sh —
  a genuine tagged-release A->B lifecycle run remains a separate,
  UNVERIFIED gap pending a first real release, see below).
- New test file `deploy/lib/tests/test-update-transactional.sh`;
  extended `test-update-conditional-restart.sh` and
  `test-state-schema-migration.sh` for the new two-code-path
  (production/dev-rebuild) structure. All static/source-inspection —
  update.sh itself needs root/systemd/a real deployment/network access,
  the same constraint the pre-existing update.sh tests already had.
- **UNVERIFIED** (no disposable AlmaLinux 9 host, no tagged release
  published this session): a real release A -> B production update has
  never been executed end-to-end; every failure-injection scenario
  listed in the checkpoint spec is verified only by static/structural
  inspection of the transaction ordering, not by actually killing the
  process mid-transaction against a live host.

## Checkpoint 1 (SSH/firewall/rollback safety) — completed this session

- **SSH/firewall ordering fixed**: firewalld used to be activated
  (`systemctl enable --now firewalld`) in `packages_stage` with only its
  distro-default rules (which cover the `ssh` **service**, i.e. port 22
  only), 12 stages before `firewall_stage` added a custom sshd port —
  an active window where a custom-port SSH session/new connections
  could be locked out. Fixed: `activate_firewalld_ssh_safe()`
  (`deploy/almalinux/install.sh`) starts firewalld and adds the
  confirmed SSH port's allow rule(s) as one atomic sequence, and never
  touches an already-active firewalld's existing configuration.
  `deploy/almalinux/firewall.sh` (also independently invocable) applies
  the same ordering when run standalone.
- **SSH port detection now fails closed**: `preflight_detect_ssh_port()`
  (`deploy/lib/preflight.sh`) no longer falls back to 22 when `sshd -T`/
  `sshd_config`/live-listener detection is all inconclusive — it returns
  1 with nothing printed. New canonical `preflight_resolve_ssh_port()`
  is the single implementation used by `install.sh`, `firewall.sh`, and
  `firewall-ufw.sh`; it accepts an explicit `VPN1_SSH_PORT` override
  (`install.sh --ssh-port PORT` / `VPN1_SSH_PORT=`), validated via
  `preflight_validate_port`. Inconclusive detection with no override now
  `die`s before any firewall mutation, naming the fix.
- **Rollback ownership tracking now starts before the first mutation**:
  `ownership_mark INSTALL_ATTEMPTED` / `ownership_set_baseline_once
  OPT_VPN1_PRE_EXISTED` used to run AFTER `persist_source_tree` (creates
  `/opt/vpn1`) and `install_idn_support` (installs `libidn2` on RHEL) —
  a failure inside either mutation looked like "nothing mutated yet" to
  `on_fatal_error`'s rollback trap and left it stranded. Both now run
  before those two calls in `preflight_stage`.
- IDN normalization (`чёрт.com` -> `xn--p1aen4b.com`) re-verified
  end-to-end with a real `idn2` binary installed in the dev sandbox
  (`deploy/lib/tests/test-idn-punycode.sh`) — `install_idn_support()`
  (installs `libidn2` for the RHEL family) already ran before
  `resolve_host_config()` in `preflight_stage`; added static regression
  coverage for that ordering plus the rhel-family package name.
- New/extended tests (`deploy/lib/tests/test-installer-hardening.sh`,
  `test-preflight-ordering.sh`, `test-idn-punycode.sh`): SSH detection
  on port 22/2222 (mocked `sshd -T`), fixture `sshd_config`, fail-closed
  with no override, explicit-override accept/reject, listener-fallback
  code path (structural — real match needs a real `sshd` process,
  UNVERIFIED), firewall.sh/firewall-ufw.sh fail-closed + canonical-call
  checks, `activate_firewalld_ssh_safe()` ordering/preservation checks,
  ownership-mark-before-first-mutation ordering.
- **UNVERIFIED** (no disposable AlmaLinux 9 host available this
  session): real custom-SSH-port lockout avoidance on a live host;
  `deploy/almalinux/lifecycle-acceptance.sh` was not run.

## Checkpoint 2 (ownership-safe complete uninstall) — completed this session

- **`/etc/vpn` is no longer blindly `rm -rf`'d**: install.sh now records
  `ETC_VPN_PRE_EXISTED` before ever creating `/etc/vpn` (stage 8).
  uninstall.sh's cleanup is now ownership-gated: `0` (vpn1 created the
  whole tree) -> full removal; `1` (pre-existing) or unset/ambiguous
  (pre-checkpoint-2 install, no record) -> remove only vpn1's own
  entries (`deployment.toml`, `compat/`), preserving everything else —
  defaulting to preservation whenever ownership can't be proven.
- **Fixed-name system resources (systemd units, certbot renewal hook,
  nginx vhost) are now ownership-tracked**: new
  `install_fixed_path_with_ownership()` helper backs up (once) whatever
  already occupied a fixed path before vpn1 wrote there, and records
  whether it pre-existed. uninstall.sh's new `restore_or_remove_fixed_path()`
  restores the exact backup when something pre-existed, removes the
  file when vpn1 created it, and leaves it untouched (never guesses)
  when no ownership record exists at all.
- **Fixed a real ordering bug**: `OPT_VPN1_PRE_EXISTED` (and the new
  fixed-path `PRE_EXISTED` facts) used to be read via `ownership_get`
  *after* `$STATE_DIR_ROOT` (`/var/lib/vpn1`, where the manifest itself
  lives) had already been `rm -rf`'d — silently returning the
  "not pre-existing" default every time, so `/opt/vpn1` was ALWAYS
  treated as vpn1-created (even on the rare host where it genuinely
  pre-existed) and a correctly-restored pre-existing fixed path could
  misreport as residue. Fixed by caching every such fact before that
  removal.
- **Truthful residue verification**: uninstall.sh now collects
  `CRITICAL_RESIDUE` (active vpn1 services, running processes, live
  secrets/credentials, vpn1 binaries, vpn1-owned firewall exposure
  still open after removal was attempted, a vpn1-created `/opt/vpn1`
  still present) separately from `NONCRITICAL_RESIDUE` (a package the
  package manager refused to remove, a userdel/groupdel failure, an
  ambiguous pre-existing fixed path left alone). Cleanup never aborts
  on the first non-fatal failure (this script has no `-e`); the final
  banner prints `UNINSTALL COMPLETE` only when `CRITICAL_RESIDUE` is
  empty, `UNINSTALL INCOMPLETE` (nonzero exit) otherwise. Two
  previously-fatal `die`s on a corrupted firewall-ownership record are
  now `warn` + critical-residue entries so the rest of cleanup still runs.
- **Package names validated before the package manager**: `PKGS_INSTALLED_BY_VPN1`
  entries are filtered through `is_safe_pkg_name()` before `dnf remove`/
  `apt-get remove`; anything not shaped like a real package name is
  skipped and reported as non-critical residue instead of being passed
  through.
- **Legacy uninstall compatibility fixed**: the online bootstrap
  (`uninstall.sh`) previously forwarded `--yes` unconditionally to a
  local `/opt/vpn1/deploy/almalinux/uninstall.sh`. The actual
  pre-`07f8b72` layout (real historical commit `d8a4c87`, verified via
  `git show`) never had a `--yes` flag or any confirmation prompt at
  all — it understood only `--purge-state`/`--purge-firewall` and
  rejected any other flag outright, exactly the failure this was filed
  about. New `run_legacy_uninstaller()` detects which interface the
  local copy actually supports (via `grep`) and translates: drops the
  meaningless `--yes` and adds `--purge-state --purge-firewall` for
  that historical layout; forwards unchanged for the current one.
- **Version-aware damaged-`/opt/vpn1` recovery**: when no local copy is
  usable at all, the bootstrap now reads `/var/lib/vpn1/install-state.json`
  for the exact installed `vpn1_repo`/`vpn1_version` and fetches that
  immutable tag (`refs/tags/$VPN1_REF`) instead of the mutable `main`
  branch, unless the operator passed an explicit `--ref` or no pinned
  version was ever recorded (dev/unreleased install).
- New test file `deploy/lib/tests/test-uninstall-ownership-checkpoint2.sh`:
  `/etc/vpn` preservation with a sentinel file, fixed-path
  restore/remove/ambiguous-preserve, package-name validation
  (accept/reject), the `OPT_VPN1_PRE_EXISTED` ordering fix, and legacy
  flag translation exercised against the **real** historical script
  from `git show d8a4c87:deploy/almalinux/uninstall.sh` (not an
  invented approximation).
- **UNVERIFIED** (no disposable AlmaLinux 9 host available this
  session): the canonical offline command
  (`sudo /opt/vpn1/bin/vpn1-uninstall --yes`) has not been exercised
  against a real install with real systemd/firewalld/SELinux/dnf state,
  nor with outbound networking disabled to prove offline-completeness
  for real.

## Completed checkpoints (one line each — see git log for detail)

1. v1.0 boundary + supported code surface documented (`docs/SUPPORTED_PRODUCT.md`).
2. Canonical FAST GATE + DESTRUCTIVE lifecycle gate (SSH-only, requires `--i-understand-this-is-destructive`).
3. Reproducible installs: `deploy/lib/versions.env` single-sourced; stable channel refuses unpinned branch fallback.
4. Persistent state versioning/migration: `deployment.toml`/`users.json` schema versions + `vpn-admin config validate|migrate`.
5. Installer hardening audit: enforced custom-domain default (`--allow-ip-hostname` required to opt out non-interactively), SSH-port fixture seam, REALITY-init idempotency test.
6. Uninstall hardening: offline `bin/vpn1-uninstall` entry point, confirmation-gated, root-controlled-path + manifest re-validation checks, ownership mode-check bug fixed.
7. Client protocol behavior audit: real sing-box interop suite run (9/9 pass); `docs/CLIENT_PROTOCOL_BEHAVIOR.md` written; subscription-has-no-dns/inbounds and dual-stack-bind regression tests added.
8. Censorship-resilience/recovery honesty audit: added out-of-band `vpn-admin user links`, `docs/RECOVERY.md`, structural Hysteria2-unavailable render test, honest single-VPS/IP-blocking limitations documented.
9. PR merge-readiness pass: fixed the production bootstrap's unverified-source-download gap (`install.sh` now SHA-256-verifies a `release.yml`-published `vpn1-src.tar.gz` for any pinned version), corrected README domain-default and REALITY-decoy claims to match actual installer behavior, shrank this file.

## Blockers

- `cargo test -p admin --test cli` has 5 failures **only when run as
  root** (this sandbox is root): `apply_restored_file_policy()` does a
  real `getgrnam("vpn-subscription")` lookup when `geteuid()==0`, and
  that group doesn't exist outside a real installed host. Pre-existing;
  does not fail in CI (non-root runner). `deploy/lib/fast-gate.sh`
  therefore reports FAIL when run as root; PASS expected in CI/any
  non-root dev environment.

## Important UNVERIFIED items (require real VPS/network/client testing)

- Public reachability, provider firewall correctness.
- Real VLESS+REALITY / Hysteria2 connectivity from an actual Hiddify
  client on a real device.
- Full clean-VPS install -> update -> migrate -> rollback -> uninstall
  lifecycle, including SSH-preservation, on real AlmaLinux 9 (no
  disposable host available this session — `lifecycle-acceptance.sh` not
  executed).
- A real tagged release has never been published — the checksum-verified
  bootstrap path (checkpoint 9) is fixture/unit-tested, not exercised
  against a real GitHub Release.
- DNS leak prevention, IPv6 correctness, kill-switch behavior,
  censorship resistance against a real adversary in a real country.

## Canonical verification commands (supported product)

```bash
# FAST GATE — one command, run after every change (see Blockers re: root)
bash deploy/lib/fast-gate.sh

# DESTRUCTIVE lifecycle gate — disposable AlmaLinux 9 host ONLY, over SSH
./deploy/almalinux/lifecycle-acceptance.sh \
  --host root@DISPOSABLE-HOST --i-understand-this-is-destructive

# offline uninstall — no network access needed
sudo /opt/vpn1/bin/vpn1-uninstall --yes

# production update — transactional, checksum-verified, no Cargo/Rust required
sudo /opt/vpn1/deploy/almalinux/update.sh --version vX.Y.Z
sudo /opt/vpn1/deploy/almalinux/update.sh --latest      # resolve latest stable tag
sudo /opt/vpn1/deploy/almalinux/update.sh --repair       # reconcile the current release, no version change

# out-of-band recovery — no subscription domain needed
vpn-admin user links <user_id>
```

See `docs/RECOVERY.md` for the full disaster-recovery procedure.

## Next logical checkpoint

Push the first real `vX.Y.Z` release tag (exercises `release.yml` +
the new checksum-verified bootstrap path for the first time for real),
then run `deploy/almalinux/lifecycle-acceptance.sh` against a real
disposable AlmaLinux 9 host AND, on that same host, work through
`docs/DEVICE_ACCEPTANCE_TESTS.md` with at least one real Hiddify device.
This remains the single highest-value gap across every checkpoint:
SSH preservation, rollback, idempotency, ACME restoration, config
migration, offline uninstall, release-integrity verification, AND real
client connectivity are all code-read/unit/fixture verified only, never
exercised end-to-end. Until then, pick one concrete supported-product
defect/gap and fix it with minimum churn, running
`deploy/lib/fast-gate.sh` after every change.
