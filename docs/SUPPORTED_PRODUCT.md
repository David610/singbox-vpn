# Supported v1.0 Product

This document is the authoritative boundary of what v1.0 ships. When any
other doc (README, ARCHITECTURE, PLAN, ADRs) conflicts with this file on
scope, this file wins for v1.0 decisions.

## In scope

- Private VPN for personal use: **<=10 trusted users** (friends/family).
- Server OS: **AlmaLinux 9 x86_64 only** is the tested/supported target.
  (`deploy/lib/os.sh` also branches for Rocky/RHEL/Debian/Ubuntu/Amazon
  Linux 2023, but AlmaLinux 9 x86_64 is the one CI validates end-to-end
  with a real `sing-box` binary — see `singbox-validate` in
  `.github/workflows/ci.yml`.)
- Default topology: **one VPS**. No v1.0 multi-node control plane or
  fleet orchestration.
- Data plane: **upstream, unmodified `sing-box`** (pinned version,
  checksum-verified at install time).
- Primary transport: **VLESS + REALITY over TCP/443**.
- Secondary transport: **Hysteria2 over UDP/443**, optional.
- Clients: **Hiddify** (Android/iOS/Linux/MagicOS) and other
  sing-box-compatible clients that are actually documented/tested under
  `docs/clients/` (currently Hiddify iOS/Linux/MagicOS, v2rayNG Android).
  No custom client software is required or shipped for this path.
- Custom domain by default; an IP-derived/convenience hostname
  (sslip.io-style) is available only through explicit opt-in, never the
  silent default for a real deployment.
- **One-command verified install** (`install.sh` -> hands off to
  `deploy/almalinux/install.sh`) and **one-command complete offline
  uninstall** (`uninstall.sh` -> `deploy/almalinux/uninstall.sh`), which
  removes everything the installer created.

## Explicitly out of scope for v1.0

- **No custom client software.** The compatibility path never requires
  running this repo's own Rust daemon on an end-user device.
- **No v1.0 multi-node control plane / fleet manager.** One VPS only.
- **No anonymity or global-adversary guarantee.** This is a private
  circumvention/privacy tool for a small trusted group, not a Tor-class
  anonymity system — see `docs/THREAT_MODEL.md` / `docs/PRIVACY_MODEL.md`
  for the actual threat boundary.
- **The native adaptive stack is a separate, unsupported-for-v1.0
  product surface**: `apps/client-daemon`, `crates/transport-native`,
  `crates/policy`, `crates/failure-classifier`, `crates/network-state`,
  `crates/rendezvous-client`, `crates/telemetry`, `services/rendezvous`,
  `services/relay-agent`. These implement a direct-tls/noise-quic
  adaptive-transport system with its own scoring engine, driven only via
  `deploy/local/run-dev-slice.sh` (loopback dev slice). The v1.0
  installer (`deploy/almalinux/install.sh`) does not build, deploy, or
  depend on any of this — see "Supported code surface" below,
  VERIFIED-CODE.
- Hiddify and other sing-box-compatible clients use **sing-box's own**
  `urltest` transport selection, never this repo's native `policy`
  scoring engine. The two adaptive systems are unrelated; do not describe
  Hiddify traffic as using the native Rust adaptive engine.

## Supported code surface (what the installer actually builds/deploys)

VERIFIED-CODE (`deploy/almalinux/install.sh` line ~794 and its top-of-file
scope comment; crate `Cargo.toml` dependency graphs):

- Binaries built: `cargo build --release -p admin -p subscription` only.
- Crate dependency closure of the supported path: `apps/admin`,
  `services/subscription` -> `crates/common`, `crates/compat-config`.
  Neither depends on `transport-native`, `policy`, `failure-classifier`,
  `network-state`, `rendezvous-client`, or `telemetry`.
- Shell: `install.sh`, `uninstall.sh`, `deploy/almalinux/*.sh`,
  `deploy/lib/*.sh`.
- Templates: `deploy/almalinux/templates/*.template`.
- Tests: see `docs/IMPLEMENTATION_STATUS.md` for the exact list.

Everything else in `crates/` and `apps/` (`client-daemon`, `cli`,
`keytool`, `transport-native`, `policy`, `failure-classifier`,
`network-state`, `rendezvous-client`, `telemetry`) and
`services/rendezvous`, `services/relay-agent` belong to the native
adaptive stack and are out of the v1.0 supported path.
