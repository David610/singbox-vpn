# Fleet Platform Plan

Status: DRAFT — Phase 0 baseline. Read alongside `docs/ADR/0010-fleet-platform-foundations.md`.

This plan spans two repositories:

- `David610/vpn-web` — Next.js/Cloudflare Functions control plane, Supabase/Postgres.
- `David610/singbox-vpn` — Rust workspace: `apps/admin` (node-local config
  render/apply/rollback), `apps/provisioning-agent` (job poller + heartbeat),
  `crates/compat-config` (deployment/config model), `crates/provisioning-contract`
  (client-facing document shipped to Tamara/subscription clients),
  `services/subscription` (renders `ProvisioningDocument`s).

It supersedes nothing in `docs/SUPPORTED_PRODUCT.md` or ADR-0009 yet — both
still describe the current single-node-per-deployment reality accurately.
This plan is the path from that reality to a multi-node fleet, executed in
phases, each shipped as reviewed PRs with tests and evidence before the next
begins.

## 1. Current state (verified 2026-09-24)

### vpn-web / Supabase

- **Billing**: one Stripe subscription per `customer_accounts` row already
  (not per-user). `INCLUDED_SEATS = 3` + `extra_seats` (Stripe line-item
  quantity mirrored into `subscriptions`). Seat math in
  `functions/lib/accounts.js`, purchase flow in `functions/api/account/seats.js`.
  This is already close to the target seat-pack model (§3 below) — no
  per-member-group subscriptions to unwind.
