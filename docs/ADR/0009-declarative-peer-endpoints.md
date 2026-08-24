# ADR-0009: Declarative peer endpoints (deferred; contract-only in this pass)

## Context

The long-term resilience goal for this project is that a small group of
trusted users can keep using the VPN if one server IP, subnet, ASN,
provider, or transport becomes degraded or blocked. That requires the
provisioning document `singbox-client` receives to be able to describe
more than one independently-hosted, independently-keyed trusted
candidate — not just this deployment's own REALITY and Hysteria2
listeners.

This deployment is explicitly single-VPS (`docs/SUPPORTED_PRODUCT.md`):
"Default topology: one VPS. No v1.0 multi-node control plane or fleet
orchestration." That constraint is not relaxed by this ADR. This document
only asks: if an operator later stands up a second, independently-run
VPS by hand, what is the smallest change this server would need to also
tell clients about it?

## What already works, with zero server code change

`provisioning-contract`'s `Endpoint` type has always carried its own
`host`, `port`, and full transport-specific credentials
(`TransportParams::VlessReality { uuid, reality: RealityParams { public_key,
short_id, .. }, .. }` / `TransportParams::Hysteria2 { password, obfs, .. }`)
— nothing ties an endpoint's identity or credentials to a
deployment-global host. `ProvisioningDocument::validate` already enforces
duplicate-id rejection, capability/endpoint coherence, and the forbidden-
content audit per-endpoint, generically.

`fixtures/singbox-client-contract/09-two-independent-endpoints.json`
(added alongside this ADR) proves this directly: two REALITY endpoints on
two different hosts, with completely independent REALITY key pairs and
VLESS UUIDs, validate and round-trip correctly through both this crate
and (via the matching `singbox-client` fixture consumer test) the real
client parser. See `two_independent_endpoints_share_no_credential_or_key_material`
for the code-level answer to "if Endpoint A is compromised, which
credentials usable on Endpoint B become exposed?" — **none**, because
nothing in the type or in that construction ties them together. That
fixture is hand-assembled from `provisioning_contract` types directly
(not through `compat_config::contract`/`standard_endpoints`, both of
which are shaped for exactly one deployment's own two transports) — it is
not producible by any code this server currently runs. That is precisely
the gap this ADR describes.

## What is NOT free: assembling that document from real config

`standard_endpoints(public_host, ...)` and
`compat_config::contract::provisioning_document_with_mode(&user, &endpoints, ...)`
are both single-deployment shaped: one `public_host`, and endpoint
credentials sourced from that one `CompatUser`'s own `vless_uuid`/
`hysteria2_password`. There is currently no config surface an operator
could use to say "also include this other, already-running server's
endpoint in the document I serve."

## Decision: defer implementation; specify the smallest shape here

This PR does not implement a runtime feature for this. Reasons:

1. **No real second VPS exists yet** — implementing and testing a
   feature against a hypothetical deployment risks getting the one part
   that matters (credential scoping) wrong with no way to validate it.
2. **Credential scoping is a real design decision, not just plumbing.**
   See "Credential model" below — it deserves its own review once a
   second VPS is actually being stood up, not a rushed answer bundled
   into a resilience-primitives PR.
3. Per this project's own scope discipline
   (`docs/SUPPORTED_PRODUCT.md`, and the brief this ADR was written
   under): "static/operator-managed trusted endpoint catalog is
   preferable to a control plane for approximately 10 trusted users" —
   the right shape is declarative config, not a service.

When a second VPS is actually deployed, the smallest addition is:

### Config shape (sketch, not implemented)

```toml
# /etc/vpn/deployment.toml — NEW, optional section
[[peer_endpoints]]
id = "eu2-reality"              # must not collide with this server's own ids
tag = "Europe 2"
host = "vpn2.example.net"       # the OTHER server's public host
port = 8443
transport = "vless-reality"
server_name = "www.example-decoy-two.org"
reality_public_key = "..."      # that server's REALITY PUBLIC key only
reality_short_id = "..."
# per-user credential: see "Credential model" below — NOT a single
# shared value here, most likely a small per-user table keyed the same
# way `users.json` already keys this server's own users.
```

### Validation additions

- Reuse `Endpoint::validate()`/`ProvisioningDocument::validate()`
  unchanged — they are already generic over host.
- Add a startup check that a `peer_endpoints` id never collides with an
  id this server generates itself (`standard_endpoints` always emits
  `reality-1`/`hysteria2-1` — a peer id must not reuse those, and
  `ProvisioningDocument::validate`'s existing `DuplicateEndpointId` check
  already catches a collision once both are assembled into one document).
- The peer's REALITY **public** key is not secret and is fine in this
  server's config on disk (same treatment as this server's own public
  key/short_id, already handled by `credentials::validate_reality_public_key_shape`).
  The peer's REALITY **private** key must never appear here — it never
  leaves the peer server, exactly as this server's own private key never
  appears in the client-facing contract today (`FORBIDDEN_CONTENT`
  already enforces this for values that DO end up in the document; the
  private key of a peer server should simply never be typed into this
  server's config in the first place, since this server has no use for
  it).

### Credential model — the actual open question

Two honest options, not yet decided:

- **A. Per-user credential pairs.** Each `CompatUser` gets an additional,
  optional map (or a small side table) of `peer_endpoint_id -> {uuid |
  password}` — the SAME person's credential as provisioned on the peer
  server by its own `vpn-admin`, entered here by the operator (who runs
  both servers) so this server's subscription response can include it.
  Most faithful to "independent endpoint credentials" (mission §10):
  compromising this server's `users.json` exposes this server's own
  credentials plus whatever peer credentials were copied in — not the
  peer server's REALITY private key or its ability to mint new
  credentials.
- **B. Shared credential per peer.** A peer endpoint gets ONE shared
  UUID/password for all users (like a single "team" login on that
  server). Simpler, but breaks per-user revocation (disabling one local
  user does not revoke their access to the peer) and blast radius is
  worse (any local user's device compromise exposes a credential that
  works for every other user on the peer, not just themselves).

**Recommendation when this is actually implemented: Option A.** It is
more code (extending `CompatUser`, `users.json` schema, `vpn-admin`
tooling to accept "paste this peer's per-user credential") but it is the
only option consistent with this project's existing "independent
endpoint credentials" posture (this server already gives each user their
own `vless_uuid`/`hysteria2_password` rather than one shared secret) and
with the mission's explicit preference against unnecessarily widening
blast radius.

## Consequences

- No server code changes ship in this pass; only the contract-level proof
  (fixture 09 + its tests) that the target document shape is already
  valid, and this design record.
- The next PR that wants a real second endpoint declared here should
  start from "Option A" above, and should not proceed without an actual
  second VPS to test against (per this repository's own
  no-speculative-infrastructure discipline).
- `docs/SUPPORTED_PRODUCT.md`'s "single-VPS, no v1.0 multi-node control
  plane" statement is unaffected — this is a static declarative addition
  to ONE server's config, not a control plane, and remains out of scope
  until deliberately taken up.
