# Paired control-plane integration evidence — vpn-web #84 × singbox-vpn #172 (2026-10-23)

Everything here ran in a **disposable local stack with test credentials**: PostgreSQL 16.15, PostgREST 16.5, vpn-web's real Node API
(`server/index.mjs`), the real `vpn-provisioning-agent` binary with a recording `vpn-admin` stand-in. No production system, Cloudflare
account, Supabase project or real agent was touched; no agent was enabled. Harness, raw results and caveats:
[`integration/`](integration/README.md). This is **not** a Cloudflare Workers test and **not** a VPS test.

Builds: agent `old` = `main` 548a376; `new` = draft #172 head; `fix` = `main` + PR #186; `new+fix` = #172 + #186 (merged locally, not pushed).
Server `new` = vpn-web #84 + #85 (PR #85 holds the fixes below); `old` = vpn-web `main`'s handlers behind the same Node adapter
(an **emulation** of the legacy Pages handlers).

## Verdict for #172 / #84

**Not ready to enable agents; not mergeable as-is.** The paired stack works and cuts idle traffic ~12×, but integration found five defects
(two inside #84, three affecting `main` today), three of which are fixed in branches. Two remain open and need an owner/design decision (I-100, I-102).
Draft #172 stays a draft. Worker CPU is UNVERIFIED.

## Defects found, with reproduction

| ID | Where | What | Before → After (real stack) | Status |
|---|---|---|---|---|
| I-99 | agent, **exists on `main`** | `adopt_snapshot` refuses every same-revision change, but `revoked` is derived from live entitlement (billing lapse, suspension, device revocation) which changes without a new revision. Revocation never applied | revocation **never** applied (old/old, old/new, new/old, new/new) → 3.4–4.0 s | fixed in singbox-vpn **#186** (strict-tightening rule). Un-revoke still refused → I-100 |
| I-98 | vpn-web | `authenticateNode` returned null on a DB error → 401 on every agent endpoint; agent treats 401 on job complete/fail as **Terminal** (report dropped) | 401, no Retry-After → 503 + Retry-After 5, recovers | fixed in vpn-web **#85** |
| I-97 | vpn-web #84 | event hub answered a pre-change cursor from a stale per-node cache when no waiter was active on that instance | `timeout` 7 s after revocation; found on the next poll (≤25 s) → `change` in 42 ms | fixed in vpn-web **#85** |
| I-101 | vpn-web #84 | app deployed before migration `20261018000000` ⇒ `/events` answers 503 ⇒ the agent's shared gate throttles claims and reconciliation | first apply 10.4 s, revocation 7.0 s, claims 1–2/30 s → 0.4 s, 2.0 s, normal; one 501 then polling | fixed in vpn-web **#85** |
| I-100 | agent × control plane | un-revoke (renewal after lapse) at the same revision is refused; paying customer stays locked out until a revision bump | not reproduced end to end; pinned by a unit test | **OPEN** — options: advance the revision on entitlement transitions (DB triggers for status/suspension + a periodic tick, because grace expiry is time-based and writes nothing), or an authenticated monotonic entitlement epoch |
| I-102 | agent × control plane | one out-of-policy credential (lifetime outside 6 h–30 d) makes the agent reject the **whole** snapshot, blocking valid credentials and revocations on that node; DB has no CHECK | reproduced: first apply never happened | **OPEN** — validate lifetime in the control plane; decide whether the agent should quarantine only the bad credential |

## The 16 requested checks

| # | Check | Result | Evidence |
|---|---|---|---|
| 1 | new agent + new server | PASS after fixes (before: revocation never applied) | `results/new-new.json`, `results/newfix-new.json` |
| 2 | new agent + old server | PASS for compatibility: `/events` → 404 → agent falls back to polling, no throttling, no storm (1,440 req/h projected at 5 s polling, same as old). Revocation not applied (I-99) | `results/new-old.json` |
| 3 | old agent + new server | PASS for compatibility (works, polls, no storm); revocation not applied (I-99) | `results/old-new.json` |
| 4 | old agent + old server baseline | baseline: works; revocation not applied (I-99 is on `main`) | `results/old-old.json` |
| 5 | with / without new migration | PASS after fix: fallback to per-device entitlement works, revocation reflected; `/events` 501. Before fix: 503 degradation (I-101) | `results/s7.txt`, `results/s8-*.json` |
| 6 | multiple API instances | PASS: two instances over one DB agree on ETag/304, events, revocation, jobs (instances are stateless apart from the per-process hub) | `results/s1.txt`, `s2.txt` |
| 7 | repeated 429 / Retry-After | **Partly.** The Node API never emits 429 (only 503 busy); a real 429 comes from Cloudflare and cannot be produced here. Agent-side behaviour (gate, delta-seconds and HTTP-date parsing, additive jitter, in-flight success not clearing the minimum) is covered by #172's wiremock unit tests: 173 pass (independently re-run). Integrated 429: **UNVERIFIED** | `cargo test -p provisioning-agent` on #172 |
| 8 | malformed / instantly returned events | server side PASS (garbage and oversized cursors → `resync`, never an error or hang). Agent side (eventless `200 {}`, floor, 1818-req/s storm case) covered by #172 unit tests only | `s2.txt` |
| 9 | 404, 405, 409, 501, 5xx | integrated: 404 (old server), 409 + Retry-After (superseded waiter), 501 (missing migration), 503 (DB down), 401 (bad key). 405: unit tests only | `s2/s3/s7`, `new-old.json` |
| 10 | expiry during an outage | **PASS.** DB stopped; credential with 40 s left was removed from the live set 40.3 s later with the control plane unreachable (agent re-renders on its local clock; `external.tick` runs every loop) | `s6.txt` |
| 11 | revocation with missed/delayed events | server catch-up PASS (stale cursor still reports the change, after fix I-97). End to end with the fixed agent: revocation applied 3.4–4.0 s, and one issued right after a 25 s outage applied in 3.5–3.7 s | `fix-new.json`, `newfix-new.json`, `s2.txt` |
| 12 | stale / replayed snapshots | agent unit tests (older revision refused, equivocation refused, 128 pass). Integrated replay: **UNVERIFIED** |  |
| 13 | agent restart, network failure, reconcile | PASS: revocation issued while the agent was down applied 0.3 s after restart; DB-outage recovery; no storm during outage (event mode: 2 requests in 25 s) | `s6.txt`, `newfix-new.json` |
| 14 | ETag / 304 | server PASS on both instances (weak validator, list form, stale ETag → 200, revocation cannot hide behind a 304). Agent conditional-fetch: unit tests + one real 304 observed | `s1.txt`, `s8` output |
| 15 | credential rotation | PASS: credential secret rotation applied in 5.0 s via a new revision; node API-key rotation: old key 401, new key 200 within the 1 s test TTL (production cache TTL default 5 s) | `s6.txt` |
| 16 | request budget and CPU | request budget **measured** (below). Worker CPU: **UNVERIFIED** (no Workers runtime here; local Node CPU would not prove it). vpn-web's own `scripts/loadtest` (mock backend) was not re-run | below |

## Request budget

Measured with the real agents, 30 s idle window, `poll_interval_secs = 5` (production default is 3 s):

| Agent | Server | Requests in 30 s | Projected / agent-hour |
|---|---|---|---|
| old (`main`) | new or old | 6 `/authorizations` + 6 `/claim` | 1,440 |
| new (#172) | new | 1 `/events` long-poll | 120 (+ the 60 s reconcile, not inside this window) |
| new (#172) | old | 6 + 6, then no `/events` | 1,440 (falls back cleanly) |

Model for fleets (agents, not users — user count changes the DB work per fetch, not the request count). #172's own model at the 3 s default:
3,600 requests/agent-hour polling; 324 in event mode (180 reconcile + 144 long polls), plus ~60 heartbeats in both.

| Agents (nodes) | Polling / hour | Event mode / hour | Polling / day | Event mode / day |
|---|---|---|---|---|
| 1 | 3,600 | 324 | 86,400 | 7,776 |
| 10 | 36,000 | 3,240 | 864,000 | 77,760 |
| 100 | 360,000 | 32,400 | 8.64 M | 0.78 M |

Caveats: (a) event mode needs the long-lived Node API; on Cloudflare Pages `/events` answers 501 and agents poll, so the saving does **not** apply
there. (b) The hub adds a constant DB cost per API instance regardless of agents: 1 wake-token query per second while any agent waits plus 1 entitlement query per 5 s
(≈4,300 queries/hour/instance). (c) Compare with plan quotas before enabling anything; this document does not.

## What is still not proven

Cloudflare Workers CPU/request budgets; a real 429 end to end; replayed/stale snapshot through HTTP; mixed agent builds on one node;
two agents on one node (the hub's 409 handles two waiters, the double-apply case was not run); behaviour when `vpn-admin` actually restarts sing-box.
**No agent was re-enabled and nothing was deployed.**
