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
- Clients, in two explicit tiers:
  - **PRIMARY, first-party: `singbox-client`**
    (<https://github.com/David610/singbox-client>), a separate
    repository. It consumes the versioned provisioning contract at
    `GET /v1/provision/{token}` — see
    `docs/PROVISIONING_CONTRACT.md`. This is the client the
    server/client relationship is designed around.
  - **FALLBACK, third-party: Hiddify** (Android/iOS/Linux/MagicOS) and
    other sing-box-compatible importers documented under `docs/clients/`
    (currently Hiddify iOS/Linux/MagicOS, v2rayNG Android). They consume
    the legacy `GET /sub/{token}` share-link and sing-box-JSON formats,
    which remain supported. Only device-verified behaviour is claimed
    for them — see `docs/CLIENT_COMPATIBILITY.md` and
    `docs/DEVICE_ACCEPTANCE_TESTS.md`.

  No custom client software is *required* for the fallback path; the
  server does not run any client software itself for either tier.
- Custom domain by default; an IP-derived/convenience hostname
  (sslip.io-style) is available only through explicit opt-in, never the
  silent default for a real deployment.
- **One-command install path, CODE/CI-VERIFIED** (`install.sh` -> hands off to
  `deploy/almalinux/install.sh`) and **one-command complete offline
  uninstall** (`uninstall.sh` -> `deploy/almalinux/uninstall.sh`), which
  removes everything the installer created.

## Server support matrix

Authoritative. Every tier below reflects actual exercised evidence, not
implementation intent — `deploy/lib/os.sh` recognizing an OS ID is
necessary but never sufficient for a tier above UNSUPPORTED. Tier
definitions:

- **SUPPORTED TARGET** — the v1.0 target and primary CI target. This names the
  intended support boundary; it does not imply current HEAD has a complete
  real-host acceptance record. Runtime evidence lives only in the canonical
  ledger.
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
| AlmaLinux 9 x86_64 | **SUPPORTED TARGET** | CI validates the configuration/data-plane path. A real-VPS install + Hiddify smoke pass was USER-REPORTED on 2026-08-16, but its commit/device/client/transport details were not recorded; a fresh install of current HEAD is UNVERIFIED. See `docs/DEVICE_ACCEPTANCE_TESTS.md`. |
| Amazon Linux 2023 | CI-TESTED | `deploy/lib/tests/test-amazon-linux-2023.sh` exercises the real `detect_os()`/`install_dependencies_rhel()` functions (curl-minimal handling, no-EPEL certbot path) against fixtures in every CI run. No live AL2023 host has been verified. |
| Rocky Linux 9 | RECOGNIZED / BEST-EFFORT | Identical `rhel`-family code path to AlmaLinux 9 (same package list, same EPEL/certbot and firewalld-SSH-safe logic) but no independent live-host or CI verification. |
| Ubuntu 22.04 LTS / 24.04 LTS | RECOGNIZED / BEST-EFFORT | Dedicated `debian`-family code path (apt/ufw) exists and is covered by an L1/L2 container CI matrix (OS detection + real package-manager dependency installation — see `.github/workflows/ci.yml`'s `os-matrix` job); no live-host systemd/firewall install has been verified. |
| Debian 12 / 13 | RECOGNIZED / BEST-EFFORT | Same as Ubuntu above. |
| RHEL 9 | RECOGNIZED / BEST-EFFORT | Falls into the same `rhel`-family branch generically; no dedicated fixture or live verification. Real RHEL also needs a subscription for EPEL-equivalent content the installer does not configure. |
| CentOS Stream 9 | RECOGNIZED / BEST-EFFORT | Explicit `centos` case in `os.sh`'s rhel-family bucket; no dedicated fixture or live verification. |
| Oracle Linux 9 | UNSUPPORTED | No explicit `ol` case in `os.sh` — falls through to the generic `ID_LIKE=fedora` fallback only, classified `OS_SUPPORT=untested`. No coverage of any kind. |
| Anything else | UNSUPPORTED | No guarantee; `detect_os()` fails loudly for anything outside the `rhel`/`debian` family shapes above. |

**Architecture** is a separate dimension from OS: **amd64/x86_64 is the
only SUPPORTED TARGET architecture.** arm64 has a working `detect_arch()`
implementation path (source-build fallback, pinned sing-box/cosign
downloads), but as of the v1.0.0-rc.4 release-pipeline fix,
`.github/workflows/release.yml` no longer builds or publishes a
prebuilt arm64 singbox-vpn binary artifact at all — the previous
workflow cross-compiled one and published it despite having no way to
execute or validate it on the (x86_64) build runner, i.e. "cross-
compilation succeeded" was never evidence it actually ran anywhere. An
arm64 host still installs via `install.sh`'s automatic source-build
fallback (`fetch_release_binaries()` returns nothing to install, and
the installer falls back to `cargo build` from source), it just never
gets a prebuilt binary. arm64 remains RECOGNIZED / BEST-EFFORT, not
SUPPORTED, regardless of OS tier; publishing it again requires the same
real ABI-baseline + runtime-execution validation x86_64 now has (see
"Binary ABI baseline" below).

### Binary ABI baseline (x86_64)

This is a **build-environment property**, distinct from the OS support
tiers above — it does not make AlmaLinux 8 a supported OS.

The published x86_64 `singbox-vpn-x86_64-unknown-linux-gnu.tar.gz`
release artifact (`vpn-admin`, `subscription`) is compiled inside a
pinned AlmaLinux 8.10 container (glibc 2.28), not on the GitHub-hosted
`ubuntu-latest` runner's own (much newer, and unpinned/drifting) host
glibc. `.github/workflows/release.yml`'s `build` job enforces, before
any artifact is published, that both binaries' maximum required
`GLIBC_*` symbol version is `<= 2.28`
(`deploy/lib/check-glibc-baseline.sh`), and a separate `runtime-compat`
job actually extracts and executes the exact packaged archive inside
clean AlmaLinux 8 and AlmaLinux 9 containers before `publish` is allowed
to run.

This baseline exists because:

- a real v1.0.0-rc.3 VPS install failed with `GLIBC_2.39' not found` —
  the release binaries had silently inherited whatever glibc the build
  runner happened to ship, which changes over time and was never
  pinned or verified against anything;
- AlmaLinux 9 (glibc 2.34) is the v1.0 SUPPORTED TARGET — a binary
  requiring anything newer than glibc 2.34 could not run there either;
- glibc is backward compatible (an old-glibc-linked binary runs fine on
  a newer-glibc host), so a conservative `<= 2.28` baseline is a strict
  superset of "runs on AlmaLinux 9" and additionally happens to run on
  AlmaLinux 8 as a best-effort extra data point — it is not itself a
  claim that AlmaLinux 8 is supported;
- making this an explicit, tested, pinned build property (rather than
  whatever `ubuntu-latest` happens to ship this month) makes release
  portability a deliberate, reproducible decision instead of an
  accident that only surfaces on a real customer VPS.

`OS_SUPPORT` in `deploy/lib/os.sh` is an internal implementation-coverage
classification used for installer warnings — it uses the same evidence
buckets in spirit (`tested`/`ci-tested`/`untested`) but is not phrased
identically to this table's public tier names. This table is authoritative
whenever the two differ in phrasing; they must not differ in substance.

## Explicitly out of scope for v1.0

- **No custom client software in this repository.** The compatibility
  path never requires running this repo's own Rust daemon on an end-user
  device. `singbox-client` is the first-party client, but it is a
  separate repository with its own release cycle — this repository ships
  only the contract it consumes (`docs/PROVISIONING_CONTRACT.md`) and
  the fixtures its CI tests against
  (`fixtures/singbox-client-contract/`).
- **No new VPN protocols.** Production transports are exactly
  VLESS+REALITY and Hysteria2 (+ Salamander obfuscation where
  configured). The provisioning contract's transport/capability
  vocabulary is extensible so a future capability is a compatible schema
  change, but nothing — NaiveProxy, AnyTLS, TUIC, Shadowsocks,
  WireGuard, a second REALITY implementation — is in v1.0 scope.
- **No v1.0 multi-node control plane / fleet manager.** One VPS only.
- **No anonymity or global-adversary guarantee.** This is a private
  circumvention/privacy tool for a small trusted group, not a Tor-class
  anonymity system — see `docs/THREAT_MODEL.md` for the actual threat
  boundary.
- **The native adaptive stack has been REMOVED from `main`.** It used to
  live at `apps/client-daemon`, `apps/cli`, `apps/keytool`,
  `crates/{crypto,config,network-state,transport-api,failure-classifier,
  policy,transport-native,rendezvous-client,telemetry}`,
  `services/{rendezvous,relay-agent,test-service}`, and the top-level
  `tests/` crate — a direct-tls/noise-quic adaptive-transport system
  with its own scoring engine, driven only via a local dev slice. It was
  never in the v1.0 supported dependency closure (the v1.0 installer,
  `deploy/almalinux/install.sh`, never built, deployed, or depended on
  any of it), so removing it changes no runtime/install behavior. Its
  full implementation, history, and design docs (including all of
  `docs/ADR/`) are preserved on the `archive/native-adaptive-stack-2026`
  branch, not deleted.
- Hiddify and other sing-box-compatible clients use **sing-box's own**
  `urltest` transport selection. The native stack's scoring engine that
  used to exist alongside it is gone; do not describe Hiddify traffic as
  using any Rust adaptive engine.

## Supported code surface (what the installer actually builds/deploys)

VERIFIED-CODE (`deploy/almalinux/install.sh` line ~794 and its top-of-file
scope comment; crate `Cargo.toml` dependency graphs):

- Binaries built: `cargo build --release -p admin -p subscription` only.
- Crate dependency closure of the supported path: `apps/admin`,
  `services/subscription` -> `crates/common`, `crates/compat-config`,
  `crates/provisioning-contract`. This is now also the ENTIRE workspace
  member list — there is no longer a separate default-members subset to
  keep in sync with it.
- Shell: `install.sh`, `uninstall.sh`, `deploy/almalinux/*.sh`,
  `deploy/lib/*.sh`.
- Templates: `deploy/almalinux/templates/*.template`.
- Tests: see `docs/IMPLEMENTATION_STATUS.md` for the exact list.

`crates/provisioning-contract` is a leaf crate: pure types and
validation for the first-party contract, with no dependency on anything
else in this workspace. `crates/compat-config` depends on it, not the
other way around, so the contract model cannot pick up server-side
concerns (key files, paths, secrets) by accident.
