# Platform v2 — current state audit

**Audit date:** 2026-09-15
**Server repository:** `singbox-vpn` at `main` = `0c9d37e` (PR #82). This audit was the first step of branch `feat/platform-v2`.
**Client repository inspected:** `tamara-next` at `c8033f3` (local worktree `D:\ISDA\tq-c8033f3`; the local `tamara-next` main checkout is 71 commits behind origin and was not used).

This file records facts only, each with the evidence behind it. Target architecture is in [TARGET_ARCHITECTURE.md](TARGET_ARCHITECTURE.md). Anything not proven here is marked UNVERIFIED.

Evidence vocabulary: CODE-VERIFIED, CI-VERIFIED, LOCAL-VERIFIED, VPS-VERIFIED, DEVICE-VERIFIED, NETWORK-VERIFIED, UNVERIFIED, BLOCKED. They are defined in [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md#evidence-vocabulary). Older documents also use SERVER-VERIFIED and USER-REPORTED; read SERVER-VERIFIED as VPS-VERIFIED.

## 1. Where the task's stated baseline was wrong

The task stated an expected baseline. Four points differ from what the repositories actually contain:

| Stated assumption | Actual state | Evidence |
|---|---|---|
| Tamara is "a Flutter client derived from Hiddify … Hiddify Core based" | **The current client, `tamara-next`, is a clean-room rewrite.** It has a separate root commit, a provenance gate (`tool/verify_clean_boundary.py`), and uses direct SagerNet sing-box `v1.14.0` (`0b89958`) with Tamara patches. Only the *legacy* `David610/tamara` repository is Hiddify-derived; it is kept as a behavioural reference, and its GitHub archival is still pending. | `tamara-next/PROVENANCE.md`, `docs/legacy-retirement.md`, `docs/provenance/core.md` |
| "Two-hop functionality is experimental/code-verified" | Relay/two-hop is **VPS-VERIFIED on two real VPSs** (RU relay → DE exit, commit `0c9d37e`, 2026-09-13). A real Windows client then completed the **DEVICE-VERIFIED** Tamara B15 run on 2026-09-15: 77 PASS / 0 FAIL / 1 INCONCLUSIVE (environmental) / 4 NOT_RUN (sleep) / 3 BLOCKED (signing). | `acceptance/2026-09-13-privacy-plus-real-vps/remediation/`, Tamara `…6029a0b-device/DEVICE-RECORD-FINAL.md` |
| "peer/multi-VPS behaviour not fully verified on real independent VPSs" | Partly outdated. Two independent providers (RU, DE) were used for the relay→exit route. **Direct multi-exit failover** (client switching between two independent exits) is still UNVERIFIED. | same |
| Hiddify-derived licensing is the main commercial blocker for Tamara | For `tamara-next` the open item is **Tamara's own licence** (`NOASSERTION`) combined with **GPL-3.0-or-later sing-box** and **LGPL-3.0 WinDivert** distribution obligations, including App Store compatibility. The Hiddify concern now applies only to the legacy repository and to legacy-data migration code. | `tamara-next/docs/provenance/licenses.md` |

## 2. Server product (singbox-vpn)

### 2.1 Code surface

| Component | Role | Size (lines) |
|---|---|---|
| `crates/provisioning-contract` | Leaf crate: the v1 first-party contract types and validation | 1.8k |
| `crates/compat-config` | Deployment/user models, credential generation, server and client rendering, relay policy, contract assembly | 6.7k + 4.9k tests |
| `crates/common` | Small shared types (framing) | 0.3k |
| `apps/admin` (`vpn-admin` / `vpn`) | Operator CLI: init, render-config, status, doctor, backup/restore, repair, user and peer management, config validate/migrate | 9.6k + 4k tests |
| `services/subscription` | axum HTTP service behind nginx: `/sub/{token}` (fallback formats), `/v1/provision/{token}` (Tamara) | 2.3k |
| `install.sh`, `deploy/almalinux/*.sh`, `deploy/lib/*.sh` | Bootstrap, installer, transactional updater, uninstall, certbot hooks, firewall, watchdog, benchmark, investigate | 35 shipped shell scripts, 51 shell test files |

The workspace has exactly these five crates. The older native adaptive stack (its own scoring engine, rendezvous service and relay agent) was removed from `main` and is preserved only on `archive/native-adaptive-stack-2026`. Platform v2 must not assume that code exists.

### 2.2 Data plane

- **Upstream, unmodified sing-box `1.13.19`**, pinned by SHA-256 in `deploy/lib/versions.env` and verified at install. CODE-VERIFIED, CI-VERIFIED, VPS-VERIFIED.
- **VLESS + REALITY over TCP** (default 443) with XTLS Vision. CI-VERIFIED with real sing-box interop (`crates/compat-config/tests/reality_interop.rs`), VPS-VERIFIED.
- **Hysteria2 over UDP** (default 443) with an optional deployment-wide Salamander password and file masquerade. CI-VERIFIED (`hysteria2_interop.rs`).
- **No AmneziaWG, WireGuard or other transport.** `docs/SUPPORTED_PRODUCT.md` explicitly lists "no new VPN protocols" as out of scope for v1.0.
- The server config is produced by `server::render_server_config_for_deployment`, the only production renderer (enforced by a test). An exit gets unrestricted `direct` egress; a relay gets an exact destination allowlist ending in `reject`.
- A `CompatibilityBackend` trait exists (`server.rs`), but it only abstracts `validate`/`apply` of one sing-box JSON document. It is **not** a transport abstraction: transport-specific logic lives in `render.rs`, `contract.rs`, `server.rs`, `deployment.rs`, `apps/admin/src/main.rs` and `services/subscription/src/lib.rs`.

### 2.3 Endpoint, peer and relay model

- `CompatEndpoint` has `id`, `transport`, `host`, `port`, `server_name`, `label`, public parameters, `origin` (Local/Peer), `failure_domain`, `region`, `provider`, `asn`, `path` and `credential_ref`.
- Local endpoints are fixed: `reality-1` and `hysteria2-1`.
- `[[peer_endpoints]]` (ADR-0009, Option A) declares endpoints on independently operated servers. Credentials are **per user, per endpoint**, pasted in by the operator (`vpn-admin user peer set … --credential-stdin`). The server never generates, synchronises or health-checks peer credentials. A peer private key in the config is refused.
- `[[access_paths]]` with `kind = "relay"` and `via_endpoint_id` makes relay routes executable as Core `detour` chains. Validation rejects chains longer than two hops, UDP/Hysteria2 relays, and dangling or aliased `credential_ref` values.
- `deployment.toml` is at schema v2 (`node_id`, `role`). The config fails closed on a newer schema.
- `users.json` is at schema v1. Peer credentials are skipped when empty, so zero-peer files stay byte-identical.
- **No first-class node or route objects exist.** A "route" today is an endpoint plus an optional path reference. The data model cannot express N nodes, each with several transports, as one fleet: every node is administered separately.

### 2.4 Provisioning contract (v1)

- `GET /v1/provision/{token}` returns schema_version 1: `server`, `capabilities`, `experimental_capabilities`, `endpoints`, optional `access_paths`, and the embedded `singbox_config`.
- An unsupported version (`/v2/…` or `?schema_version=2`) returns **HTTP 400 `unsupported_schema_version`**. CODE-VERIFIED.
- Validation requires the embedded `select` group to list **exactly** the endpoint tags plus `auto`. A transport that sing-box cannot express therefore cannot appear in a v1 document without changing that invariant.
- Fallback routes: `/sub/{token}` (sing-box JSON), `?format=hiddify|uri` (share links), `?format=singbox`, plus the diagnostic `compat=` and `profile=` switches.
- The bearer token is stored as a SHA-256 hash, rate limited, and every unknown token gets the same 404.

### 2.5 Health, failover and scoring

- **Server side:** `vpn-admin doctor` runs layered checks. L1 process, L2 config/key/cert, L3 listeners, L4 subscription coherence and `--protocol` L5/L6 (a real loopback REALITY handshake) are CODE-VERIFIED. A watchdog timer restarts failed services.
- **Client side (fallback clients):** sing-box `urltest` plus `selector`. Once relayed routes exist, `urltest` contains only relayed routes; this was the D3 remediation.
- **Client side (Tamara):** `ResiliencePlanner` (`lib/domain/resilience/resilience_planner.dart`) ranks candidates by operator priority, a bonus for recent success and a penalty for consecutive failures. It uses fixed backoff (1/3/10/30/60 s), a 5-minute health TTL, a 3-attempt limit, and never mixes Fast with Privacy+. It has **no RTT, jitter, loss, transfer-health, per-network history, hysteresis/probation, or typed failure classes**: `VpnFailureCode` is `unavailable | rejected | policyViolation | internal`.
- There is **no typed network-failure classification anywhere** on the server or the client.

### 2.6 Lifecycle

- Install: preflight → packages → firewall → sing-box (pinned and verified) → binaries (release artifacts with cosign attestation, or a source build) → certbot → nginx → first user → `doctor --protocol --require-protocol` → the `accepted` manifest. CODE/CI-VERIFIED; a fresh install of HEAD was VPS-VERIFIED on 2026-09-13 as part of the two-VPS run.
- Update: transactional STAGE→PREPARE→SWITCH→ACTIVATE→VERIFY→COMMIT with rollback. `--repair` isolates its test phase (D6 fix). VPS-VERIFIED for `--repair`.
- Backup/restore: a mode-0600 tar of users, REALITY keys and Hysteria2 material. Restore refuses a backup from another role or node. VPS-VERIFIED (L7).
- Uninstall: a complete offline uninstall with residue audit. CODE/CI-VERIFIED.
- Supply chain: pinned sing-box and cosign checksums, release SBOM (CycloneDX) plus licence inventory, and GitHub attestations. No `curl | bash` path runs unverified content after the bootstrap itself (see `docs/SUPPLY_CHAIN_SECURITY.md`).

### 2.7 Measurement and investigation tooling

- `deploy/lib/vpn-benchmark.sh`: host, network path, raw VPS throughput, and same-host REALITY/Hysteria2 protocol overhead, with JSON output. It is explicitly **not** a real-client path measurement.
- `deploy/lib/vpn-investigate.sh`: FACT/INFERENCE/UNKNOWN-labelled server-side investigation. It mutates nothing.
- **There is no probe agent, periodic measurement matrix, telemetry schema, or cross-node result store.**

### 2.8 Operations surface

`vpn-admin` / `vpn`: `init`, `render-config`, `version`, `status`, `doctor [--protocol|--json|--report|--performance|--client|--telegram]`, `backup`, `restore`, `repair`, `hysteria-obfs-rotate`, `user {create,list,qr,links,enable,disable,rotate-token,rotate-vless,rotate-hysteria,rotate-credentials,remove,peer {set,rotate,remove,list},vision-off-experiment,subscription}`, `config {validate,migrate}`.
Missing commands (the target surface in the task): `endpoint`, `node`, `transport`, `rotate` (top level), `benchmark` (exists only as a script), `rollback` (exists inside `update.sh`, not as a command).

## 3. Client product (tamara-next at c8033f3)

- Flutter app with a clean layered architecture: `domain`, `application`, `infrastructure`, `ports`, `presentation`.
- Engine boundary: `VpnEngine` port → `SingBoxVpnEngine` → a service-owned daemon (`boxdd`, patched upstream sing-box 1.14.0) on Windows/Linux, and NetworkExtension plus pinned `Library.xcframework` on Apple platforms. **The worktree has no `android/` runner directory**: runners exist for Windows, Linux, macOS and iOS. Android is therefore UNVERIFIED and, at repository level, unimplemented.
- The provisioning adapter accepts schema v1 only (`decoded['schema_version'] != 1` → hard failure). When building direct bundles it considers only the `vless-reality` and `hysteria2` transports and ignores others.
- Privacy+ (exactly two hops) never downgrades to Fast.
- Managed control-plane contract **proposal**: login/refresh, entitlement, an Ed25519-signed monotonic route directory, and pseudonymous short-lived VPN authorization. **No backend exists.**
- Telemetry: none. The app has no analytics, crash-upload or remote-logging SDKs, and Core logging is disabled (`docs/architecture/privacy.md`).
- Evidence:
  - Windows B15 physical qualification: **DEVICE-VERIFIED**, one machine, Wi-Fi and USB-tether uplinks, 2026-09-15.
  - Linux, macOS and iOS: UNVERIFIED.
  - Android/MagicOS: UNVERIFIED and not implemented.
  - Signing: BLOCKED (development signer only).

## 4. Evidence ledger summary (current, not target)

| Claim | Status |
|---|---|
| REALITY / Hysteria2 config interop with real sing-box | CI-VERIFIED |
| Relay fail-closed forwarding and two-hop detour chain | CI-VERIFIED (loopback S1–S17), VPS-VERIFIED (RU→DE, 2026-09-13) |
| Tamara Windows Fast and Privacy+ through real VPSs, including kill switch, crash, reboot, upgrade and network transition | DEVICE-VERIFIED (one Windows 10 machine) |
| Russian **datacenter** network as a client vantage point (RU VPS → DE) | VPS-VERIFIED for relay forwarding. **RUSSIAN-DATACENTER-NETWORK reachability to foreign exits was not systematically measured** (no probe matrix) → UNVERIFIED |
| Russian residential / mobile networks | UNVERIFIED |
| Direct multi-exit failover between independent providers | UNVERIFIED |
| AmneziaWG (any version) | not implemented (starting state) |
| Adaptive route scoring, typed failure classification, automatic transport switching | not implemented (starting state) |
| Automatic VPS provisioning / node replacement | not implemented (starting state) |
| Android / MagicOS / iOS client behaviour | UNVERIFIED |
| Commercial distribution of Tamara | BLOCKED (licence selection, GPL/LGPL obligations, signing) |

## 5. Licensing and provenance inventory (engineering facts, not legal conclusions)

| Component | Licence found in repository | Where it is distributed | Engineering note |
|---|---|---|---|
| singbox-vpn (this repository) | Apache-2.0 (`LICENSE`) | GitHub releases | Release SBOM plus licence inventory already generated |
| sing-box (server) | GPL-3.0-or-later (upstream) | **Not redistributed by us.** The installer downloads the upstream release asset on the VPS. | Pin and checksum only |
| sing-box (Tamara core, patched) | GPL-3.0-or-later plus an upstream naming restriction | Inside Tamara packages | Distributing it creates corresponding-source obligations; `tamara-next` records this as a release blocker pending legal review |
| WinDivert 2.2.2 | LGPL-3.0-only (the option upstream sing-box selects) | Tamara Windows package | Full notice must accompany the package |
| Tamara application | `NOASSERTION` | — | **BLOCKED: owner/legal licence selection** |
| Legacy `David610/tamara` | Hiddify-derived (inherits Hiddify licensing) | Not distributed by `tamara-next` | Must not be copied; only the migration code references its data layout |
| amneziawg-go (planned) | MIT (WireGuard LLC copyright retained) | Built on server hosts; a future Tamara engine | Permissive; keep notices |
| amneziawg-tools (planned) | GPL-2.0 | Built from source on server hosts | If prebuilt `awg` binaries are ever shipped in releases, GPL-2.0 source obligations apply |
| amneziawg-linux-kernel-module (optional) | GPL-2.0 | DKMS on host | Optional; userspace amneziawg-go is the default |

**Technical isolation already in place:** the server contract is client-agnostic. Fallback clients use `/sub`, and nothing on the server depends on Tamara code. Platform v2 keeps it that way: every route, credential and telemetry contract lives in this repository as versioned JSON schemas with fixtures, so any first-party client that passes the fixtures can replace Tamara's UI or engine.

## 6. Local environment used by this pass

- Windows 10 host, repository at `D:\SWTPP\singbox-vpn`, CRLF checkout. Shell tests do not run here.
- WSL2 Ubuntu 24.04 (kernel `6.18.33.2-microsoft-standard-WSL2`) with rustup 1.94.1, Go 1.22.2 plus an auto-fetched Go 1.25.12 toolchain, shellcheck and sing-box 1.13.19. Root is available through `wsl -u root` for network-namespace tests. There is no `make`, `iperf3` or `nft`, and the AmneziaWG kernel module is not loadable.
- Upstream AmneziaWG built locally from pinned sources:
  - `amneziawg-go` `v3.1.20260828` = `b5928efb6ca19f0153958460c3d141f04abc5c2e`: `go mod verify` succeeded in a fresh module cache, and the binary SHA-256 is `6afd9d43…5766`.
  - `amneziawg-tools` `v3.1.20260812` = `ee0f0a9aa34ff0a0da4b3433b9512781cfe02843`: built with gcc from `src/*.c`, and the `awg` SHA-256 is `b50f7415…c492`.
  - The first build attempt **failed closed** on a Go checksum mismatch. It came from a corrupt entry in a pre-existing local module cache, not from upstream: the repository `go.sum` matches `sum.golang.org`. This is the intended fail-closed behaviour; it was not bypassed.
- Real VPS access: RU `135.106.178.167` (Ubuntu 24.04), DE `91.244.71.165` (AlmaLinux 9.8), both on `0c9d37e`. **Not used in this pass**: every change here is local, and deploying to them is an operator-approved step.
