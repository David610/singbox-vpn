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
