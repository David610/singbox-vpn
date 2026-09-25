# Fleet Phase 8: Synthetic Health Probes, Hysteresis, and Passive Failover

Status: approved design, pending implementation plan
Repos affected: `singbox-vpn` (provisioning agent), `vpn-web` (control plane)
Corresponds to: `docs/FLEET_PLATFORM_PLAN.md` Phase 8 ("Synthetic health probes,
hysteresis/cooldown, draining, exit failover; client entry-fallback investigated
against real sing-box/Hiddify configs before any claim is made")

## 1. Problem

Today, node health is inferred only from whether the provisioning agent is
heartbeating and from coarse resource telemetry (CPU/memory/disk). Nothing
verifies that the actual VPN data plane (sing-box, VLESS/Hysteria2) is
functioning. `nodes.lifecycle_state` (`functions/lib/node-lifecycle.js`)
already models `READY`/`DEGRADED`/`FAILED` and more, and the scheduler
(`functions/lib/scheduler.js`) already excludes non-`READY` nodes from new
placements — but nothing ever moves a node into `DEGRADED` or `FAILED`
automatically. That transition logic is this phase's scope, and it was
explicitly deferred by `node-lifecycle.js`'s own header comment when it was
written.

## 2. Goals

- Detect real data-plane failures (not just "agent process alive").
- Automatically transition nodes `READY` → `DEGRADED` → `FAILED` and back,
  using existing lifecycle machinery, with hysteresis so single bad probes
  don't cause flapping.
- Ensure the existing scheduler naturally stops sending new devices to
  degraded/failed nodes (already true — verify with a regression test).
- Give admins visibility via existing alerting, not new UI.

## 3. Non-goals (deferred)

- External/multi-region probing — agent self-probe only for this increment.
- Active eviction of already-connected devices — passive drain only; a
  device already assigned to a degraded node keeps its connection until it
  naturally reconnects, at which point normal scheduling avoids the bad node.
- Replace-node/canary rollout workflows (Phase 12).
- Client entry-fallback UX for exit failures — needs verification against
  real sing-box/Hiddify configs on real devices before any design claim;
  left for a follow-up increment.

## 4. Design

### 4.1 Data-plane probe (agent side — `singbox-vpn`)

**Amended 2026-09-25**: rather than embedding a VLESS/Hysteria2 client
library in the agent (a large new dependency for this increment), the
probe uses sing-box's Clash API, which the agent already optionally
connects to (`AgentConfig::clash_api_url`, `worker_client.rs` stats
reporting). Sing-box's Clash API exposes `GET /proxies/{name}/delay?url=
<test-url>&timeout=<ms>`, which actively drives the configured outbound
to fetch a real URL and reports success/latency or failure — a genuine
data-plane egress test, not a mock.

`apps/provisioning-agent/src/telemetry.rs`'s `collect()` gains a probe
step that runs before building the heartbeat payload: if `clash_api_url`
is configured, call the delay-test endpoint against the node's outbound
proxy group and a fixed low-cost test URL; record success/failure and
latency. If `clash_api_url` is not configured (older/unconfigured nodes),
omit the probe fields entirely — `null`, never coerced to `false` —
matching the existing `active_users_recent` null-vs-false convention, so
an unconfigured node cannot look identical to a genuinely failing one.
Result is added to the existing heartbeat payload as:

```rust
probe_ok: Option<bool>,
probe_latency_ms: Option<u64>,
```

No new endpoint, no new dependency, no DNS/HTTP/multi-region matrix in
this increment.

### 4.2 Schema (`vpn-web`)

New additive migration:

```sql
alter table nodes
  add column consecutive_probe_failures int not null default 0,
  add column consecutive_probe_successes int not null default 0,
  add column last_probe_at timestamptz,
  add column last_probe_ok boolean;
```

Matches the additive-migration precedent already established (ADR-0001).
No backfill required; existing nodes populate on their next heartbeat.

### 4.3 Transition logic (`vpn-web`, inline in `functions/api/agent/heartbeat.js`)

**Amended 2026-09-25**: `node-lifecycle.js`'s current `ALLOWED_TRANSITIONS`
does not yet include `DEGRADED → FAILED` or `FAILED → READY` — today
`FAILED` is reachable only from `PROVISIONING`/`WARMING_UP` and only
returns to `PROVISIONING`/`QUARANTINED`/`RETIRED`. The file's own header
comment already anticipates this ("automated health-based transitions...
are Phase 8 and are not modeled here yet"), so Task 1 of the
implementation plan extends `ALLOWED_TRANSITIONS` to add these two edges,
with a comment marking them as the Phase 8 health-automation additions.
This is a prerequisite for the rest of this section to be implementable.

**Amended 2026-09-25 (post-implementation, final review)**: the original
text below assumed `canTransitionLifecycle()` (the admin-authorized
transition table) was itself a sufficient guard for automated transitions.
It is not: `ALLOWED_TRANSITIONS` legitimately permits an admin to move
MAINTENANCE/DRAINING/WARMING_UP → READY and PROVISIONING/WARMING_UP →
FAILED, and reusing that same table as automation's filter let a passing
probe streak silently undo an admin's MAINTENANCE/DRAINING, and let a
failing probe streak push a PROVISIONING/WARMING_UP node to FAILED and
break node creation. The shipped implementation (`node-health-transition.js`)
instead hardcodes the automation-eligible source states explicitly,
narrower than what the table alone would allow:

- `probe_ok = true` while `lifecycle_state = DEGRADED`: increment
  `consecutive_probe_successes`, reset failures to 0; at successes ≥ 5,
  transition to `READY`. Automation never recovers FROM MAINTENANCE,
  DRAINING, WARMING_UP, or PROVISIONING, regardless of streak — only a
  DEGRADED node recovers this way.
- `probe_ok = false` while `lifecycle_state = READY`: increment
  `consecutive_probe_failures`, reset successes to 0; at failures ≥ 3,
  transition to `DEGRADED`. Automation never pushes any other state
  (DEGRADED, WARMING_UP, PROVISIONING, etc.) to `FAILED` via the probe
  path — only `READY → DEGRADED` exists on the failure side.
- `probe_ok = null` (no `clash_api_url` configured): streak counters are
  left untouched for a `READY`/`DEGRADED` node, as originally specified —
  **except** when `lifecycle_state = FAILED`: a single authenticated
  heartbeat, regardless of probe result (there is none), recovers
  FAILED → READY. A node with no probe capability has no other evidence
  of health to offer, so "it is heartbeating again" is treated as
  sufficient — see the FAILED→READY note below, which applies uniformly
  regardless of probe configuration.
- `FAILED → READY` recovery: a single successful probe (`probe_ok = true`
  while `lifecycle_state = FAILED`) is sufficient, as a distinct case from
  the DEGRADED-recovery threshold above — it does not require 5 successes.

**Known limitation, not fixed by this phase**: `FAILED` is not silence-only
in this codebase as originally assumed — `fleet-operations.js`'s
`failNodeIfBooting` also sets `WARMING_UP`/`PROVISIONING → FAILED` when
node creation's readiness check fails or times out, and an admin can set
`FAILED` by hand. The FAILED→READY recovery rules above apply uniformly to
any FAILED node, meaning a node that failed its readiness check (or was
manually failed) can be auto-promoted back to READY on its next successful
heartbeat, skipping the check it failed or overriding the admin. This is a
pre-existing risk sharpened by this phase (previously bounded by the
5-success DEGRADED threshold for probe-capable nodes; now immediate for
both probe-capable and null-probe nodes). **This is a hard precondition,
not yet satisfied, for enabling `FEATURE_AUTO_NODE_HEALTH` in any real
environment**: distinguish silence-caused FAILED from other-caused FAILED
(e.g. a `failed_reason` column, set only by the silence-detection path) and
gate the FAILED→READY rules on it. Requires a migration; out of scope for
this implementation pass.

Automation's transitions still call `canTransitionLifecycle()` as a second
guard (defense in depth), but the primary safety mechanism is the explicit
automation-eligible-state list above, not the shared admin table.
Manual admin-triggered transitions continue to work unchanged via the
unmodified `ALLOWED_TRANSITIONS` table and act as an override at any time.

### 4.4 Silence detection (no heartbeat received)

Silence is worse than a failed probe — the node might be fully offline —
so it's checked lazily rather than via a new scheduler, since neither repo
has a cron/job system today and this phase should not introduce one just
for this:

**Amended 2026-09-25**: silence-eligible states are narrower than
originally specified. Any incoming heartbeat request from another node, or
the admin dashboard's node-list read, evaluates `now() - last_seen_at`
only for nodes currently `READY` or `DEGRADED` — not "all nodes except
FAILED/RETIRED/QUARANTINED" as originally written, which would have let a
stale `PROVISIONING`/`WARMING_UP` node (e.g. one mid-re-enrollment, whose
`last_seen_at` predates a fresh enrollment token) get silence-transitioned
to `FAILED`, breaking re-enrollment. Both call sites share one source of
truth (`isNodeSilent` in `node-health-transition.js`, via a shared
`functions/lib/node-silence-failover.js` helper) rather than each
filtering their own candidate query independently — a prior version of
this implementation had them diverge, which was itself a bug. Past a
threshold (3× the expected heartbeat interval), an eligible node is
transitioned directly to `FAILED` regardless of prior streak state —
silence is treated as immediate failure exhaustion, not counted against
the 3-failure threshold. Worst-case detection latency is bounded by
whichever comes first: the next heartbeat from any other node, or the
next admin dashboard load.

### 4.5 Failover (passive drain)

No new mechanism. `scheduler.js` already filters `lifecycle_state = READY`
for all placement decisions (direct, double-hop, AUTO). The instant a node
transitions to `DEGRADED`/`FAILED`, it stops receiving new devices. Devices
already assigned to it keep their existing connection (no forced
disconnect, no client signaling) until they naturally reconnect, at which
point the normal scheduling pass avoids the bad node. This section is
primarily a regression test confirming existing behavior, not new code.

### 4.6 Observability

Reuse the existing `operational_alerts` dedup-key pattern already used for
disk/memory alerts (`functions/api/agent/heartbeat.js` lines ~87-125): raise
an alert on automated transition into `DEGRADED`/`FAILED`, auto-resolve on
recovery to `READY`. No new admin UI required.

### 4.7 Rollout

Automated-transition logic is gated behind `FEATURE_AUTO_NODE_HEALTH`,
defaulting off in production, following the same pattern as
`FEATURE_MULTI_NODE_SCHEDULING`. Enabled first in staging against a real
node before production rollout, per the fleet plan's standing requirement
that data-plane claims be verified against real infrastructure rather than
assumed from code review.

## 5. Testing

- Unit tests for the streak/threshold/transition logic in `heartbeat.js`
  (pure function, isolable from the HTTP handler).
- Regression test confirming `scheduler.js` excludes `DEGRADED`/`FAILED`
  nodes from placement (may already be covered — verify, extend if not).
- Scripted staging integration check against one real node: kill the
  sing-box process → expect `DEGRADED` within 3 heartbeats → restart →
  expect `READY` within 5 heartbeats.

## 6. Open follow-ups (not blocking this increment)

- External/multi-region probing, if agent self-probe proves insufficient
  (e.g. doesn't catch ISP-level blocking between real clients and a node).
- Active eviction + client reconnect signaling, if passive drain proves too
  slow in practice.
- Exit-node client-fallback UX, pending real-device verification.
