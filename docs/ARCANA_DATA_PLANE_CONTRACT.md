# Arcana data-plane contract (phase 1)

## Implementation status

**Implemented and tested in this repository:** the authorization policy validator; rolling native
windows; correct interval-concurrency validation; typed opaque identifiers; projection through the
existing lease-pool input into the single hardened `CompatUser` store; renderer-side revalidation;
atomic store/config application; no-restart expiry extension when the rendered credentials are
unchanged; bounded final-expiry reconciliation; and restart-backed revocation. Legacy user
documents remain readable and keep their established expiry behavior.

**Control-plane work required in `vpn-web`:** emit the extended lease-pool fields (`principal_id`,
`credential_id`, `class`, `valid_from`, `expires_at`, and `revoked`), schedule 15-minute native
renewals, allocate separate A/B slots during compatibility rotation, and consume the capability
matrix. Unknown input fields are rejected by the node.

**Client work required in `tamara-next`:** perform native renewal, enforce strict routing/kill
switch behavior across a server restart, and consume logical route declarations. This repository
does not claim those cross-repository pieces are complete.

The provisioning agent's 0600 lease state is the node-side desired-state authority. It persists a
stable CSPRNG native principal per slot and a CSPRNG credential ID per generation, plus the rolling
window, revocation state, generation and protocol secrets. `vpn_admin_input()` writes those fields
to a 0600 file in a 0700 temporary directory; `vpn-admin` validates and atomically projects the
complete set into `users.json`; the role-aware renderer validates again before atomic sing-box
activation. Legacy state is assigned opaque IDs without changing secrets, and the complete sync
replaces both legacy `lease-*` and authorization-backed `cred_*` projections, preventing duplicates.

Before initial activation and before a static revision can change live configuration, the REALITY handshake hostname is resolved and
the whole answer set is rejected if even one IPv4/IPv6 address is loopback, private/ULA,
link-local, CGNAT, metadata, multicast, documentation, benchmarking, unspecified, or reserved.
The node egress firewall remains the use-time defense against DNS rebinding after validation.

## Authorization lifecycle

Nodes receive no account, email, payment, subscription-token, or billing fields. Each record has
an opaque `principal_id`, a separately opaque `credential_id`, protocol secret(s), credential
class, `valid_from`, `valid_until`, and `revoked`. Time intervals are **[valid_from, valid_until)**.
Every render and process start recomputes the active set from persisted bounds; expired records
cannot return after restart. Records are sorted by principal and credential id before rendering.

Native authorization is a 30-minute lease, renewed at approximately 15 minutes. Renewal extends
server authorization while preserving the UUID/password, and consequently does not intentionally
disconnect the tunnel. With a healthy provisioning agent, expiry is enforced by its next poll and
apply after `valid_until`. If the agent is unavailable, the persistent systemd fallback runs every
10 minutes, so the hard operational bound is `valid_until + 10 minutes + apply time` (and therefore
up to roughly 40 minutes after the last successful renewal). The 10-minute fallback deliberately
coalesces expirations because each effective removal restarts sing-box and disconnects every user.
Revocation bypasses natural expiry and is applied at the fastest safe poll plus
validate/atomic-swap/restart boundary.

Compatibility authorization is configurable from 6 hours through 30 days. The control plane
chooses a duration based on the target client's documented refresh/background behavior; the node
does not silently invent one. Rotation permits current A and next B concurrently. Pairwise overlap
is capped at 48 hours and no principal may have more than two simultaneous credentials. Either
credential can be revoked independently.

Because sing-box does not provide an authenticated in-process user reload, an active-set change
uses the existing transaction: render restricted candidate, `sing-box check`, atomic rename,
restart and verify, or roll back. There is never an open intermediate config. Active TCP/UDP flows
may be interrupted by that restart and clients must reconnect through the tunnel; a client kill
switch/strict route is required to prevent direct fallback. Expiry-only lease extension that does
not alter the rendered secrets requires no restart.

## Logical route declaration

The control plane resolves a public logical route to physical endpoints before submitting it:

```json
{"route_id":"route_opaque","location":"de","mode":"privacy_plus",
 "entry_endpoint_id":"endpoint_opaque_a","exit_endpoint_id":"endpoint_opaque_b"}
```

`route_id` is public and stable; physical endpoint ids are node-facing only. `fast` requires only
the exit endpoint. `privacy_plus` requires distinct entry and exit endpoints and an explicit
nested client composition. Missing/invalid hops are errors—never a Fast fallback. Relays retain
reject-by-default Internet egress and may forward only to declared exits.

## Client capability matrix

