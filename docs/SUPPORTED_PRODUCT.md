# Supported v1.0 Product

This document is the authoritative boundary of what v1.0 ships. When any
other doc (README, ARCHITECTURE, PLAN, ADRs) conflicts with this file on
scope, this file wins for v1.0 decisions.

## In scope

- Private VPN for personal use: **<=10 trusted users** (friends/family).
- Server OS: see the **Server support matrix** below.
  AlmaLinux 9 x86_64 is the one CI validates end-to-end with a real
  `sing-box` binary — see `singbox-validate` in `.github/workflows/ci.yml`.
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

## Server support matrix

Authoritative. Every tier below reflects actual exercised evidence, not
implementation intent — `deploy/lib/os.sh` recognizing an OS ID is
necessary but never sufficient for a tier above UNSUPPORTED. Tier
definitions:

- **SUPPORTED** — the v1.0 target: exercised end-to-end (a real host
  install, not just fixtures) and the primary CI target.
- **CI-TESTED** — automated tests exercise the real `detect_os()` /
  `install_dependencies_*()` functions against fixtures/containers in
  CI, but no live-host install has been observed to succeed.
- **RECOGNIZED / BEST-EFFORT** — `deploy/lib/os.sh` has a dedicated
  implementation path (package manager, firewall backend, family-specific
  handling) and the installer will attempt a full install, but there is no
  dedicated automated coverage and no live-host verification.
- **UNSUPPORTED** — no dedicated coverage of any kind. The installer may
  still attempt a generic `ID_LIKE`-based install path and warn loudly, or
  may refuse outright.

| Distribution | Tier | Evidence |
|---|---|---|
| AlmaLinux 9 x86_64 | **SUPPORTED** | Owner-reported real-VPS install + Hiddify smoke pass (2026-08-16); primary target of every install stage; `singbox-validate` CI job builds/validates the real data plane on every push. |
| Amazon Linux 2023 | CI-TESTED | `deploy/lib/tests/test-amazon-linux-2023.sh` exercises the real `detect_os()`/`install_dependencies_rhel()` functions (curl-minimal handling, no-EPEL certbot path) against fixtures in every CI run. No live AL2023 host has been verified. |
| Rocky Linux 9 | RECOGNIZED / BEST-EFFORT | Identical `rhel`-family code path to AlmaLinux 9 (same package list, same EPEL/certbot and firewalld-SSH-safe logic) but no independent live-host or CI verification. |
| Ubuntu 22.04 LTS / 24.04 LTS | RECOGNIZED / BEST-EFFORT | Dedicated `debian`-family code path (apt/ufw) exists and is covered by an L1/L2 container CI matrix (OS detection + real package-manager dependency installation — see `.github/workflows/ci.yml`'s `os-matrix` job); no live-host systemd/firewall install has been verified. |
| Debian 12 / 13 | RECOGNIZED / BEST-EFFORT | Same as Ubuntu above. |
| RHEL 9 | RECOGNIZED / BEST-EFFORT | Falls into the same `rhel`-family branch generically; no dedicated fixture or live verification. Real RHEL also needs a subscription for EPEL-equivalent content the installer does not configure. |
| CentOS Stream 9 | RECOGNIZED / BEST-EFFORT | Explicit `centos` case in `os.sh`'s rhel-family bucket; no dedicated fixture or live verification. |
| Oracle Linux 9 | UNSUPPORTED | No explicit `ol` case in `os.sh` — falls through to the generic `ID_LIKE=fedora` fallback only, classified `OS_SUPPORT=untested`. No coverage of any kind. |
| Anything else | UNSUPPORTED | No guarantee; `detect_os()` fails loudly for anything outside the `rhel`/`debian` family shapes above. |

**Architecture** is a separate dimension from OS: **amd64/x86_64 is the
only SUPPORTED architecture.** arm64 has a working `detect_arch()` and
release-build implementation path (release tooling publishes arm64
binaries), but no live arm64 host install has been verified — treat it as
RECOGNIZED / BEST-EFFORT, not SUPPORTED, regardless of OS tier.

`OS_SUPPORT` in `deploy/lib/os.sh` is an internal implementation-coverage
classification used for installer warnings — it uses the same evidence
buckets in spirit (`tested`/`ci-tested`/`untested`) but is not phrased
identically to this table's public tier names. This table is authoritative
whenever the two differ in phrasing; they must not differ in substance.

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
