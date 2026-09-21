# VPS provisioning agent — design

Status: DRAFT — awaiting owner approval before any implementation.

Extends `docs/superpowers/specs/2026-09-20-vpn-website-mvp-design.md` (§6,
§7, §9) — read that first for the payment→provisioning flow and data model
this design closes the loop on. That spec left the provisioning agent and
the Worker-side job API as future work; this is that work.

## 1. Scope

Close the gap between "a Stripe webhook inserted a `provisioning_jobs`
row" and "the customer has a working VPN config in their dashboard."
Two new pieces:

- **`vpn-web`**: Worker-side (Cloudflare Pages Functions) API the agent
  polls, plus `GET /api/vpn/config` for the dashboard to retrieve the
  provisioned config.
- **`singbox-vpn`**: a new `apps/provisioning-agent` Rust binary that runs
  on the VPN VPS as a systemd service, polls the Worker over outbound
  HTTPS only, and drives the existing `vpn-admin` CLI.

**Out of scope for this design** (confirmed with owner):
- §7's misuse-detection sampling (distinct-IP counting → `abuse_signals`)
  — separate, later work.
- Multi-node fleet orchestration — the `nodes` table and `node_id`
  scoping below make a second node an additive row, not a redesign, but
  actually adding a second node is not part of this piece of work.

## 2. Architecture

```
Stripe webhook (already merged)
  → provisioning_jobs row (status=pending, node_id='node-1')

Provisioning agent (on the VPS, outbound HTTPS only, no inbound admin
surface added to the VPS — unchanged from the parent spec)
  poll loop, every 15s:
    POST /api/agent/claim  (Bearer: this node's raw API key)
      → Worker atomically claims the oldest pending job for this node_id
      → { job: null } if nothing pending, or { job: {...} }
    dispatch by job_type to the matching `vpn-admin` subcommand
    on success: POST /api/agent/jobs/:id/complete  { result }
    on failure (after 3 local retries): POST /api/agent/jobs/:id/fail  { error }

Worker (Cloudflare Pages Functions, vpn-web)
  /complete: creates/updates vpn_accounts, AES-GCM-encrypts any
             subscription URL into vpn_secrets, marks job done
  /fail:     records the error, marks job failed, sends one Resend email

Dashboard
  GET /api/vpn/config (Supabase-session-gated)
    → checks subscriptions.status active, decrypts vpn_secrets,
      returns with Cache-Control: no-store
```

The agent never receives a Supabase credential — its only credential is
the per-node shared secret described below. This preserves the parent
spec's "no inbound admin surface added to the VPN VPS" property: the
agent only ever makes outbound calls.

## 3. Data model additions (`vpn-web`, new migration)

```sql
create table public.nodes (
  node_id text primary key,
  api_key_hash text not null,  -- sha256 hex of the raw key; raw key never stored
  created_at timestamptz not null default now(),
  revoked_at timestamptz
);
-- service_role only; never exposed to anon/authenticated (no RLS policy
-- grants needed — follows this schema's existing revoke-by-default
-- pattern for service-role-only tables like vpn_secrets/stripe_events).
alter table public.nodes enable row level security;
revoke all on public.nodes from anon, authenticated;
```

A node's raw API key is generated once per node by an operator-run script
(not a public endpoint) and pasted into that VPS's agent config file. Only
the hash is ever stored server-side. Revoking a node's access going
forward is `update nodes set revoked_at = now() where node_id = ...` — no
effect on other nodes.

Job claiming needs a genuinely atomic claim (two nodes, or a slow request
and a retry, must never claim the same job). Implemented as a Postgres
function called via `supabase.rpc(...)`, not a plain REST `UPDATE`:

```sql
create or replace function claim_next_job(p_node_id text)
returns setof provisioning_jobs
language sql
security definer
set search_path = ''
as $$
  update public.provisioning_jobs
  set status = 'claimed', claimed_at = now()
  where id = (
    select id from public.provisioning_jobs
    where node_id = p_node_id and status = 'pending'
    order by created_at asc
    limit 1
    for update skip locked
  )
  returning *;
$$;
revoke all on function claim_next_job(text) from anon, authenticated, public;
```

