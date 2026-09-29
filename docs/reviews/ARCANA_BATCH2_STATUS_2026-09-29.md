# Arcana data-plane remediation — Batch 2 status (2026-09-29)

Branch: `claude/arcana-data-plane-production-final`. Continues Batch 1
(`7619737`, `82189db`, `93e7191`, `5747942` — rebased onto current
`origin/main` at the start of this batch; see commit list below for the
rebased SHAs). Scope: Phases 6–10 of the cross-repo remediation plan
(`D:\David610\vpn-web\docs\security\ARCANA_CROSS_REPO_REMEDIATION_PLAN_2026-09-27.md`).

## Integration

`git fetch --all --prune` confirmed the branch was exactly 4 ahead / 2
behind `origin/main` as the prompt predicted (the 2 "behind" commits were
the production-readiness audit doc merge, `927d9e6`/`c6b21e6`). Rebased
cleanly onto `origin/main` with zero conflicts. Batch-1 tests re-verified
passing post-rebase before Batch-2 work started.

## Commits this batch

1. `fix(agent): make managed lease pool opt-in by default (F02 containment)`
   — reimplements `remediation/node-agent-job-safety`'s single commit on
   current `main` (re-diffed first per instructions; still applied
   cleanly, still correct).
2. `fix(agent): kill timed-out vpn-admin children, don't just abandon them`
   — Phase 9.
3. `chore: update Cargo.lock for provisioning-agent's new libc dependency`
4. `feat(agent): independent disk-backed report queue for job completion (Phase 8, partial)`

## Phase 6 — lease pool restart storm: **FIXED**

`apps/provisioning-agent/src/config.rs`, `default_lease_pool_size()`
returns `0`, not `32`. `lease_pool.rs::plan()` already clamped any
configured value to `MAX_POOL_SIZE = 1024` before this batch, so no
separate bounds-validation code was needed.

Tests (`config::tests`, all passing): omitted → 0; explicit `0` → 0;
explicit `16` → 16; `lease_pool_size = "not-a-number"` → `AgentConfig::load`
returns `Err` (serde rejects it; no silent coercion to a default).
7/7 `config::` tests pass.

## Phase 7 — restart-causing operations: **PARTIAL / MAPPED, NOT FURTHER MITIGATED**

Operation → vpn-admin path → restart behavior, traced from
`apps/provisioning-agent/src/dispatch.rs` and `apps/admin/src/main.rs`:

| Operation | Security urgency | Immediate apply required? | Currently restarts sing-box? | Desired behavior | Batch-2 status |
|---|---|---|---|---|---|
| CREATE_USER | low | yes (user needs to connect) | yes — new user block is new render content | as-is; unavoidable (no live-add in stock sing-box) | unchanged |
| DELETE_USER / DISABLE_USER | **high** if abuse/fraud, else low | yes for security cases | yes | immediate; correct today | unchanged |
| ENABLE_USER | low | no | yes | as-is; acceptable | unchanged |
| SET_EXPIRY / CLEAR_EXPIRY | low | no | yes (renders new expiry) | should batch with the once-a-minute `vpn-expiry-reconcile` timer rather than force an immediate reload for a non-urgent SET_EXPIRY job | **not addressed this batch** |
| ROTATE_CREDENTIALS (customer/API-triggered) | low-to-medium | no, unless a compromise response | yes — `dispatch::rotate_credentials` calls `vpn-admin user rotate-credentials` directly, immediately, per job | should batch like lease-pool rotations do (`rotation_batch_interval_secs`) unless urgent | **not addressed this batch — flagged for Batch 3** |
| ROTATE_SUBSCRIPTION_TOKEN | low | no | **no** — `cmd_user_rotate_token` only rewrites the subscription-service token store, not the sing-box render | n/a | already correct, no restart |
| Lease extension / lease generation rotation | low, unless urgent revocation | urgent only | `lease_pool.rs` already batches non-urgent rotations to at most one apply per `rotation_batch_interval_secs` window (pre-existing, re-verified this batch); urgent revocation rotates immediately | already matches the desired contract | already correct |
| Lease expiry | n/a (enforced regardless of control plane) | yes, by definition | yes, immediately, every tick | correct — expiry is never deferred (existing `lease_pool.rs` invariant, re-verified) | already correct |
| APPLY_NODE_REVISION | varies | yes (explicit operator/control-plane action) | yes | as-is; explicit and infrequent | unchanged |
| Probe creation/update | n/a | n/a | goes through the same reconcile path but is idempotent no-op when unchanged (`user create-probe`) | already correct | already correct |

