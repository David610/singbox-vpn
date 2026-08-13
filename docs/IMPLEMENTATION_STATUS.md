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

## Blockers

None for this checkpoint.

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
# shell syntax + targeted shell tests
bash -n install.sh uninstall.sh deploy/almalinux/*.sh deploy/lib/*.sh
bash deploy/lib/tests/test-*.sh   # individually, or the set CI runs

# supported-crate build/test only (no full workspace)
cargo build --locked -p admin -p subscription -p compat-config -p common
cargo test --locked -p compat-config -p admin -p subscription

# real sing-box protocol validation (mirrors CI singbox-validate job)
cargo test --locked -p compat-config --test reality_interop \
  --test reality_decoy_budget --test hysteria2_interop -- --nocapture
```

## Next logical checkpoint

Prompt 2: pick one concrete supported-product defect or gap (not a new
feature) and fix it with minimum churn, using this file to skip
re-discovery.