Called only from the Worker's `/api/agent/claim` handler using the
service-role client, after that handler has already authenticated the
node's Bearer key — `security definer` here is standard Postgres practice
for this row-skipping pattern, not a client-facing privilege escalation
(no grant exists for anon/authenticated to call it directly).

## 4. Worker API (`vpn-web`, new Cloudflare Pages Functions)

All three require `Authorization: Bearer <raw node key>`, verified by
sha256-hashing the presented key and comparing against
`nodes.api_key_hash` for a non-revoked row. Same "log real error, return
fixed generic body" error pattern as the existing Stripe webhook.

- **`POST /api/agent/claim`** — body: none (node id comes from which key
  authenticated). Calls `claim_next_job`. Returns `{ job: null }` or
  `{ job: { id, job_type, payload } }`. No long-polling — the agent's own
  15s loop is the polling interval.
- **`POST /api/agent/jobs/:id/complete`** — body: `{ result }`, shape
  depends on `job_type`:
  - `CREATE_USER`: `{ vpn_user_id, subscription_url, expires_at }` →
    Worker inserts a `vpn_accounts` row (`user_id` from the job's
    original payload, `vpn_user_id` from this result, `node_id` from the
    job), AES-GCM-encrypts `subscription_url` into a new `vpn_secrets`
    row for it.
  - `ROTATE_SUBSCRIPTION_TOKEN`: `{ subscription_url }` → looks up the
    existing `vpn_accounts` row (by the job's `vpn_account_id`),
    encrypts + inserts a new `vpn_secrets` row (old ciphertext rows are
    left in place — an append-only secret history costs nothing and
    means a bug can't silently destroy the only working config).
  - `SET_EXPIRY` / `ENABLE_USER` / `DISABLE_USER`: `{}` — no
    `vpn_secrets` write, just marks the job done.
  - Every branch: `provisioning_jobs.status = 'done'`,
    `completed_at = now()`, `result = <the body>`.
- **`POST /api/agent/jobs/:id/fail`** — body: `{ error: string }`.
  Sets `status = 'failed'`, `result = { error }`. Sends one email via
  Resend (`RESEND_API_KEY` Worker secret) to
  `platte-kantig.0o@icloud.com`: job type, affected user id (if known),
  and the error string. A failed job is not retried automatically —
  someone fixes the underlying cause and manually resets
  `status = 'pending'` via Supabase Studio.

AES-GCM key: a Worker secret (`VPN_SECRETS_ENCRYPTION_KEY`), never stored
in Supabase — matches the parent spec's §4 requirement exactly. `nonce`
is 12 random bytes per encryption (already enforced by the existing
`vpn_secrets` schema's `check (octet_length(nonce) = 12)`).

## 5. `GET /api/vpn/config` (`vpn-web`, dashboard-facing)

Requires a valid Supabase session (same Bearer-access-token pattern as
`create-checkout-session`). Looks up the caller's `subscriptions.status`
— if not `active`, returns 403 with a message the dashboard can show
("no active subscription"). Otherwise looks up their `vpn_accounts` row,
fetches the most recent `vpn_secrets` row for it, decrypts, and returns
`{ subscription_url }` with `Cache-Control: no-store`. If no
`vpn_accounts`/`vpn_secrets` row exists yet (provisioning still in
flight), returns 404 with a "still being set up" message rather than an
error — the dashboard can poll this on an interval or just show a
"check back shortly" state after `?checkout=success`.

## 6. Provisioning agent (`singbox-vpn`, new `apps/provisioning-agent`)

New Cargo binary crate, sibling to `apps/admin`. Rust, matching this
repo's existing stack — reuses `apps/admin`'s own binary as a subprocess
(`std::process::Command`) rather than linking its internals directly, so
the agent stays a thin dispatcher and `vpn-admin`'s own
validate/apply/reload discipline is the only place that logic lives.

Config (file or env, mirroring `apps/admin`'s existing config
conventions): `worker_url`, `node_id`, `agent_api_key` (the raw key for
this node), `poll_interval_secs` (default 15), path to the local
`vpn-admin` binary.

Dispatch table (`job_type` → `vpn-admin` invocation):
| job_type | command | needs `--json` |
|---|---|---|
| `CREATE_USER` | `user create --name <user_id> --expires-at <ts> --json` | yes (already exists) |
| `SET_EXPIRY` | `user set-expiry <vpn_user_id> --expires-at <ts>` | no — exit code only |
| `ENABLE_USER` | `user enable <vpn_user_id>` | no |
| `DISABLE_USER` | `user disable <vpn_user_id>` | no |
| `ROTATE_SUBSCRIPTION_TOKEN` | `user rotate-token <vpn_user_id> --json` | **yes — new, see Task 0** |

On a non-`CREATE_USER` job, `vpn_user_id` comes from the job's own
`payload.vpn_user_id`, resolved by the Worker at job-*insertion* time
(not by the agent, and not at claim time) — this requires a small,
targeted change to the already-merged webhook handlers in
`functions/lib/stripe-events.js`:

- `handleInvoicePaid`'s `SET_EXPIRY` branch and `handleSubscriptionUpdated`
  /`handleSubscriptionDeleted`'s `DISABLE_USER` branches each look up the
  caller's `vpn_accounts` row by `user_id` before inserting the job, and
  set both `provisioning_jobs.vpn_account_id` and
  `payload.vpn_user_id` from it.
- `CREATE_USER` is unaffected — it still inserts with `vpn_account_id`
  null, because the whole point of that job is to create the
  `vpn_accounts` row that doesn't exist yet.
- If a `SET_EXPIRY`/`DISABLE_USER` job's lookup finds no `vpn_accounts`
  row (the account's `CREATE_USER` job hasn't been processed by the
  agent yet — a real race, since Stripe can fire a renewal before the
  first `CREATE_USER` job is claimed), the handler throws — same
  transient-vs-permanent pattern already established in that file
  (500, Stripe retries) rather than enqueueing a job with no
  `vpn_user_id` to act on.

This is a small, isolated addition to already-merged code, not a
redesign of it — the implementation plan should treat it as its own
task alongside the two new Worker endpoints, reviewed with the same
scrutiny as the original webhook (this is exactly the kind of
correctness detail this project's review process has caught before).

Retry: up to 3 attempts per job with a short fixed backoff (e.g. 2s) on a
`vpn-admin` non-zero exit, before reporting `/fail`. No unbounded retry.

## 7. Backend changes needed to `singbox-vpn` (Task 0, before the agent)

`rotate-token` currently has no `--json` output (only `create` does),
so the agent has no reliable way to parse the new subscription URL out
of it. Add a `--json` flag to `RotateToken`, mirroring `create`'s
`{"id","name","enabled","subscription_url"}` shape (rotate-token doesn't
need `enabled`/`name` necessarily — `{"id","subscription_url"}` is
sufficient). Same "never print server private keys" constraint already
documented on `create --json`.

## 8. Testing

Everything except the final real-VPS acceptance-criteria run (parent
spec §11) is testable locally, no real VPS required:

- Worker endpoints: local Supabase + `wrangler pages dev`, a fake node
  key seeded into the local `nodes` table, synthetic `provisioning_jobs`
  rows inserted directly (mirroring how the Stripe webhook plan used
  synthetic signed events).
- Agent binary: built locally, pointed at `SITE_URL=http://127.0.0.1:8788`
  and a locally-built `vpn-admin` — the full claim → dispatch → complete
  loop runs end to end against the local stack.
- Resend email: Resend's API can be called in test mode / to a real
  inbox without needing a production domain verified — verify at least
  once that a `/fail` call actually sends.

## 9. Explicitly not in this design

- §7 misuse-detection sampling — separate work, confirmed with owner.
- Actually deploying to a real VPS and running the systemd service there
  — that's the parent spec's §11 acceptance-criteria step, done once a
  real VPS exists, not part of this design's implementation plan.
- A UI/CLI for generating and revoking node keys beyond the one-off
  operator script described in §3 — no admin dashboard exists (parent
  spec §10 non-goal) and one node doesn't need one yet.