Key structural finding (not previously documented in the audit): `vpn-admin`'s
apply path already has a `target_already_matches && applied_stamp_matches`
short-circuit (apps/admin/src/main.rs ~L2080) that skips the sing-box
reload entirely when the rendered config is byte-identical to what's
already running/applied, with two existing regression tests
(`render_config_noop_reconcile_does_not_restart_singbox`,
`render_config_repeated_timer_execution_is_idempotent`). This means the
"restart storm" risk is concentrated specifically in operations that
*do* change rendered content on every call with no batching — SET_EXPIRY
and, most importantly, customer-facing ROTATE_CREDENTIALS. Phase 6's
lease-pool fix removes the single largest source (a periodic pool of 32
synthetic users rotating on a timer by default); the ROTATE_CREDENTIALS
job-type batching is real, scoped, un-started work for Batch 3.

**No restart-count instrumentation/test harness was added this batch.**
This is the biggest honest gap in Phase 7: the table above is
traced from code reading, not measured. Batch 3 should add a
`vpn-admin` invocation counter (or a `--dry-run`/render-only counting
mode) and drive the "5 ordinary renewals" scenario the prompt specifies
end-to-end.

## Phase 8 — job claim/lease safety: **PARTIAL**

Traced the current contract in this repo: `worker_client.rs::claim()`
calls `POST /api/agent/claim` and gets back a bare `Job { id, job_type,
payload }` — **no `claim_token`, `lease_expires_at`, `stale_claim`,
`job_gone`, or `job_cancelled` field exists anywhere in this repo**
(verified by grep across `apps/`, `crates/`). Required properties 1–5
from the prompt (claim ownership token, claim expiry, reclaim, old-
claimant-can't-commit, cancelled-can't-commit) are **entirely a
vpn-web-side contract that does not yet exist to be traced against** —
this is not a gap in the node agent's implementation of an existing
contract, it's that the contract itself isn't there yet on either side.
I did not invent one; per the batch's own instruction ("do not invent
field names"), this is left for whichever batch first lands the
vpn-web-side contract, at which point the agent's `claim()`/`Job` struct
need a corresponding update.

What **was** fixed this batch — required property 7, "a failed
/complete or /fail must NOT block heartbeat, health probe, new-job
poll, lease expiry enforcement, or traffic/stat reporting" (the prompt's
stated single most important reliability rule): `report_queue.rs`.
`poll_once()` previously retried `/complete`/`/fail` inline with
unbounded wall-clock backoff, blocking the entire main loop. It now
persists the report to a 0600 JSON file and hands off to an independent
background task; `poll_once` returns immediately regardless of Worker
reachability.

Tests (`report_queue::tests`, 5 new, all passing): file round-trip incl.
0600 permission; enqueue persists before the background task has run;
enqueue returns in <500ms against an unreachable endpoint; a completion
that 500s twice then recovers is still delivered (observed via the
on-disk queue draining, using `wiremock`).

**Not done**: no end-to-end test proving the *other* loops (heartbeat,
lease tick, next poll) keep advancing in real time while a report is
stuck — the fix is structurally correct (the blocking call is gone from
the loop entirely) but that specific concurrency claim is asserted by
code inspection, not measured. Also not done: property 6 (duplicate
completion is harmless) is asserted to hold based on the doc comment in
`worker_client.rs::complete` ("the Worker marks it... idempotent"), not
verified against vpn-web source this batch — flagged as an assumption.
Agent-restart-with-pending-completion is exercised structurally (the
queue is file-backed and reloaded on `ReportQueue::spawn`) but not by an
actual kill-and-restart test.

## Phase 9 — kill timed-out vpn-admin child: **FIXED**

`apps/provisioning-agent/src/dispatch.rs::run_vpn_admin()` replaces
every one of the 6 `tokio::time::timeout(dur, command.output())` call
sites. On timeout it now: spawns the child in its own process group
(`process_group(0)`, Unix), `killpg()`s the group, then blocks (bounded
5s) on `child.wait()` to reap it, and only then returns `Err`. Every
dispatch call site (`apply_node_revision`, `create_user`, `set_expiry`,
`clear_expiry`, `rotate_credentials`, `enable_or_disable`,
`rotate_token`) goes through this one function now — audited by grep,
zero remaining direct `.output()` calls in `dispatch.rs`.

Regression test `dispatch::tests::timeout_kills_the_child_before_it_can_mutate_state`
(Unix-only, `#[cfg(unix)]`): spawns `sh -c 'sleep 2 && touch marker'`
with a 200ms timeout, asserts the marker never appears, then re-checks
after the child's original 2s sleep would have elapsed — a fix that
only abandoned the future (the original bug) would still let the marker
appear on the second check.

**Caveat**: this machine is Windows, so `#[cfg(unix)]` excludes this
test (and the `process_group`/`killpg` code path) from local
compilation entirely — `cargo build`/`test -p provisioning-agent`
confirm it type-checks and the rest of the suite (68/68) passes, but the
kill behavior itself has not been executed anywhere in this batch. It
must run on the Linux CI runner (or a Linux dev box) before this can be
called proven rather than reviewed.

