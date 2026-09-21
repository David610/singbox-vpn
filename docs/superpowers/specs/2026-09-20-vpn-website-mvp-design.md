# VPN business website — MVP design

Status: DRAFT — awaiting owner approval before any implementation.
Owner decisions locked in this doc are marked **DECIDED**; open items before
implementation starts are marked **PREREQUISITE**.

## 1. Scope

Build a customer-facing website that lets a stranger sign up, pay monthly via
Stripe, and receive a working VPN config, backed by the existing
`singbox-vpn` repo. Tamara and Privacy+ (two-hop relay) are explicitly out of
scope for this MVP.

**DECIDED (from owner):**
- Backend: `David610/singbox-vpn`, may be modified.
- Billing: one monthly recurring plan, no trial, no one-time plans.
- Market: EU customers first, no Russia.
- Target scale: up to ~100 active paying users in month one; data model must
  make adding more VPS nodes later an additive change, not a rewrite
  (`node_id` on the relevant tables from day one).
- Sharing: one subscription usable on a couple of devices; misuse prevention
  wanted from early on, but **soft** (detect + alert + manual disable), not
  hard technical device-count blocking, for v1.
- Privacy+ (two-hop relay): deferred. Not sold or exposed in v1 UI.
- Design language: `david610.github.io` — white background, system sans
  font, mono accents, `#333`/`#666` text, `#0070f3` blue, `#eaeaea` borders,
  narrow content column, no stock VPN visuals (shields, globes, gradients).
- Base for the frontend: `David610/ml-consulting-website` (Next.js +
  Tailwind), trimmed to what a VPN SaaS site needs — not reused wholesale.

## 2. Why not touch Tamara or rebuild the VPN

`singbox-vpn` already has the hard networking work done and evidence-backed
(`docs/CLIENT_COMPATIBILITY.md`, `docs/SUPPORTED_PRODUCT.md`). Tamara is a
separate first-party client repo and is unaffected by this work — nothing
here changes its contract. The website is a new consumer of the *existing*
fallback surface (`/sub/{token}` and friends), plus the two small backend
additions in §5.

## 3. Repo layout

- **New repo `vpn-web`** (or similar) — Next.js static frontend + one
  Cloudflare Worker API. Seeded from `ml-consulting-website`'s tooling
  (Next.js/Tailwind config, build pipeline) but with the consulting content,
  schema, admin/contact code stripped. Independent from the freelance site
  going forward — no shared deploys.
- **`singbox-vpn`** gets two backend changes (§5) plus a small new
  **provisioning agent** binary/script that runs on the VPN VPS itself and
  polls the Worker API over outbound HTTPS. No inbound admin surface is
  added to the VPN VPS.

## 4. Data model (Supabase)

Tables, all with `auth.uid()`-scoped RLS except `vpn_secrets` and
`stripe_events` (service-role only, no client RLS policy at all):

- `profiles` — one row per Supabase auth user.
- `subscriptions` — Stripe customer/subscription id, status, current
  period end. Source of truth is always the latest verified Stripe webhook,
  never client input.
- `vpn_accounts` — `user_id`, `vpn_user_id` (the singbox-vpn user), `node_id`
  (which VPS this account lives on — **hardcoded to the single v1 node now,
  but present as a column from day one** so a second node later is a new row
  + agent target, not a migration).
- `vpn_secrets` — AES-GCM-encrypted subscription URL(s) for that account,
  service-role read only. Encryption key lives as a Cloudflare Worker
  secret, never in Supabase.
- `provisioning_jobs` — idempotency-keyed job queue
  (`CREATE_USER` / `SET_EXPIRY` / `ENABLE_USER` / `DISABLE_USER` /
  `ROTATE_SUBSCRIPTION_TOKEN`), status, target `node_id`. The VPS agent polls
  filtered by its own `node_id`.
- `stripe_events` — raw event log keyed by Stripe event id, for idempotent
  webhook processing and audit.
- `abuse_signals` — append-only log of the soft misuse checks in §7
  (distinct-IP count per account per day), reviewed manually, not
  auto-enforced in v1.

## 5. Backend changes to `singbox-vpn`

1. **`vpn user set-expiry USER_ID --expires-at UNIX_TIMESTAMP`** (and
   `clear-expiry`) — same locking/atomic-render/validate/reload discipline
   as the existing mutating commands in `apps/admin`. Required because
   subscriptions renew; today only initial creation takes an expiry.