- **Members**: `account_members` (user↔account, one account per user today),
  `member_invites` (hashed tokens, now identity-bound to confirmed email as of
  PR #19).
- **VPN identity**: `vpn_accounts` (`user_id → vpn_user_id, node_id`, one row
  per user, `node_id default 'node-1'`) + `vpn_secrets` (encrypted). One
  credential per user, not per device — devices don't exist yet.
- **Provisioning**: `provisioning_jobs` is a Postgres-table job queue.
  Job types today: `CREATE_USER, SET_EXPIRY, CLEAR_EXPIRY, ENABLE_USER,
  DISABLE_USER, ROTATE_SUBSCRIPTION_TOKEN, ROTATE_CREDENTIALS`. Agent claims
  via `claim_next_job()` (SECURITY DEFINER, `FOR UPDATE SKIP LOCKED`).
  `functions/lib/provision-entitlement.js` diffs `account_members` against
  `vpn_accounts` and inserts jobs.
- **Nodes**: `nodes` table is flat — `node_id, api_key_hash, last_seen_at,
  agent_version, vpn_version, singbox_version`, resource/telemetry columns.
  No region, provider, ASN, capacity, or lifecycle state. Node selection is
  `resolveNodeForUser()` in `functions/lib/resolve-node.js`, hardcoded to
  return `"node-1"` — already isolated as the single choke point to replace.
- **RLS/security house style**: every table RLS-enabled; internal tables
  `revoke all ... from anon, authenticated` with zero policies; every
  privileged function is `security definer` + `set search_path = ''` +
  explicit `revoke execute ... from public, anon, authenticated`. New fleet
  tables must follow this pattern exactly.
- **API layout**: `functions/api/{account,admin,agent,billing,vpn}/` with a
  real shared `functions/lib/` business-logic layer already — route handlers
  are thin. New domain modules (`fleet`, `profiles`, `devices`) slot into
  this existing shape; no need to invent a new layering convention.

### singbox-vpn

- **No fleet/node-registry concept exists in this repo at all.** Every
  primitive here is single-deployment: `DeploymentConfig` + `NodeRole`
  (`Exit`/`Relay`) describe *this* node's own role, not a fleet-wide catalog.
  The node registry, heartbeat ingestion, and job queue all live in vpn-web.
- **`peer_endpoints`** (compat-config, `CompatEndpoint{origin: Peer}`) and
  **`access_paths`** (provisioning-contract, `AccessPath{kind, via_endpoint_id,
  failure_domain, region, provider, capabilities}`) already carry the
  location/failure-domain/provider metadata this plan needs — they're
  currently populated by hand in `deployment.toml`, not by a scheduler.
- **Relay/double-hop**: `NodeRole::Relay` config rendering is fail-closed
  (route rules restrict to `relay_targets()`, implicit reject-all, no
  `direct` fallback) — genuinely a single relay→exit hop, not an N-hop
  chain renderer. `provisioning-contract` cross-validates that a relayed
  endpoint's outbound carries a `detour` to its first hop
  (`cross_validate_embedded_config`, rule E). This is the strongest existing
  foundation for §12 (double-hop) — reuse it, don't replace it.
- **Credentials**: per-user, per-peer-endpoint (`CompatUser.peer_credentials:
  BTreeMap<endpoint_id, PeerCredential>`, ADR-0009 Option A). Local
  (non-peer) credentials are flat `vless_uuid`/`hysteria2_password` fields.
  No device concept; per-user is the finest grain today.
- **Apply pipeline already implements the fail-closed invariant this plan
  must never weaken**: render → fingerprint short-circuit → validate
  (`sing-box check`) → backup (`.bak`) → atomic rename → reload/restart →
  verify (`systemctl is-active` polling) → rollback from `.bak` on failure
  (`apps/admin/src/main.rs::render_and_apply_singbox_config`). §9's
  declarative-revision work extends this pipeline; it does not replace it.
- **Provisioning-agent** is a polling consumer only (`POST
  /api/agent/claim`), not a push target — the job-creation/queue side is in
  vpn-web. Heartbeat and traffic-reporting endpoints
  (`/api/agent/heartbeat`, `/api/agent/traffic`) are likewise implemented in
  vpn-web, not this repo.

### Baseline commits

- `vpn-web@main`: PR #19 (invite identity binding + recent-auth hardening)
  merged 2026-09-24, CI green (unit tests + Postgres migration smoke).
- `singbox-vpn@main`: `2e16504` (sing-box 1.14.1, v1.1.0-rc.1).

## 2. Target state

See the full spec (62 sections, supplied by the user 2026-09-24) for the
complete target architecture. Summary of the load-bearing invariants this
plan protects at every phase:

```
billing != routing
user != device
location != node
profile != credential
desired != observed
logical path != physical VPS
```

Concretely: one Stripe subscription per account with a seat-pack quantity;
devices as first-class, individually-revocable, owning their own VPN
identities; stable `locations` that survive node replacement; a `nodes`
fleet registry with explicit lifecycle states and desired/observed
revision tracking; `connection_profiles` as pure routing policy, assigned
to devices, never touching Stripe; a deterministic (non-ML) scheduler with
sticky assignments and hysteresis; double-hop built on the existing
Relay/Exit primitives with an approved-route allowlist.

## 3. Migration sequence and cross-repo contracts

Each phase below is a `vpn-web` migration set + `vpn-web` API/UI change,
occasionally paired with a `singbox-vpn` agent/contract change. The
contract boundary between the two repos stays exactly where it is today:
**vpn-web owns the queue and the node registry; singbox-vpn's
provisioning-agent polls and reports.** Phase 6 (declarative reconciliation)
is the first phase that changes that contract — it adds a
`APPLY_NODE_REVISION` job type alongside the existing seven, fetched the
same way, so the agent's poll loop and worker_client don't need a new
transport.

| Phase | Repo(s) | Deliverable | Depends on |
|---|---|---|---|
| 0 | vpn-web | PR #19 merged, CI green, this plan + ADR | — |
| 1 | vpn-web | `devices`, `locations`, node lifecycle columns, `connection_profiles`, `device_profile_assignments`, `allowed_paths`, `fleet_operations`/`operation_steps`, desired/observed revision columns on `nodes`. Backfill legacy device per existing `vpn_accounts` row. No credential rotation. | 0 |
| 2 | vpn-web | Fleet admin UI: lifecycle transitions, drain/maintenance/quarantine, audit log entries, replace `resolveNodeForUser()`'s hardcoded return with a real (still single-node-equivalent) lookup against `locations`/`nodes`. | 1 |
| 3 | vpn-web + singbox-vpn | One-time enrollment token flow; move a manually-provisioned node through PROVISIONING→WARMING_UP→READY without hand-inserted rows. | 2 |
| 4 | vpn-web | Provider adapter interface + one real provider implementation + mock adapter for tests. | 2 |
| 5 | vpn-web | Scheduler (pure/testable), sticky assignment, multi-node direct routing behind a flag; old single-node path stays the fallback. | 1, 4 |
| 6 | vpn-web + singbox-vpn | `APPLY_NODE_REVISION` job type; agent-side revision fetch/apply reusing `render_and_apply_singbox_config`'s existing pipeline unchanged; coalescing of rapid changes; rollback/recovery tests. | 5 |
| 7 | singbox-vpn (mostly) + vpn-web | Double-hop: approved route pairs table, real two-node relay→exit test, reuse existing `NodeRole::Relay`/`access_paths`/`peer_credentials`. No new N-hop renderer. | 5 |
| 8 | vpn-web + singbox-vpn | Synthetic health probes, hysteresis/cooldown, draining, exit failover; client entry-fallback investigated against real sing-box/Hiddify configs before any claim is made. | 6, 7 |
| 9 | vpn-web | Customer-facing Connections/Devices UI; free device↔profile reassignment; device revoke. | 1, 5 |
| 10 | vpn-web | Seat-pack entitlement (`seat_capacity = 3 × pack_quantity`), safe upgrade/downgrade (no silent eviction), Stripe webhook reconciliation. | 1 |
| 11 | singbox-vpn + vpn-web | V2Ray stats API feasibility spike against sing-box 1.14.1; per-device counters only if trustworthy, else capability-gate UI. | 1, 9 |
| 12 | vpn-web | Replace-node workflow, canary rollout states, capacity-aware scheduling refinement. | 3, 4, 6, 8 |
| 13 | vpn-web | Telegram identity linking + Mini App on the same backend APIs. | 9, 10 |
| 14 | both | Load tests, chaos/failure injection, backup/restore drill, security review, docs. | all |

Phases 0–2 are the near-term commitment of this plan. Phases 3–14 are
sequenced but each starts with its own brainstorming/planning pass and its
own PR(s) — this doc is not a blanket authorization to implement all 14
phases unreviewed.

## 4. Risks

- **`node_id default 'node-1'`** is depended on by existing rows; Phase 1's
  `locations`/lifecycle columns must be additive and nullable, with
  `node-1` backfilled into the new model rather than replaced.
- **`resolveNodeForUser()`'s hardcoded return** is a good choke point but
  today has zero test coverage forcing it to stay in sync with `nodes` —
  add tests before changing its behavior in Phase 2.
- **CI runner Postgres is not ephemeral per job** (confirmed while fixing
  PR #19 — `create role anon` collided across two migration-smoke scripts
  in the same job). Any new migration-smoke script added in Phase 1+ must
  use the same idempotent-role-creation pattern now in
  `scripts/test-admin-ops-migration.sh` and
  `scripts/test-invite-identity-guard.sh`.
- **`services/subscription` and `provisioning-contract`'s cross-validation**
  (rules A–E) must keep passing as new endpoint/access-path shapes are
  introduced for multi-node routing — these are the existing guardrail
  against client/server config drift and should gate Phase 5+ changes.
- **No fleet registry exists in singbox-vpn** — confirmed by direct
  inspection, not assumed. All registry/lifecycle/scheduling state lives in
  vpn-web/Supabase per this plan; singbox-vpn stays a per-node agent that
  polls, applies, and reports. This keeps §19/§20's "no Kubernetes, no new
  service mesh" constraint trivially satisfied — there is no second control
  plane to build.

## 5. Phase 0 checklist

- [x] `git fetch --all --prune`, `gh pr list` on both repos
- [x] Inspect `vpn-web` main and PR #19
- [x] Inspect `singbox-vpn` main
- [x] Fix CI: idempotent role creation in both Postgres migration-smoke
      scripts (`scripts/test-admin-ops-migration.sh`,
      `scripts/test-invite-identity-guard.sh`)
- [x] PR #19 CI green (unit tests + migration smoke)
- [x] Independent code review of PR #19: zero findings
- [x] PR #19 merged to `vpn-web@main`
- [x] This plan (`docs/FLEET_PLATFORM_PLAN.md`)
- [x] ADR-0010 (fleet platform foundations)

## 6. Phase 1 checklist (domain/schema foundation)

- [x] `locations`, `devices`, `connection_profiles`,
      `device_profile_assignments`, `allowed_paths`,
      `fleet_operations`/`operation_steps` added
      (`vpn-web@20260924000000_fleet_foundations.sql`)
- [x] `nodes` extended with lifecycle/role/provider/failure-domain and
      desired/observed revision columns
- [x] `vpn_accounts.device_id` added (additive, nullable) and every
      existing row backfilled to a "Legacy device"; no credential rotated
- [x] `scripts/test-fleet-foundations-migration.sh` added and wired into
      CI unconditionally
- [x] Four review rounds found and fixed real bugs before merge: a
      device-backfill mis-pairing bug in the original set-based backfill,
      a `lifecycle_state` column default that would have silently
      defaulted every future node insert to READY instead of
      PROVISIONING, missing DB-level constraints on `connection_profiles`
      (exit-location-required, distinct-hops), an `ON DELETE` gap that
      would have broken the live `accept_member_invite` RPC path, a
      `register-node.mjs` breakage plus a TOCTOU race introduced while
      fixing it, and a missing same-account invariant on
      `device_profile_assignments` (closed with a trigger mirroring
      `enforce_member_invite_identity`)
- [x] CI green, merged to `vpn-web@main` (PR #20)
- [ ] Phase 2 (fleet registry/admin) — not started
