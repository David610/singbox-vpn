# Multi-endpoint Resilience Spine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the server describe independently-hosted endpoints with per-user credentials and failure-domain metadata, let Tamara ingest and cache them safely, and provide a pure selector that decides which endpoint to use.

**Architecture:** Additive `schema_version` 1 extension carrying optional endpoint metadata plus an embedded, opaque `singbox_config` rendered from the same endpoint model. Runtime declarative peer endpoints with per-user operator-supplied credentials. Tamara parses the envelope only (never transports), stores the catalog inside the existing profile-operation journal's transaction, and a pure state machine decides selection without touching Core.

**Tech Stack:** Rust (serde, axum, clap, toml), Dart/Flutter (freezed, drift, fpdart, riverpod).

**Spec:** [`docs/superpowers/specs/2026-09-09-multi-endpoint-resilience-spine-design.md`](../specs/2026-09-09-multi-endpoint-resilience-spine-design.md)

## Global Constraints

- `schema_version` stays **1**. Never call this "v1.1" in code, docs, or commit messages. The correct phrase is "additive `schema_version` 1 extension".
- `DEPLOYMENT_SCHEMA_VERSION` stays `1`. `USERS_SCHEMA_VERSION` stays `1`.
- With zero peers configured, `users.json` and `deployment.toml` load byte-identically and every client-facing output is equivalent to current behaviour.
- A peer endpoint the user has no valid credential for is **absent** from that user's document, never present-but-broken.
- Peer credentials are **never auto-generated**. The operator supplies them.
- The peer's REALITY **private** key is rejected by config validation.
- `singbox_config` is **never logged** at any level.
- Dart **never** parses VLESS/REALITY/Hysteria2 parameters, and the provisioning envelope is **never** handed to Core as sing-box config.
- The Dart catalog model has **no field capable of holding a credential**.
- Selector health state is **in memory only**. No telemetry database.
- No `CENSORSHIP_CONFIRMED` / `DPI_DETECTED` enum variants may exist.
- No production addresses hard-coded in tests. Fixtures and loopback only.
- No changes to `David610/singbox-client`.
- No rename of `fixtures/singbox-client-contract/`.

---

## Task 1: Contract metadata fields and structural config audit

**Files:**
- Modify: `crates/provisioning-contract/src/lib.rs`

**Interfaces:**
- Produces: `PathType` enum (`Direct`, `Other(String)`); `Endpoint.failure_domain/region/provider/asn/path: Option<...>`; `ProvisioningDocument.singbox_config: Option<serde_json::Value>`; `ProvisioningDocument::with_singbox_config(v)`; `ContractError::{ForbiddenContent, EmbeddedConfigInvalid, EndpointTagMissingOutbound, SelectorOptionsMismatch, RouteFinalMismatch}`.

- [ ] **Step 1:** Write failing tests: optional fields absent serialize byte-identically; `insecure: false` in embedded config is accepted; `insecure: true` rejected; a `dns` top-level key in embedded config rejected; `private_key` anywhere rejected.
- [ ] **Step 2:** Run `cargo test -p provisioning-contract` — expect failures.
- [ ] **Step 3:** Add the fields, `PathType`, and split `audit_serialized` into envelope substring audit (serialize with `singbox_config` removed) plus `audit_embedded_config` structural walk (top-level allowlist `outbounds`/`route`; `route` allows only `final`/`rules`; reject private-key-shaped keys; reject `insecure` unless exactly `false`; reject `-----BEGIN`/`.pem`/`/etc/`/`/var/`/`/opt/` in string values).
- [ ] **Step 4:** Run tests — expect pass.
- [ ] **Step 5:** Commit.

## Task 2: Catalog/config cross-validation

**Files:**
- Modify: `crates/provisioning-contract/src/lib.rs`

**Interfaces:**
- Produces: `ProvisioningDocument::validate` additionally enforcing invariants A–D from the spec (endpoint tag ↔ outbound tag; selector options == tags + `auto`; `route.final` == selector tag).

