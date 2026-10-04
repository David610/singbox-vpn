# Arcana Product Contract — V1

**Status:** frozen 2026-10-04. This is the canonical source of truth across
`vpn-web`, `singbox-vpn`, and `tamara-next` (mobile/desktop client). A copy
of this exact file lives in `docs/contracts/ARCANA_PRODUCT_V1.md` in all
three repositories. If they ever disagree, this is a bug — fix by
re-syncing, never by letting one repo's copy drift into its own truth.

This freezes *model* — names, fields, states, lifecycles — not visual
design. See `docs/design/ARCANA_FINAL_APP_DESIGN_2026-10-03.md` (in
`tamara-next`) for the presentation direction, and the 2026-10-04 "Arcana —
Full Product Transformation Plan" (not yet committed anywhere as of this
writing — ask David for a copy if you need the full rationale) for *why*
this document exists and the phased rollout it belongs to (this is Phase 0
of that plan).

This document sits **above** `tamara-next`'s existing
`docs/contracts/managed-control-plane-v1.md` ("client-side contract
proposal") in layering, not in replacement of it: that document is the
detailed, already-partially-implemented wire protocol (session model,
`GET /v1/routes` envelope, pseudonymous authorization) and remains
authoritative for wire-level field shapes. Where the two disagree on
naming (see §4), that is flagged explicitly below, not silently resolved
in either direction — reconciling it is named follow-up work, not done by
this freeze.

## 0. Non-negotiable invariants

These hold across every phase of the transformation plan, not just V1:

- **No account identity ever reaches a VPN node.** Nodes authenticate
  opaque, pseudonymous credentials (`vpn_user_id`, subscription tokens).
  Never email, billing id, Stripe customer id, or subscription name.
- **No silent two-hop → one-hop downgrade.** If `TWO_SERVER` mode's
  required hop fails, the connection fails closed — it never silently
  reconnects as `ONE_SERVER`.
- **No fabricated usage data.** Per-link/per-user cumulative traffic
  attribution is not technically trustworthy with the currently shipped
  `singbox-vpn` runtime (see `singbox-vpn/docs/reviews/
  ARCANA_PHASE1_DATA_PLANE_ASSESSMENT_2026-10-01.md`). Do not display,
  store as fact, or bill against per-link traffic numbers until that's
  fixed at the runtime level.
- **Server is canonical for capacity/entitlement.** No client — web or
  app — computes device/pack capacity from local constants it owns; it
  only renders what the account API returns.

## 1. Account

```
Account
 ├─ subscriptions (0 or more; see §3)
 ├─ devices       (see §2)
 └─ links         (see §5 — external/non-Arcana-app VPN access)
```

One person per account. No shared seats, no member invites (existing
`account_members`/`customer_accounts` tables predate this and should not
grow new multi-user functionality under V1).

Source of truth: `vpn-web`, tables `public.customer_accounts`,
`public.profiles` (Supabase `auth.users.id` mirror).

## 2. Device

A device is one Arcana application installation/session.

```
device_id        uuid
name             text          -- user-editable
platform         text          -- "windows" | "macos" | "ios" | "android" | "linux" | null
account_id       uuid
status           "ACTIVE" | "REVOKED"
current          bool          -- derived client-side: is this *this* install
created_at       timestamptz
last_seen_at     timestamptz   -- nullable; only retained if privacy policy allows it
```

No traffic-destination information on a device row, ever (see §0).

Source of truth: `vpn-web`, table `public.devices`
(`supabase/migrations/20260924000000_fleet_foundations.sql`). Removing a
device must revoke its sessions and any managed VPN identity tied to it —
this already happens via `functions/lib/device-provisioning.js`'s
`revokeDevice`; keep that invariant true through every future change.

## 3. Subscription

**Frozen for V1 (explicit decision 2026-10-04): keep the model that is
already live and coded.** The "10-seat pack" direction mentioned during
planning is *not* adopted — re-opening pricing is a separate, later
decision, not part of Phase 0.

```
plan        "arcana"
status      "incomplete" | "trialing" | "active" | "past_due" | "canceled" | "unpaid"
included_devices   3          -- INCLUDED_SEATS in functions/lib/seat-constants.js
pack_size          3          -- DEVICE_PACK_SIZE, devices per extra pack
pack_price_cents   699        -- €6.99 per 3-device pack
max_extra_packs    17         -- MAX_EXTRA_PACKS in functions/lib/account-service.js (max capacity: 3 + 17*3 = 54 devices)
capacity    included_devices + (extra_packs * pack_size)
used        count of this subscription's ACTIVE devices
renews_at   current_period_end
```

The client (web or app) renders exactly this server-reported
`{capacity, used, renews_at, status}` shape — e.g. "7 of 10 places used" —
and never recomputes capacity from a local constant. One account may hold
several subscriptions (`pickSubscriptionWithRoom` picks which one a new
device lands on); devices can move between an account's subscriptions.

Source of truth: `vpn-web`, table `public.subscriptions`
(`supabase/migrations/20260921000000_initial_schema.sql`,
`20260926000000_subscription_devices.sql`), webhook-driven only — never
client input. Legacy naming note (keep until a deliberate rename): the
`extra_seats` column and `STRIPE_SEAT_PRICE_ID` env var both mean "extra
devices in packs of 3", not literal seats.

## 4. VPN route

The client understands only:

```
location      text    -- e.g. "Germany"; backend resolves to a healthy node pool
mode          "fast" | "privacy_plus"
availability  bool
route_id      opaque string
```

Never exposed to normal UI: protocol name (VLESS/REALITY/Hysteria2/
sing-box), node hostname, node IP, internal region code, transport.

**Naming reconciliation — resolved 2026-10-04.** Three vocabularies existed:

1. `vpn-web` DB (`public.connection_profiles.routing_mode`):
   `AUTO | DIRECT | DOUBLE_HOP`.
2. `tamara-next`'s already-shipped wire contract
   (`docs/contracts/managed-control-plane-v1.md`, `GET /v1/routes` route
   payload `mode` field, covered by passing interop tests, enforced down to
   `sing_box_config_builder.dart`'s outbound count): `"fast" |
   "privacy_plus"` — one hop vs. exactly two hops.
3. This document's product-facing conceptual vocabulary, used only in
   prose/UI-copy discussions: `ONE_SERVER | TWO_SERVER`. It had **zero**
   code footprint anywhere in any of the three repos — grepped and
   confirmed absent from `singbox-vpn`'s implementation, which uses its own
   third vocabulary internally (`role=relay`/`exit`, `detour` chaining) for
   the same hop-count concept.

**Resolution: vocabulary 3 is retired.** `"fast"`/`"privacy_plus"` is
canonical everywhere this document means hop-count mode, because it is the
one of the three that is live, signed, schema-validated and already
enforces exact hop counts — not because it was first. `ONE_SERVER`/
`TWO_SERVER` must not appear in new code or docs; existing occurrences are
being swept to `"fast"`/`"privacy_plus"` as each repo is touched, not in
one blanket rename.

`vpn-web`'s `routing_mode` column is **not** migrated — `AUTO | DIRECT |
DOUBLE_HOP` remains the value at rest, because `AUTO` and `DIRECT` are
secretly the same hop-count value (`exitNodeId === entryNodeId` either
way; confirmed in `device-provisioning.js`/`scheduler.js`) differing only
in who picks the location, an axis the wire contract doesn't carry at all.
Migrating the column was weighed against a read-model translation and
rejected as unnecessary blast radius for an internal value nothing outside
`vpn-web` ever sees. Instead, every JSON response that includes
`routing_mode` now also derives the clean two-axis read model alongside it
(`functions/lib/connection-profiles.js`'s exported `routingAxes()`,
consumed by `connection-profiles/index.js`, `devices.js` and
`telegram-mini-app-service.js`):

```
hopMode       "fast" | "privacy_plus"   -- AUTO and DIRECT both "fast"
locationMode  "AUTO" | "PINNED"         -- AUTO is the only "AUTO" value
```

`routingMode` (`AUTO|DIRECT|DOUBLE_HOP`) is kept alongside these, additive,
for existing consumers — nothing that read it before had to change.

### 4a. Automatic location

`Automatic` is a first-class location choice, not a UI default dressed up
as one. Backend selects by health, capacity, network quality, failure
domain, availability, maintenance state — never a client-side hardcoded
latency race.

### 4b. Two-server mode

```
Entry: Automatic | <location>
Exit:  <location>
```

Entry and exit use independently scoped credential material. Either hop
failing fails the connection closed (§0). This is already the documented
behavior of the managed-client contract — this document doesn't change
it, only restates it as frozen.

## 5. Links (external/non-Arcana-app VPN access)

**Already fully specified and implemented — this is not an open item.**
Canonical, stable contract: `docs/contracts/ARCANA_LINKS_V1.md` (this
repo, `main`). This section is a pointer/summary, not a competing
definition; where the two differ, `ARCANA_LINKS_V1.md` wins.

```
POST   /v1/links                              create
GET    /v1/links                               list
GET    /v1/links/:id                           detail (+ its clients)
POST   /v1/links/:id/clients                   issue a client (idempotent)
POST   /v1/links/:linkId/clients/:clientId/replace-link   rotate bearer token
DELETE /v1/links/:linkId/clients/:clientId     revoke one client
DELETE /v1/links/:id                           revoke Link + all clients
```

A Link: `id, name, status (active|revoked), route_id, route_label,
configuration_family ("compatibility"), active_clients, max_clients (1–100),
created_at, revoked_at`. A client: `id, name, status, client_type
("links"), created_at, last_seen_at, revoked_at`. `route_id`/`route_label`
are exactly this contract's §4 route concept — Links consume the same
route abstraction, not a separate one.

Enforced server-side, already shipped: idempotent client creation
(`Idempotency-Key` header, replay returns no secret), capacity atomically
consumed against subscription/Link limits together, secret-bearing
`configuration_url` returned exactly once (`shown_once: true`) and never
re-revealable, replace rotates only the subscription bearer token (not
VLESS/Hysteria protocol credentials), revoke cascades Link → all child
clients transactionally. Compatibility is driven by one table
(`functions/lib/client-capabilities.js`) — today, Link-created clients
only support `"fast"` routes; `"privacy_plus"` is fail-closed
(`422 unsupported_route`), meaning **two-server routing is not yet
available through Links**, only through the Arcana app itself (§4b).

Underlying storage (`public.connection_profiles` +
`device_profile_assignments` for routing preference, `singbox-vpn`'s
`vpn_accounts`/`vpn_secrets` for the actual protocol credential) is an
implementation detail behind this API — callers (including `tamara-next`)
should talk to `/v1/links`, not those tables directly.

## 6. Signed route directory

```
version       bigint, monotonic
issued_at     timestamptz
expires_at    timestamptz
payload       the node/transport directory itself
key_id        signing key identifier
signature     Ed25519
```

Client must verify: signature, expiry, schema, monotonic version (reject
any directory whose version is lower than the last one it accepted — anti-
rollback). A compromised CDN/cache must not be able to serve an arbitrary
node directory.

Source of truth: `vpn-web`, table `public.route_directory_state`
(`supabase/migrations/20260928000000_route_directory.sql`) — currently
tracks `version` + `last_payload_hash`; the signed payload itself is
served via the route directory endpoint (see that migration's adjoining
API code for the current shape).

## 7. Connection state machine

```
DISCONNECTED → AUTHORIZING → STARTING → CONNECTING → CONNECTED
CONNECTED → RECONNECTING → CONNECTED
any state → DISCONNECTING → DISCONNECTED
any state → FAILED
any state → BLOCKED   (kill-switch engaged, fail-closed)
```

No screen infers state from which button was last pressed; the state
machine is the single source of truth, and every client (web account
pages, Windows/macOS/Linux desktop, future iOS/Android) renders *from* it
rather than maintaining parallel state.

## 8. Node lifecycle

Actual enum (`public.nodes.lifecycle_state`,
`supabase/migrations/20260924000000_fleet_foundations.sql`) — this
section restates reality rather than the transformation plan's
simplified `NEW→PROVISIONING→READY→DRAINING→UNHEALTHY→OFFLINE→RETIRED`
sketch, which doesn't match the shipped schema closely enough to freeze
as-is:

```
PROVISIONING → WARMING_UP → READY → DEGRADED
READY → DRAINING → RETIRED
any  → MAINTENANCE → READY
any  → FAILED
any  → QUARANTINED → RETIRED   (see 20261002000000_node_key_revocation.sql —
                                 only QUARANTINED/RETIRED are valid
                                 revocation-transition targets)
```

Traffic is only ever routed to `READY` nodes. Already-established sessions
may continue being served during `DRAINING`. No routing to a manually
configured, unregistered host. `desired_revision`/`observed_revision`
(both on the same table) drive reconciliation — a node whose
`observed_revision` lags `desired_revision` has a pending config change,
independent of `lifecycle_state`.

## 9. Repository ownership boundaries (restated, not changed)

```
tamara-next   UI, local app state, secure token storage, VPN state
              machine, OS VPN integration, signed route verification,
              pseudonymous-authorization consumption, kill switch.
              Does NOT own billing truth, node allocation, route
              generation, customer entitlement truth, raw infra creds.

vpn-web       auth, accounts, subscriptions, device inventory, links
              lifecycle, entitlements, signed route directory,
              authorization issuance, fleet registry/health,
              provisioning jobs, admin, billing integration, audit.

singbox-vpn   node install/runtime, node-local credential application,
              one-hop/two-hop data path, firewall/isolation,
              provisioning agent, fail-closed credential enforcement,
              node health reporting, safe update/rollback.
```

## 10. Open items (explicitly not resolved by this document)

- Links supporting `"privacy_plus"` routes (§5) — currently fail-closed
  with `422 unsupported_route`; two-hop is Arcana-app-only today.
  **Investigated 2026-10-04, implementation now complete and tested,
  capability deliberately still closed** (see `ARCANA_LINKS_V1.md`'s
  "Route compatibility" section): permanently impossible for
  every bare-connection-URI client format (listed in `ARCANA_LINKS_V1.md`) — their formats are bare
  connection URIs with no field that can express a second hop, not a
  qualification gap. For `singbox`'s format: the per-hop credential model
  (`compatibility_credentials.hop`, hop-aware `create_external_vpn_device`/
  `rotate_compatibility_credential`) and the renderer
  (`subscription-renderers.js`'s `renderSingBox`, now building a correct
  `detour` chain with its own entry-hop credential rather than the
  malformed, uncredentialed outbound an earlier version silently built)
  both exist now, covered by SQL and unit tests. `client-capabilities.js`
  still refuses `privacy_plus` for every client including `singbox`: that
  is a deliberate, separate decision, not a leftover implementation gap —
  nothing has verified a real sing-box-based compatibility client actually
  honors the rendered chain against this pipeline's own real, deployed
  node infrastructure (as opposed to `singbox-vpn`'s own proven, but
  separate, implementation). Flipping it is its own explicit follow-up
  once that verification exists.
- ~~Reconciling `routing_mode` with the `Automatic`-location vs. hop-count
  two-axis model (§4).~~ **Resolved 2026-10-04** — see §4. `routing_mode`
  stays at rest unmigrated; API responses now also carry derived
  `hopMode`/`locationMode`. Still open, deliberately deferred: an actual
  schema rename (`routing_mode` → `hop_mode` + new `location_mode` column)
  if the read-model shim ever stops being enough — not needed today.
- Per-link/per-user traffic attribution (§0, blocked on `singbox-vpn`
  runtime work).
- Android platform support (`tamara-next` currently has no `android/`
  directory).
- Subscription pricing/capacity model beyond what's frozen in §3 — if
  that's ever revisited, it's a new decision, not an amendment slipped
  into this document.

Each of these is a named follow-up, not a silent gap — do not treat their
absence from the rest of this document as permission to invent an answer
ad hoc in implementation code. Update this contract first, in all three
repos, then implement.
