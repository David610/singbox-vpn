# ADR-0010: Fleet platform foundations

> Status: PROPOSED (Phase 0). Companion to
> `docs/FLEET_PLATFORM_PLAN.md`. Read ADR-0009 first — it explains why
> peer-endpoint credentials are per-user-per-endpoint rather than shared,
> and that decision is inherited unchanged here (§5 of the spec this ADR
> answers: "credentials remain individual").

## Context

Arcana is moving from a single-node VPN SaaS (one `nodes` row, one
`vpn_accounts` row per user, `node_id default 'node-1'` hardcoded almost
everywhere) to a multi-node fleet: stable locations, disposable nodes,
devices as first-class credential owners, connection profiles as routing
policy independent of billing, and a scheduler that assigns concrete
node(s) to a device's chosen profile.

Two repositories are involved. `vpn-web` (Cloudflare Functions +
Supabase/Postgres) already owns the customer/billing/provisioning-job/node
data model. `singbox-vpn` (Rust) owns the per-node agent that polls jobs,
renders sing-box config, and applies it with a fail-closed
render→validate→backup→apply→verify→rollback pipeline. Neither repo has a
fleet *registry* today — confirmed by direct code inspection, not assumed
(see plan doc §1).

Three decisions need to be made explicit before any schema or agent code
changes, because getting them wrong is expensive to unwind later.

## Decision 1: The fleet registry lives in vpn-web/Supabase, not a new service

**Decision**: `locations`, node lifecycle/desired-observed-revision
columns on `nodes`, `connection_profiles`, `allowed_paths`, and
`fleet_operations` all live in the existing Supabase Postgres database,
governed by the same RLS + `security definer`/`search_path=''` house style
already used throughout `vpn-web`'s migrations. No new datastore, no
Kubernetes, no service mesh, no message queue.

**Why**: `singbox-vpn` has zero fleet-registry code today — every type in
that repo (`DeploymentConfig`, `NodeRole`, `CompatEndpoint`) describes a
single deployment's own config, not a fleet-wide catalog. Building a
second registry there would duplicate `nodes`/`provisioning_jobs` and
create a synchronization problem between two sources of truth. vpn-web
already has the job queue (`claim_next_job`, `SECURITY DEFINER`, `FOR
UPDATE SKIP LOCKED`), the heartbeat/telemetry ingestion endpoints, and the
admin UI shell (`src/app/admin/nodes/page.tsx`). Extending that surface is
strictly less risky than introducing a second control plane, and matches
the spec's explicit "no Kubernetes/service mesh/event bus unless measured
need appears" constraint (§19, §58).

**Consequence**: `singbox-vpn`'s `apps/provisioning-agent` stays a polling
consumer of a vpn-web-owned queue. Phase 6's `APPLY_NODE_REVISION` job
type is an eighth entry in the existing job-type enum
(`apps/provisioning-agent/src/dispatch.rs`), not a new transport or
protocol.

## Decision 2: Desired/observed revision tracking extends the existing apply pipeline; it does not replace it

**Decision**: The fail-closed pipeline in
`apps/admin/src/main.rs::render_and_apply_singbox_config` (render →
fingerprint short-circuit → `sing-box check` → `.bak` → atomic rename →
`reload_and_verify()` → rollback from `.bak`) remains the mechanism by
which config actually gets applied on a node. Phase 6 adds a
revision-numbered "desired state" document fetched by
`APPLY_NODE_REVISION` and an `observed_revision` reported back on
heartbeat; it does not rewrite the render/validate/backup/apply/rollback
steps themselves.

**Why**: That pipeline is the single strongest reliability guarantee in
the codebase today and is exactly what §9 of the spec demands be
preserved ("never optimize this away"). The gap it doesn't cover is
*coalescing* — many rapid imperative job-queue changes (CREATE_USER,
SET_EXPIRY, ...) each currently trigger an independent apply/restart
cycle. Revision-based reconciliation addresses that gap by letting vpn-web
batch several user-facing changes into one desired-state document before
the agent ever calls the existing apply function once.

**Consequence**: Imperative job types (`CREATE_USER`, `DISABLE_USER`, ...)
are not deleted in Phase 6. They keep working for existing flows.
`APPLY_NODE_REVISION` is introduced alongside them and only selected
operations migrate to it after equivalent-behavior and rollback tests
exist (spec §9, §54 Phase 6).

## Decision 3: Double-hop reuses `NodeRole::Relay` + `access_paths` + per-endpoint credentials as-is; no new N-hop config renderer

**Decision**: Phase 7 (double-hop) is scoped to the existing two-role
model — `NodeRole::Relay` forwarding to a declared exit via
`relay_targets()`, with `access_paths`/`AccessPathKind::Relay` and
`peer_credentials` as the client-facing contract, plus a new
`allowed_paths` table in vpn-web naming which entry×exit location pairs
are permitted. No generic chain-of-N-relays renderer is built.

**Why**: `crates/compat-config/src/server.rs`'s relay config rendering is
already fail-closed (reject-all with no `direct` fallback for relay
routes) and already has test coverage
(`crates/compat-config/tests/relay_role_policy.rs`,
`tests/two_hop_system.rs`). `provisioning-contract`'s cross-validation
(rule E) already asserts a relayed endpoint's outbound carries a `detour`
to its first hop. This is a verified, working two-hop primitive; the gap
is orchestration (which entry/exit pairs are *allowed*, and which
concrete nodes get assigned), not the rendering/validation logic itself.
Building a new N-hop renderer would duplicate tested code for a
capability (§12 explicitly says "do not initially support every possible
entry × exit combination") the spec doesn't ask for yet.

**Consequence**: `allowed_paths` (vpn-web) constrains route *selection*;
`NodeRole`/`relay_targets()`/`access_paths` (singbox-vpn) stay the
mechanism that *enforces* a selected route is actually fail-closed at the
config level. If a future phase needs true N-hop chains, that is a new
ADR, not an extension of this one.

## Alternatives considered

- **A separate Rust "fleet-controller" service** owning node lifecycle,
  independent of vpn-web's Cloudflare Functions. Rejected for Phase 0–8:
  no measured need yet (§58), and it would split the node's source of
  truth across two repos/runtimes for no capability vpn-web's existing
  Postgres + RPC pattern can't already provide. Revisit only if
  reconciliation latency or vpn-web's Workers runtime becomes a proven
  bottleneck (§51 load-test data would be the trigger).
- **Replacing the imperative job queue outright with revision-only
  reconciliation in Phase 1.** Rejected: spec §9 and §54 both require
  proving equivalent behavior and rollback before migrating any existing
  operation, and the imperative queue has real production traffic
  (`provisioning_jobs`) today.

## Status

Proposed at Phase 0. To be revisited (new ADR, not an edit to this one) if
Phase 5 load-test evidence or Phase 7 real two-node testing contradicts
any of the three decisions above.