- [ ] **Step 1:** Write failing negative tests deliberately constructing: an endpoint with no matching outbound; a selector missing an endpoint tag; a selector with an extra unknown tag; `route.final` pointing somewhere other than the selector.
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Implement the cross-check inside `validate`, skipped entirely when `singbox_config` is `None`.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 3: Peer endpoint config surface

**Files:**
- Modify: `crates/compat-config/src/deployment.rs`
- Modify: `crates/compat-config/src/model.rs`

**Interfaces:**
- Produces: `PeerEndpointSection` (TOML `[[peer_endpoints]]`); `DeploymentConfig.peer_endpoints: Vec<PeerEndpointSection>` with `#[serde(default)]`; `EndpointOrigin::{Local, Peer}`; `CompatEndpoint.origin`, `.failure_domain`, `.region`, `.provider`, `.asn`, `.path`, all `#[serde(default)]`.

- [ ] **Step 1:** Write failing tests: a `deployment.toml` with no `[[peer_endpoints]]` loads unchanged; a peer id colliding with `reality-1` is rejected; a private-key-shaped key in the section is rejected; a malformed REALITY public key is rejected.
- [ ] **Step 2:** Run `cargo test -p compat-config` — expect failures.
- [ ] **Step 3:** Implement the section, its validation in `DeploymentConfig::validate`, and `CompatEndpoint` extension. Add `PeerEndpointSection::to_compat_endpoint()`.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 4: Per-user peer credentials in the user store

**Files:**
- Modify: `crates/compat-config/src/model.rs`
- Modify: `crates/compat-config/src/store.rs` (tests only)

**Interfaces:**
- Produces: `PeerCredential` enum (`VlessUuid(String)` / `Hysteria2Password(SecretString)`); `CompatUser.peer_credentials: BTreeMap<String, PeerCredential>` with `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`; `CompatUser::peer_credential(&self, endpoint_id) -> Option<&PeerCredential>`.

- [ ] **Step 1:** Write failing tests: an existing `users.json` with no peer credentials round-trips byte-identically; `USERS_SCHEMA_VERSION` unchanged; `Debug` of a `PeerCredential` never prints the secret.
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 5: Document assembly with peers and embedded config

**Files:**
- Modify: `crates/compat-config/src/contract.rs`
- Modify: `crates/compat-config/src/render.rs`

**Interfaces:**
- Produces: `contract_endpoint` sourcing credentials from `user.peer_credentials` when `origin == Peer`; `provisioning_document_with_mode` omitting peers lacking a credential and attaching `singbox_config`; `render::derive_failure_domain(host) -> String`.

- [ ] **Step 1:** Write failing tests: a peer with a credential appears; a peer without one is absent; two users get different peer credentials; a zero-peer document is unchanged from today; the embedded config's selector lists every endpoint tag.
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 6: `vpn-admin user peer` commands

**Files:**
- Modify: `apps/admin/src/main.rs`

**Interfaces:**
- Produces: `PeerCommands::{Set, Rotate, Remove, List}` under `UserCommands::Peer`.

- [ ] **Step 1:** Write failing tests for the credential-mutation helpers (transport/flag mismatch rejected; rotate on a missing credential fails; list returns ids only).
- [ ] **Step 2:** Run `cargo test -p vpn-admin` — expect failures.
- [ ] **Step 3:** Implement the subcommands and their handlers. `list` prints endpoint ids and presence only, never a value.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 7: Serve the extended document; fixtures and docs

**Files:**
- Modify: `services/subscription/src/lib.rs`
- Create: `fixtures/singbox-client-contract/10-peer-endpoint-with-embedded-config.json`
- Modify: `docs/PROVISIONING_CONTRACT.md`, `docs/ADR/0009-declarative-peer-endpoints.md`, `fixtures/singbox-client-contract/README.md`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces: `AppState.peer_endpoints`; a served document carrying `singbox_config`.

- [ ] **Step 1:** Write failing tests: `/v1/provision/{token}` response contains `singbox_config`; a user without peer credentials sees no peer endpoint.
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Wire peers into `AppState`, add the fixture, retarget docs to Tamara while retaining the `singbox-client` history as a dated note, and extend the versioning rule to optional top-level fields.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 8: Tamara catalog model and parser

