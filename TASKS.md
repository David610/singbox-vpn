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

## Phase 4b — Production key management (ADR-0008 tooling)
- [x] `apps/keytool` offline signing-ceremony CLI: `root-init`,
      `release-issue`, `bundle-issue`, `revoke-issue`, `verify-chain`.
      Never opens a socket; only reads/writes local files.
- [x] `crypto::KeyPair::save_to_file` / `load_from_file`: hex-encoded key
      files written mode 0600, load refuses any file with group/other
      permission bits set (`CryptoError::InsecureKeyPermissions`).
- [x] `config::revocation::SignedRevocationList`: revocation lists signed
      by the release key (chains to root), independently verifiable by
      any holder regardless of delivery channel.
- [x] `services/rendezvous --key-dir/--release-cert-file`: loads a
      persisted bundle key + cert chain instead of generating an
      ephemeral hierarchy every boot (ephemeral path kept for local dev,
      now logs `warn` not `info`). `--revocation-list-file` serves the
      signed list verbatim at `GET /v1/revocation-list`.
- [x] `services/relay-agent --identity-dir`: persists the relay's
      TLS/QUIC identity (cert mode 0644, key mode 0600) across restarts
      so previously-issued bundles' `cert_sha256_hex` pins don't go stale.
- [x] Full chain tested for real against the actual `vpn-keytool` binary
      and real files on disk (`apps/keytool/tests/ceremony.rs`): sign
      with a persisted hierarchy, verify via `config::verify_bundle` on a
      fresh check, rotate the bundle key, issue a signed revocation
      naming the old key, confirm the old key's signature is now
      rejected (`ConfigError::RevokedKey`) while the new key still
      verifies. Also covers `verify-chain` and refusal to overwrite an
      existing root key.
- [~] Revocation-list *serving* is wired (rendezvous endpoint), but
      `rendezvous-client`/`apps/cli` do not yet fetch it automatically —
      today a caller must pass a `RevocationList` in explicitly (see
      `RendezvousClient::get_bundle`). Wiring an automatic
      fetch-and-verify-then-use-for-every-bundle-check path is the
      natural next step; not done this session for scope reasons, not
      because of a technical blocker.

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
- [!] TUN device + routing + kill-switch firewall integration — still not
      implemented (follow-up session priority, not attempted this session:
      time was spent on Priority 1 key-management tooling, which this
      session's branch/task explicitly led with). Interface
      (`KillSwitch` trait) and policy remain defined and unit tested
      against a mock backend only; real nftables backend documented as
      follow-up in DEPLOYMENT.md. Re-checked this session:
      `ip`/`tc`/network-namespace tooling is still absent from the
      sandbox (see Phase 11 below), which would also have blocked testing
      a real backend even if implemented.

## Phase 10 — CLI and diagnostics
- [x] apps/cli: `transports`, `config-verify`, `diagnostics` (no daemon control-plane IPC exists yet, so `connect`/`disconnect`/`status` against a *running* daemon are not implemented — see docs/DEPLOYMENT.md and the CLI's own module doc comment)

## Phase 11 — Security testing
- [x] fuzz targets: config bundle parser, rendezvous response parser
- [x] property tests: state machine invariants, scoring bounds, serialization round-trips
- [~] tc netem hostile-network tests: script + `#[ignore]`d test written (`tests/hostile_network/`) and reviewed; still not executed — re-checked this session (`which ip tc` both fail, uid 0 but `iproute2` is simply not installed in this sandbox image), so the situation is unchanged from the prior session, not newly re-verified as fixable. Failure-independence *is* proven without netem via `tests/failure_independence.rs` (deterministic connection-refused simulation), which is a real but weaker substitute — see docs/TEST_STRATEGY.md.
- [~] cargo-fuzz targets (`fuzz/`) still not executed — re-checked this
      session: only the `stable` rustup toolchain is installed, no
      nightly, and `cargo-fuzz` was not present. Proptest-based substitute
      tests (`crates/config` `signed_bundle_json_parsing_never_panics_on_arbitrary_bytes`,
      `expiry_check_never_panics`) remain the documented fallback and run
      on every `cargo test`.

## Phase 12 — Performance
- [x] criterion benchmarks: config bundle verify (`crates/config/benches/verify.rs`), scoreboard observe+select (`crates/policy/benches/scoring.rs`)

## Phase 13 — UX
- [!] Desktop GUI — explicitly deferred per spec ("only after core works";
      out of scope for this session's time budget)

## Final
- [x] cargo fmt / clippy / test clean
- [x] Final security self-review notes in docs/THREAT_MODEL.md §Review
- [x] Final engineering report delivered to user
