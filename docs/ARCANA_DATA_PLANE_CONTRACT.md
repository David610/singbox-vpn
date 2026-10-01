# Arcana data-plane contract (phase 1)

## Authorization lifecycle

Nodes receive no account, email, payment, subscription-token, or billing fields. Each record has
an opaque `principal_id`, a separately opaque `credential_id`, protocol secret(s), credential
class, `valid_from`, `valid_until`, and `revoked`. Time intervals are **[valid_from, valid_until)**.
Every render and process start recomputes the active set from persisted bounds; expired records
cannot return after restart. Records are sorted by principal and credential id before rendering.

Native authorization is a 30-minute lease, renewed at approximately 15 minutes. Renewal extends
server authorization while preserving the UUID/password, and consequently does not intentionally
disconnect the tunnel. Missing renewal kills authorization within 30 minutes. Revocation bypasses
the lease immediately and is applied at the fastest safe validate/atomic-swap/restart boundary.

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
| Hiddify / Shadowrocket / INCY share links | yes | client-dependent | **unsupported** | by refresh | 24 hours recommended |

The canonical machine-readable matrix is `fixtures/arcana-data-plane/client-capabilities-v1.json`.
Consumers must reject unknown modes or a capability with `privacy_plus=false`; they must never
substitute Fast. Full configs, rather than individual share links, are required for two-hop route
composition.

## `vpn-web` integration

1. Mint opaque principals per external device (`ext_…`) or native installation (`native_…`).
2. Select a declared lifetime within the class policy; renew native leases every 15 minutes.
3. During rotation create B with explicit bounds, publish both during the bounded overlap, then
   remove A. Send revocation immediately rather than waiting for expiry.
4. Display logical route names only. Resolve them to node-facing endpoints internally.
5. Read the capability matrix before offering a mode. If Privacy+ is false, omit/disable it with
   an explicit unsupported result; never return a Fast profile for that request.
6. Never include customer identity or subscription/billing tokens in node jobs or logs.
