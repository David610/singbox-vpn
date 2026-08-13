# Implementation Status (v1.0 baseline)

Compact engineering handoff. Full boundary definition: `docs/SUPPORTED_PRODUCT.md`.
Read that file first; do not re-audit the repo from scratch.

## Supported scope (summary)

AlmaLinux 9 x86_64, <=10 users, one VPS, upstream sing-box, VLESS+REALITY
TCP/443 primary, Hysteria2 UDP/443 optional, Hiddify/sing-box-compatible
clients only, custom domain default. Native adaptive stack
(`client-daemon`, `transport-native`, `policy`, `failure-classifier`,
`network-state`, `rendezvous-client`, `telemetry`, `services/rendezvous`,
`services/relay-agent`) is a separate, out-of-scope product surface for
v1.0 — see `deploy/local/run-dev-slice.sh` for its dev-only entry point.

## Supported file/test surface (avoid full-workspace runs when only this changed)

- Installer: `install.sh` (bootstrap) -> `deploy/almalinux/install.sh`
  (real implementation, builds `-p admin -p subscription` only).
- Uninstall: `uninstall.sh` -> `deploy/almalinux/uninstall.sh`.
- Update/repair/render: `deploy/almalinux/update.sh`, `render-config.sh`,
  `health-check.sh`, `acceptance-test.sh`, `certbot-deploy-hook.sh`.
- Shared shell libs: `deploy/lib/*.sh` plus `deploy/lib/versions.env`
  (one authoritative sing-box version/checksum/arch source).
- Rust crates in the supported dependency closure: `crates/common`,
  `crates/compat-config`, `apps/admin`, `services/subscription`.
- Rust tests: `crates/compat-config/tests/*` + `src/**` unit tests
  (interop, decoy budget, deployment/store schema+migration),
  `apps/admin/tests/*`, `services/subscription/tests/*`.
- Shell tests: `deploy/lib/tests/*.sh` (16 files).
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

## Completed checkpoints

1. v1.0 boundary + supported code surface documented
   (`docs/SUPPORTED_PRODUCT.md`); no Hiddify/native-engine doc corrections
   needed (already correctly scoped).
2. Canonical FAST GATE (`deploy/lib/fast-gate.sh`) and DESTRUCTIVE
   lifecycle gate (`deploy/almalinux/lifecycle-acceptance.sh`, SSH-only,
   requires explicit `--i-understand-this-is-destructive`) added. One
   tiny testability hook in install.sh (`VPN1_LIFECYCLE_GATE_ABORT_AFTER`,
   no-op by default).
3. Reproducible production installs: `deploy/lib/versions.env` is now the
   ONE source for sing-box version/checksums/arch (install.sh, CI,
   fast-gate.sh all read it). Bootstrap `install.sh`'s `resolve_version()`
   now refuses (die) to fall back to mutable branch source in the default
   channel when no release tag exists — `VPN1_CHANNEL=dev` is the only
   explicit opt-in. Install-state manifest records pinned sing-box
   version/checksum/arch. New test: `test-release-reproducibility.sh`.
4. **Persistent state versioning/migration** (this checkpoint):
   - `deployment.toml` gets `schema_version` (`#[serde(default)]` = 0 for
     every pre-existing file — still fully loadable; a version newer than
     `DEPLOYMENT_SCHEMA_VERSION` is refused at load, everywhere, fail-closed).
   - `users.json` moves from a bare JSON array to a versioned envelope
     `{"schema_version":1,"users":[...]}`. `load_users`/`parse_users_bytes`
     tolerate both shapes (subscription service must keep working through
     an upgrade window); only a genuinely newer-than-supported version is
     refused. Every `save_users_atomic` now self-heals the format.
   - New shared `crates/compat-config/src/migrate.rs`: backup (mode 0600)
     + atomic-write primitives used by both formats' migration.
   - New `vpn-admin config validate` (read-only; exit 0=OK, 2=MIGRATION_REQUIRED,
     3=INVALID) and `vpn-admin config migrate` (backup -> migrate -> validate
     incl. a real sing-box render+check when possible -> atomic commit;
     idempotent; fail-closed on corrupted/future-schema input, leaving the
     original untouched).
   - `install.sh` (`check_state_schema`, between `binaries_stage` and
     `reality_keys_stage`) and `update.sh` now explicitly report install
     mode (FRESH/REPAIR/UPGRADE) and auto-run `config migrate` on
     MIGRATION_REQUIRED, `die()` (rollback) on INVALID.
   - Fixed `cmd_restore` (previously raw-parsed the legacy bare-array
     shape only — would have rejected a backup taken from any
     current-format deployment) to use the same tolerant parser.
   - Found+fixed a real bug while testing: `check_state_schema`'s
     no-deployment.toml early exit used a bare `[ -f ... ] || return`,
     which returned the FAILED test's status (1) instead of success —
     would have aborted a legitimate no-op under `set -e`. Now `return 0`.
   - New tests: `crates/compat-config` unit tests (deployment.rs,
     store.rs — schema load/migrate/backup/idempotency/corruption/future-
     version, ~30 new cases), `apps/admin/tests/cli.rs` (9 new
     `config_*`/restore-envelope cases), `deploy/lib/tests/test-state-schema-migration.sh`
     (shell wiring, 15 checks). VERIFIED-TEST (all executed, all pass).

## Blockers

- `cargo test -p admin --test cli` has 5 failures **only when run as
  root** (this sandbox is root): `apply_restored_file_policy()` only does
  a real `getgrnam("vpn-subscription")` lookup when `geteuid()==0`, and
  that group doesn't exist outside a real installed host. Pre-existing
  (reproduced via `git stash` on prior commits); does not fail in CI
  (non-root runner). `deploy/lib/fast-gate.sh` therefore reports FAIL
  when run as root; PASS expected in CI/any non-root dev environment.

## Important UNVERIFIED items (require real VPS/network/client testing)

- Public reachability, provider firewall correctness.
- Real VLESS+REALITY / Hysteria2 connectivity from an actual Hiddify
  client on a real device.
- Full clean-VPS install -> update -> migrate -> rollback -> uninstall
  lifecycle, including SSH-preservation, on real AlmaLinux 9 (no
  disposable host available this session — `lifecycle-acceptance.sh` not
  executed; migration correctness above is fixture/unit-test-verified
  only, not exercised against a real upgraded deployment).
- DNS leak prevention, IPv6 correctness, WebRTC, kill-switch behavior,
  censorship resistance against a real adversary.

## Canonical verification commands (supported product)

```bash
# FAST GATE — one command, run after every change (see Blockers re: root)
bash deploy/lib/fast-gate.sh

# DESTRUCTIVE lifecycle gate — disposable AlmaLinux 9 host ONLY, over SSH
./deploy/almalinux/lifecycle-acceptance.sh \
  --host root@DISPOSABLE-HOST --i-understand-this-is-destructive

# state schema validate/migrate (new this checkpoint)
vpn-admin --config /etc/vpn/deployment.toml config validate
vpn-admin --config /etc/vpn/deployment.toml config migrate
```

## Next logical checkpoint

Prompt 5: push the first real `vX.Y.Z` release tag so the default
`curl | sudo bash` one-liner has something to resolve, then exercise
`config migrate`/install-mode detection against a real disposable
AlmaLinux 9 host via `lifecycle-acceptance.sh`. Until then, pick one
concrete supported-product defect/gap and fix it with minimum churn,
running `deploy/lib/fast-gate.sh` after every change.
