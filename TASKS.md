# TASKS.md

Status legend: `[ ]` not started · `[~]` in progress · `[x]` completed · `[!]` blocked/deferred (with reason)

## Phase 0 — Repo audit & architecture
- [x] Inspect repository (empty repo, fresh start)
- [x] PLAN.md
- [x] docs/ARCHITECTURE.md
- [x] docs/THREAT_MODEL.md
- [x] docs/TRANSPORT_MODEL.md
- [x] docs/RENDEZVOUS_DESIGN.md
- [x] docs/SECURITY_MODEL.md
- [x] docs/PRIVACY_MODEL.md
- [x] docs/FAILURE_CLASSIFICATION.md
- [x] docs/DECISION_ENGINE.md
- [x] docs/DEPLOYMENT.md
- [x] docs/TEST_STRATEGY.md
- [x] docs/ADR/0001-language-choice.md
- [x] docs/ADR/0002-transport-portfolio.md
- [x] docs/ADR/0003-transport-runtime-deferred.md
- [x] docs/ADR/0004-rendezvous-design.md
- [x] docs/ADR/0005-telemetry-policy.md
- [x] docs/ADR/0006-relay-topology.md
- [x] docs/ADR/0007-persistence.md
- [x] docs/ADR/0008-signing-hierarchy.md

## Phase 1 — Core foundations
- [x] Workspace Cargo.toml
- [x] crates/common (ids, error types, time buckets)
- [x] crates/crypto (ed25519 signing/verify via `ed25519-dalek`; three-tier key hierarchy; `Secret<T>` no-Debug wrapper). Session-layer crypto (TLS/QUIC, using `rustls`'s `ring` backend) lives in `transport-native`, not here.
- [x] crates/config (signed bundle schema, validation, expiry, revocation) + tests
- [x] crates/transport-api (Transport trait, capability negotiation) + tests
- [x] crates/network-state (failure categories, observation types)
- [x] crates/failure-classifier (state machine) + unit + property tests
- [x] crates/policy (endpoint/transport confidence scoring, quarantine) + tests

## Phase 2 — Local vertical slice
- [x] crates/transport-native: `direct-tls` (rustls) stream transport
- [x] crates/transport-native: `noise-quic` (quinn) datagram-oriented transport
- [x] services/relay-agent: ingress + egress forwarding (TCP/QUIC -> upstream)
- [x] apps/client-daemon: local SOCKS5-ish loopback proxy entrypoint driving transport+engine
- [x] tests/: end-to-end local test service reachable client -> ingress -> egress -> test HTTP server

## Phase 3 — Adaptive connection engine
- [x] Endpoint + transport scoring wired into client-daemon
- [x] Non-deterministic fallback (weighted, jittered) instead of fixed order
- [x] Quarantine on repeated failure, decay over time
- [x] Failure classification wired to real connect() error paths
- [x] Simulated failure integration tests (blocked transport / blocked endpoint)

## Phase 4 — Signed rendezvous
- [x] services/rendezvous: issue signed, expiring, limited relay subset
- [x] rendezvous-client crate: fetch + verify + cache + emergency-bundle fallback
- [x] key rotation model documented + implemented (root -> release -> bundle signing key)

## Phase 5 — Sandboxed transport runtime
- [!] WASM/WASI transport runtime — deferred, documented in ADR-0003 and
      TRANSPORT_MODEL.md with the security boundary spec a follow-up must
      satisfy. Not stubbed as fake-working code.

## Phase 6 — Multiple real transport families
- [x] Two independent families implemented (direct-tls, noise-quic)
- [!] Third family (e.g. obfs4/Snowflake adapter) — not started, evaluated
      in ADR-0002, left for follow-up (external project integration needs
      separate legal/license review time this session doesn't have).

## Phase 7 — Measurement plane
- [x] telemetry crate: typed, minimal event schema (no destinations/payloads)
- [x] docs/TELEMETRY_DICTIONARY.md
- [!] Aggregation/collection service — not started (documented as future work)

## Phase 8 — Relay separation
- [x] relay-agent supports combined ingress+egress and split ingress->egress

## Phase 9 — Linux networking integration
- [!] TUN device + routing + kill-switch firewall integration — not
      implemented this session (needs root/CAP_NET_ADMIN + real interface
      manipulation, higher risk of destructive host changes than the sandbox
      permits). Interface (`KillSwitch` trait) and policy defined and unit
      tested against a mock backend; real nftables backend documented as
      follow-up in DEPLOYMENT.md.

## Phase 10 — CLI and diagnostics
- [x] apps/cli: `transports`, `config-verify`, `diagnostics` (no daemon control-plane IPC exists yet, so `connect`/`disconnect`/`status` against a *running* daemon are not implemented — see docs/DEPLOYMENT.md and the CLI's own module doc comment)

## Phase 11 — Security testing
- [x] fuzz targets: config bundle parser, rendezvous response parser
- [x] property tests: state machine invariants, scoring bounds, serialization round-trips
- [~] tc netem hostile-network tests: script + `#[ignore]`d test written (`tests/hostile_network/`) and reviewed; not executed in this session — the sandbox has neither `iproute2`/`tc` nor permission to mutate host network namespaces (verified: `id` shows uid 0 but `ip`/`tc` binaries are absent). Failure-independence *is* proven without netem via `tests/failure_independence.rs` (deterministic connection-refused simulation), which is a real but weaker substitute — see docs/TEST_STRATEGY.md.

## Phase 12 — Performance
- [x] criterion benchmarks: config bundle verify (`crates/config/benches/verify.rs`), scoreboard observe+select (`crates/policy/benches/scoring.rs`)

## Phase 13 — UX
- [!] Desktop GUI — explicitly deferred per spec ("only after core works";
      out of scope for this session's time budget)

## Final
- [x] cargo fmt / clippy / test clean
- [x] Final security self-review notes in docs/THREAT_MODEL.md §Review
- [x] Final engineering report delivered to user