| Client output | VLESS+REALITY Fast | Hysteria2 Fast | Privacy+ | Overlap | Minimum safe lifetime |
|---|---:|---:|---:|---:|---:|
| Arcana native/full sing-box | yes | yes | yes, nested detour | yes | 30 minutes, silent renewal |
| generic full sing-box config | yes | yes | yes, nested detour | by subscription refresh | 6 hours |
| generic full Xray config | yes | no | unsupported until real-binary nested validation exists | by refresh | 6 hours |
| Hiddify share links | yes | yes | **unsupported** | by refresh | 24 hours recommended |
| Shadowrocket share links | yes | yes | **unsupported** | by refresh | 24 hours recommended |
| INCY share links | yes | unproven | **unsupported** | by refresh | 24 hours recommended |

The canonical machine-readable matrix is `fixtures/arcana-data-plane/client-capabilities-v1.json`.
Consumers must reject unknown modes or a capability with `privacy_plus=false`; they must never
substitute Fast. Full configs, rather than individual share links, are required for two-hop route
composition.

## `vpn-web` integration

1. Native pool IDs are generated locally from the OS CSPRNG: a principal remains stable for a
   slot, while a credential ID changes with each secret rotation. Future externally assigned IDs
   use the same validated wire fields. Mint external-device principals (`ext_…`) in the control
   plane because only it knows device continuity; never derive either ID from account data.
2. Select a declared lifetime within the class policy; renew native leases every 15 minutes.
3. During rotation create B with explicit bounds, publish both during the bounded overlap, then
   remove A. Send revocation immediately rather than waiting for expiry.
4. Display logical route names only. Resolve them to node-facing endpoints internally.
5. Read the capability matrix before offering a mode. If Privacy+ is false, omit/disable it with
   an explicit unsupported result; never return a Fast profile for that request.
6. Never include customer identity or subscription/billing tokens in node jobs or logs.

## External compatibility authorization synchronization

The two node synchronization calls have disjoint ownership:

* `POST /api/agent/leases/sync` exchanges the node-created native slot pool. It may
  replace only `native_*` principals (and legacy `lease-*` slot records).
* `GET /api/agent/authorizations` returns the complete desired compatibility set
  assigned to the authenticated node. Its strict response is
  `{"authorizations":[...]}`; each item contains only `principal_id`,
  `credential_id`, `class`, `valid_from`, `valid_until`, `revoked`, and optional
  `vless_uuid` / `hysteria2_password`. Unknown fields are rejected. Account,
  billing, subscription-token, and device-label data must never be sent.

The agent validates `ext_*` / `cred_*`, compatibility class, protocol secrets,
lifetime, two-credential concurrency, and the 48-hour overlap bound. It persists
the accepted snapshot in a mode-0600 file before invoking `vpn-admin
external-authorizations`. That command preserves native, reserved-probe, legacy,
and operator users; it passes the merged store through the normal candidate
render, `sing-box check`, atomic replace, restart, health verification, and
rollback transaction. The agent does not treat the snapshot as applied unless
`vpn-admin` reports that the live service was verified.

Revocation and newly fetched desired state are therefore bounded by the agent
idle poll interval (configured `poll_interval_secs`, clamped to 1–60 seconds),
the HTTP client timeout (30 seconds), and the 60-second apply timeout: a
conservative 150-second bound while the control plane and node are healthy. There is no separate urgent wake-up contract,
so this is deliberately not described as immediate. Locally known expiry does
not depend on either control-plane endpoint: on each loop the agent compares the
active credential set and reconciles an expiry transition. A process restart
first re-applies the durable snapshot using current node time, so it cannot
resurrect an expired credential.

Only an authenticated, successfully parsed response replaces that durable
desired state. In particular, a successful `{"authorizations":[]}` is an
authoritative request to remove every externally managed compatibility
credential, while a timeout, non-success HTTP status, malformed JSON, unknown
field, invalid secret, or invalid validity window leaves the last accepted
snapshot intact. Keeping the snapshot during an outage does not extend any
authorization: its original `valid_until` remains authoritative and local
expiry still removes it. Conversely, a node cannot enforce a revocation it has
never received before that bound; poll, HTTP, and apply timing above describe
the healthy-path delivery bound, not a guarantee during a partition. Applying
an unchanged effective set (including a differently ordered response) does not
restart sing-box; a secret, membership, activation, revocation, or expiry
transition does require live reconciliation and may restart the service.

Logical-route publication remains a control-plane responsibility. A new target
must not become subscription-visible until that node has fetched and reported
its authorization live. The old target must retain the authorization through a
bounded subscription-refresh overlap, after which its next complete snapshot
omits it. A failure target follows the same prepare/confirm/publish ordering;
there is no Privacy+ to Fast fallback.