**Files:**
- Create: `lib/features/profile/model/endpoint_catalog.dart`
- Create: `lib/features/profile/data/provisioning_document_parser.dart`
- Create: `test/features/profile/data/provisioning_document_parser_test.dart`
- Create: `test/features/profile/model/endpoint_catalog_credential_safety_test.dart`

**Interfaces:**
- Produces: `CatalogEndpoint`, `EndpointCatalog`, `EndpointPath`; `ProvisioningDocumentParser.tryParse(String) -> ProvisioningParseResult`; `ProvisioningParseFailure.{malformed, unsupportedSchemaVersion, missingConfig}`.

- [ ] **Step 1:** Write failing tests: a valid document parses; a Hiddify subscription is *not* recognised; unknown transport round-trips opaque; absent `failure_domain` derives from host; `toString`/`jsonEncode` of a catalog never contains a fixture credential.
- [ ] **Step 2:** Run `flutter test` on those files — expect failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 9: Catalog persistence inside the existing journal transaction

**Files:**
- Modify: `lib/features/profile/model/profile_entity.dart`
- Modify: `lib/core/db/db.dart` (new column + migration)
- Modify: `lib/features/profile/data/profile_data_mapper.dart`, `profile_data_source.dart`
- Create: `test/features/profile/data/catalog_crash_consistency_test.dart`

**Interfaces:**
- Produces: `RemoteProfileEntity.endpointCatalog: String?` carried through `commitDb(operationId)` so the existing `_commitCandidate` journal covers it.

- [ ] **Step 1:** Write failing tests: recovered state is never NEW CONFIG + OLD CATALOG or the reverse; a decrypt failure raises a typed error rather than returning ciphertext.
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Implement the column, migration, mapper encryption, and entity field.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 10: Wire the provisioning path into the real import flow

**Files:**
- Modify: `lib/features/profile/data/profile_parser.dart`
- Modify: `lib/features/profile/data/profile_repository.dart`
- Create: `test/features/profile/data/provisioning_import_path_test.dart`

**Interfaces:**
- Consumes: Tasks 8–9.
- Produces: remote fetch recognising a provisioning envelope, writing only the extracted `singbox_config` to the config file and the catalog to the row.

- [ ] **Step 1:** Write failing tests: the envelope itself is never written to the config file; ordinary subscriptions take the unchanged path; a malformed document preserves the cached catalog; an unsupported schema preserves it.
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 11: Pure selector state machine

**Files:**
- Create: `lib/features/connection/selector/endpoint_health.dart`
- Create: `lib/features/connection/selector/endpoint_selector.dart`
- Create: `test/features/connection/selector/endpoint_selector_test.dart`

**Interfaces:**
- Produces: `EndpointHealthState` enum; `EndpointFailureCategory` enum; `EndpointHealth` (bounded scalars); `EndpointSelector.observe(tag, outcome, generation, now)`, `.select(now) -> SelectionDecision?`; `SelectionDecision(outboundTag, reason)`.

- [ ] **Step 1:** Write failing tests for spec scenarios 1–11, 15, 16, 18 (single endpoint; healthy selection; within-domain switch; cross-domain switch; no flap-back; no latency displacement; bounded retries; cancellation; catalog change; rotation; domain sharing; transport failure not poisoning a domain; stale generation rejection).
- [ ] **Step 2:** Run — expect failures.
- [ ] **Step 3:** Implement with derived domain health, asymmetric hysteresis, capped counters, capped backoff, generation rejection.
- [ ] **Step 4:** Run — expect pass.
- [ ] **Step 5:** Commit.

## Task 12: Client documentation and evidence boundary

**Files:**
- Create: `docs/CENSORSHIP_RESILIENCE_ARCHITECTURE.md` (Tamara repo)
- Create: `docs/ENDPOINT_CATALOG.md` (Tamara repo)

- [ ] **Step 1:** Write both documents with an explicit VERIFIED-LOCALLY vs UNVERIFIED split, stating IMPLEMENTED ≠ REAL-VPS-VERIFIED and LOCAL FAILOVER LOGIC VERIFIED ≠ CENSORSHIP RESISTANCE VERIFIED.
- [ ] **Step 2:** Commit.