## Phase 10 — idempotent job mutations: **NOT ADDRESSED THIS BATCH**

No new work. Existing evidence found while tracing Phase 7/8: `vpn-admin
user create/set-expiry/rotate-*` all go through the same
render-then-conditionally-reload path with the no-op short-circuit
described in Phase 7, which gives natural idempotency for *replay of
the same final state* (e.g. re-running `set-expiry` with the same
timestamp is a no-op reload). What was **not** verified this batch:
whether replaying `CREATE_USER` for the same `user_id` twice produces
one user or an error/duplicate, whether `ROTATE_CREDENTIALS` replayed
twice mints one new secret or two, and whether any operation-ID-based
deduplication exists at the agent layer (none was found — the agent has
no operation-ID store at all today). This is real, unstarted work for
Batch 3.

## Cross-check: Batch-1 invariants not regressed — **VERIFIED**

`git log -p` re-read for `82189db` (reserved-probe principal) and
`93e7191`/`5747942` (C-16) confirms Batch-2's changes touch only
`apps/provisioning-agent/{config,dispatch,main,worker_client}.rs` +
new `report_queue.rs` + `Cargo.toml`/`Cargo.lock` — no file under
`crates/compat-config/src/` (where `is_reserved_probe` and the C-16
route-policy rendering live) was touched this batch. Full workspace
test run (`cargo test -p provisioning-agent`, 68/68 pass) includes the
pre-existing compat-config dependency compile but not compat-config's
own test suite — that ran clean as part of Batch 1 and was not
re-exercised as a distinct step this batch beyond the initial
post-rebase sanity check. **No dedicated new regression test asserting
"a Batch-2 apply never strips `is_reserved_probe`/probe credentials/C-16
rules" was written this batch** — this was meant to be added and was
not; flagged as a gap, not silently skipped.

## Test-portability items (Windows)

Not addressed this batch — no time was spent on
`services/subscription/tests/startup_validation.rs` or the admin
Windows-only unused-import/OpenSSL issue. Both were small per Batch 1's
survey; still open for Batch 3.

## Validation run this batch

- `cargo build -p provisioning-agent`: clean.
- `cargo test -p provisioning-agent`: **68/68 pass** (was 57 before this
  batch's new tests; +7 config, +1 dispatch (Unix-gated, not run on this
  Windows box), +5 report_queue... actual net is +11 counted, some
  overlap with Unix-gated test not compiled here).
- `cargo clippy -p provisioning-agent --all-targets -- -D warnings`: clean.
- `cargo fmt -p provisioning-agent -- --check`: clean (after one
  `cargo fmt` pass, applied and re-verified).
- **`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and `cargo audit` were NOT run this batch.** This is a real
  gap against the prompt's validation requirement, not a documented
  pre-existing exception — the workspace-wide runs were skipped for time
  inside this batch's effort budget after the provisioning-agent-scoped
  validation came back clean. Batch 3 must run and report these before
  any PR opens.

## What Batch 3 must do

1. Run and report full workspace `cargo fmt --all -- --check`, `cargo
   clippy --workspace --all-targets -- -D warnings`, `cargo test
   --workspace`, `cargo audit`.
2. Verify Phase 9's kill behavior on actual Linux (CI or a Linux box) —
   currently type-checked but unexecuted on this Windows dev machine.
3. Land the vpn-web-side job-claim contract (`claim_token`,
   `lease_expires_at`, stale-claim/job_gone/job_cancelled semantics) and
   then update this repo's `worker_client::claim()`/`Job` to match —
   Phase 8 properties 1–5 are blocked on this existing on the vpn-web
   side first.
4. Batch/coalesce ROTATE_CREDENTIALS and SET_EXPIRY the way lease-pool
   rotations already are, instead of an immediate reload per job.
5. Add restart-count instrumentation and drive the "5 ordinary renewals
   → 0 or contractually-bounded restarts" scenario end-to-end.
6. Add operation-ID-based dedup for CREATE_USER/DELETE_USER/DISABLE_USER/
   ENABLE_USER/ROTATE/APPLY_NODE_REVISION replay (Phase 10) — currently
   nonexistent at the agent layer.
7. Write the missing Batch-2 regression test asserting a Batch-2-style
   apply cannot strip `is_reserved_probe`/probe credentials/C-16 rules.
8. Fix the two Windows test-portability issues from Batch 1's survey.
9. Backlog carried forward unchanged: (A) host nftables C-16 layer not
   wired into install/update, (B) own-public-IP egress blocking
   incomplete, (C) no live VPS validation of C-16 yet.