2. **Fix the `/sub/{token}` share-link format for import-by-URL clients
   (Shadowrocket confirmed broken today).** Root cause, verified in
   `services/subscription/src/lib.rs`: the endpoint returns the raw share
   links with no `Content-Type` and no base64 encoding. Standard
   subscription-import clients expect the body base64-encoded (classic
   V2Ray/Shadowrocket subscription convention). Fix: base64-encode the
   `format=uri` response body and set an appropriate content type; add a
   second explicit endpoint or `format=` value for the current raw-text
   behavior so nothing that already parses it today breaks. This must be
   **verified against a real Shadowrocket import**, not just schema-checked
   — consistent with this repo's existing evidence rule
   (`docs/CLIENT_COMPATIBILITY.md`'s "no yes without evidence").
3. No other protocol/data-plane changes. Capacity/load testing for ~100
   users is a **prerequisite investigation** (§9), not a code change by
   itself — it may surface a config change (e.g. connection limits) but
   that's scoped after the numbers come back.

## 6. Payment → provisioning flow

Stripe Checkout (hosted) + Customer Portal for billing management. The
**webhook is the only writer of subscription/access state** — the
checkout-success page never grants access itself.

```
Stripe webhook (signature-verified, raw body, 2xx fast)
  → Worker: upsert `stripe_events` (idempotency), upsert `subscriptions`
  → on invoice.paid (first time): insert `provisioning_jobs` row = CREATE_USER
  → on invoice.paid (renewal): insert `provisioning_jobs` row = SET_EXPIRY
  → on subscription canceled/unpaid: insert `provisioning_jobs` row = DISABLE_USER
  → on past_due: no immediate action, just surface a warning in the dashboard
       (Stripe's own recommended pattern)

VPS provisioning agent (outbound poll only, its own node_id)
  → claims a job by idempotency key (never double-creates a VPN identity)
  → runs the fixed vpn-admin subcommand locally
  → returns the result (incl. the one-time subscription URL on CREATE_USER
    / ROTATE_SUBSCRIPTION_TOKEN) to the Worker over the same outbound call
  → Worker AES-GCM-encrypts it and writes `vpn_secrets`; the raw URL is
    never logged and never stored anywhere in plaintext after this point
```

`GET /api/vpn/config` (Worker, requires a valid Supabase session): checks
subscription is active, decrypts, returns with `Cache-Control: no-store`.
Never queried directly by the browser against Supabase.

## 7. Misuse / multi-device soft enforcement (v1)

No hard connection limiting in v1 (sing-box has no built-in concurrent-session
cap, and building real-time connection tracking + kill is a separate,
nontrivial project). Instead:

- Provisioning agent (or a lightweight periodic job) samples distinct
  source IPs per `vpn_user_id` over a rolling window from sing-box's own
  logs/stats and writes to `abuse_signals`.
- A simple threshold (e.g. many more distinct IPs than a "couple of
  devices" story explains, sustained) flags the account for manual
  review — email/log alert, no auto-disable.
- Contractual terms of service state the personal/couple-of-devices policy
  explicitly, so a manual disable has a clear basis.
- Hard technical enforcement (kill excess sessions in real time) is called
  out as a deferred v2 item, same tier as Privacy+.

## 8. Auth

Supabase email + password + required email verification. No magic-link-only
flow (Supabase's own docs note mail-scanner prefetch breaks single-use
links). OAuth providers not in v1. Supabase's own production checklist
items apply: custom SMTP (built-in email is rate-limited, not
production-grade), CAPTCHA/abuse protection on signup, RLS reviewed on
every table above.

## 9. Prerequisites (owner/infra actions before or during implementation)

These are things I cannot do for you, or shouldn't do without your sign-off:

1. **VPS provider decision** — recommendation: **Hetzner** (Germany/Finland
   datacenters, good latency for EU customers, widely used in practice for
   VPN hosting). Their public ToS/system-policy pages don't spell out
   VPN/proxy hosting explicitly either way, so **email
   abuse@hetzner.com describing the exact business before committing
   revenue to it**, same as ChatGPT's Stripe suggestion below. Contabo is a
   cheaper fallback with similarly non-explicit terms.
2. **Stripe account** — create it, and **describe the VPN business to
   Stripe support/onboarding explicitly** before going live; their
   restricted-business list doesn't name VPNs but isn't exhaustive either.
3. **Supabase project** — create it, configure custom SMTP (their default
   email is not production-grade), enable CAPTCHA on signup.
4. **Cloudflare account** — Pages + Workers, plus a domain (not Vercel —
   their AUP prohibits proxy/VPN use even for an adjacent control-plane
   site).
5. **Load/capacity test of one VPS at ~100 concurrent REALITY+Hysteria2
   users** — needed before advertising "100 users" as safe; may change VPS
   sizing or surface a need to shard sooner than expected. I can help design
   and run this once a real VPS exists.
6. **Legal**: German provider-info (§5 DDG) and cancellation-button
   (§312k/§356a BGB) text for the site; confirm whether Stripe's Customer
   Portal alone satisfies the German cancellation-button requirement or a
   custom flow is needed; VAT/OSS registration for EU consumer sales — the
   last two need an actual accountant/lawyer, not me.
7. **Company/business entity + bank account for Stripe payouts**, if not
   already in place.
8. **New repo created** (`vpn-web` or your preferred name) that I'll build
   into.

## 10. Explicit non-goals for this MVP

- Privacy+ / two-hop relay sold to customers.
- Multi-node fleet orchestration, auto-provisioning of new VPS nodes,
  cross-node load balancing.
- Hard technical device/session-count enforcement.
- Traffic charts, referrals, affiliate codes, teams, custom apps, speed
  tests, server selection, support chat, usage-based billing, admin
  dashboard beyond Stripe/Supabase/`vpn-admin` themselves.
- Clash/Clash Meta and Shadowrocket-specific extra formats beyond fixing the
  existing base64/URI path — revisit only if real customers report a
  client the fixed formats don't cover.

## 11. Acceptance criteria before calling this "done"

- A real signup → Stripe Checkout → webhook → provisioning job → VPS agent
  → working VPN connection, observed end to end on a real VPS with a real
  card (test mode) and a real client device.
- The base64 subscription fix verified against a real Shadowrocket import,
  not just a schema/unit test.
- `set-expiry` exercised by a real renewal webhook, not just a CLI call.
- Cancellation flow verified to actually disable VPN access and reflect in
  the dashboard within a defined SLA (e.g. next billing check).
- RLS reviewed table-by-table; `vpn_secrets` confirmed unreadable by the
  anon/client role.
- No plaintext subscription URL in any log, anywhere, checked by grep over
  Worker logs after a real provisioning run.
