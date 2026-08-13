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

- Installer: `install.sh` (bootstrap) -> `deploy/almalinux/install.sh` (real
  implementation, builds `-p admin -p subscription` only).
- Uninstall: `uninstall.sh` -> `deploy/almalinux/uninstall.sh`.
- Update/repair/render: `deploy/almalinux/update.sh`, `render-config.sh`,
  `health-check.sh`, `acceptance-test.sh`, `certbot-deploy-hook.sh`.
- Shared shell libs: `deploy/lib/*.sh` (os detection, ownership, preflight,
  perf-tuning, secret-logging check).
- Rust crates in the supported dependency closure: `crates/common`,
  `crates/compat-config`, `apps/admin`, `services/subscription`.
- Templates: `deploy/almalinux/templates/*.template`.
- Rust tests: `crates/compat-config/tests/*` (reality/hysteria2 interop,
  decoy budget), `apps/admin/tests/*` (cli, startup_validation),
  `services/subscription/tests/*`.
- Shell tests: `deploy/lib/tests/*.sh` (14 files — os/ownership/preflight/
  perf-tuning/idempotency/parity/IDN/etc.).
- CI gates (`.github/workflows/ci.yml`): `fmt`, `clippy`, `test` (currently
  workspace-wide — see "Known CI scope note" below), `audit`, `docs`,
  `shell`, `secret-logging-check`, `license-check`, `singbox-validate`
  (real pinned `sing-box` binary + real REALITY/Hysteria2 handshake via
  `vpn-admin doctor --protocol`).

## Known CI scope note (documented, not changed this pass)

`fmt`, `clippy --workspace`, `cargo build/test --workspace` currently
build the whole workspace, including the out-of-scope native adaptive
crates. `singbox-validate` already builds/tests only the supported crates
(`-p admin`, `-p subscription`, `-p compat-config`) and is the canonical
supported-product gate. Splitting the workspace-wide jobs to skip the
native crates would touch CI trust/release plumbing (out of the allowed
change set for this checkpoint — see task DO NOT list) for a
runner-minutes optimization only; left for a later, explicitly-scoped
checkpoint if it becomes a real bottleneck.

## Completed checkpoints

- v1.0 boundary and supported code surface identified and documented
  (`docs/SUPPORTED_PRODUCT.md`), VERIFIED-CODE from `deploy/almalinux/install.sh`
  scope comment + `cargo build --release -p admin -p subscription` line +
  `Cargo.toml` dependency graphs of `apps/admin`, `services/subscription`,
  `crates/compat-config`, `crates/common` (none depend on the native stack).
- Reviewed user-facing Hiddify/native-engine claims (README.md,
  `docs/ALMALINUX_DEPLOYMENT.md`, `docs/COMPATIBILITY_VERSIONS.md`):
  already correctly scoped — README explicitly states Hiddify uses
  sing-box's own `urltest` selector, not this repo's native `policy`
  engine, and the two "native sing-box" mentions in the other docs mean
  unmodified upstream sing-box, not the native Rust adaptive stack. No
  correction needed. VERIFIED-CODE (read, not modified).
- Added `deploy/lib/fast-gate.sh`: the canonical FAST GATE — one command
  covering shell syntax/shellcheck, secret-logging check, supported-crate
  fmt/clippy/test, all `deploy/lib/tests/*.sh` fixtures, and a real
  render-config + `sing-box check` against the same pinned version/hash
  `install.sh` uses (extracted from `install.sh` at run time, never
  hand-duplicated). VERIFIED-TEST (executed; see Blockers for the one
  known failing subset).
- Added `deploy/almalinux/lifecycle-acceptance.sh`: the canonical
  DESTRUCTIVE lifecycle gate for a disposable AlmaLinux 9 host over SSH
  (install/SSH/reboot/repair/update/user-lifecycle/protocol/interrupted-
  install/uninstall/reinstall). Requires `--host` (non-localhost,
  non-production) and `--i-understand-this-is-destructive`; refuses
  otherwise. Not executed against a real host this checkpoint (none
  available) — UNVERIFIED, safety-refusal logic only was validated
  (VERIFIED-TEST: `--host` omitted, `--host localhost`, and missing-ack
  all correctly `die` before touching anything).
- Added one tiny testability hook to `deploy/almalinux/install.sh`:
  `lifecycle_gate_abort_hook` (env `VPN1_LIFECYCLE_GATE_ABORT_AFTER`),
  called once after `singbox_install_stage`; no-op unless that env var is
  explicitly set, used only by `lifecycle-acceptance.sh` stage 10.
  Real install/uninstall behavior otherwise unchanged.

## Blockers

- `cargo test -p admin --test cli` has 4 pre-existing failures
  (`backup_then_restore_round_trips_users`,
  `backup_then_restore_round_trips_hysteria2_obfuscation_password`,
  `restore_never_widens_permissions_on_restored_secrets`,
  `restore_of_differing_reality_key_restarts_subscription_service_too`)
  **only when the test process runs as root** (this session's sandbox is
  root): `apply_restored_file_policy()` in `apps/admin/src/main.rs` only
  does a real `getgrnam("vpn-subscription")` lookup when `geteuid() == 0`
  (apps/admin/src/main.rs:1427), and that group does not exist outside a
  real installed host. VERIFIED-CODE this is pre-existing and unrelated
  to this checkpoint's changes: reproduced identically via `git stash` on
  the prior commit. GitHub Actions' `test` job runs as a non-root
  runner user, so this does not fail there. Not fixed here (would mean
  changing production restore code, out of this checkpoint's scope).
  `deploy/lib/fast-gate.sh` therefore reports FAIL when run as root; it
  is expected to report PASS in CI / any non-root dev environment. Left
  for a later checkpoint to either add a root-aware test skip or a
  fixture group.

## Important UNVERIFIED items (require real VPS/network/client testing)

- Public reachability, provider firewall correctness.
- Real VLESS+REALITY / Hysteria2 connectivity from an actual Hiddify
  client on a real device.
- iOS/Android/MagicOS-specific behavior beyond `docs/clients/`.
- DNS leak prevention, IPv6 correctness, WebRTC, kill-switch behavior.
- Censorship resistance against a real adversary.
- Full clean-VPS install -> update -> rollback -> uninstall lifecycle,
  including SSH-preservation invariant, on AlmaLinux 9 (CI's
  `singbox-validate` covers protocol correctness on Ubuntu runners, not
  the AlmaLinux 9 install/uninstall lifecycle itself).

## Canonical verification commands (supported product)

```bash
# FAST GATE — one command, run after every change (see Blockers re: root)
bash deploy/lib/fast-gate.sh

# DESTRUCTIVE lifecycle gate — disposable AlmaLinux 9 host ONLY, over SSH
./deploy/almalinux/lifecycle-acceptance.sh \
  --host root@DISPOSABLE-HOST --i-understand-this-is-destructive

# individual pieces fast-gate.sh wraps, if isolating a failure:
bash -n install.sh uninstall.sh deploy/almalinux/*.sh deploy/lib/*.sh
cargo build --locked -p admin -p subscription -p compat-config -p common
cargo test --locked -p compat-config -p admin -p subscription
cargo test --locked -p compat-config --test reality_interop \
  --test reality_decoy_budget --test hysteria2_interop -- --nocapture
```

## Next logical checkpoint

Prompt 3: pick one concrete supported-product defect or gap (not a new
feature) and fix it with minimum churn, using `deploy/lib/fast-gate.sh`
after every change instead of re-inventing ad-hoc checks.
