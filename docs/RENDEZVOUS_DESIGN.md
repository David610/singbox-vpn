# RENDEZVOUS_DESIGN.md

## Goal

A client must be able to learn a *usable* set of relay endpoints without
any single request revealing the *entire* fleet, and without trusting the
rendezvous service's online key to authorize anything beyond "here is a
subset of endpoints the release key already vouched for."

## Flow

```
client --(HTTPS, rate-limited)--> rendezvous
rendezvous: pick a bounded random subset (default 5) of currently-healthy
            relays from its pool, weighted by AS/geo/provider diversity
rendezvous: wrap subset in a RelayBundle {schema_version, issued_at,
            expires_at (default 15 min), nonce, endpoints[]}
rendezvous: sign RelayBundle with its *bundle signing key* (an
            intermediate key certified by the offline release key —
            see ADR-0008)
rendezvous --(signed bundle)--> client
client: verify signature chain (bundle key cert signed by release key,
        release key's fingerprint pinned in client build), verify
        expires_at > now, verify schema_version supported
client: cache bundle to disk as the "emergency bundle" (last-known-good)
```

## Key properties implemented

- **Bounded subset, not full listing**: `services/rendezvous` never
  serializes its full relay table to any client response; it samples.
- **Short validity**: default 15 minutes (`RelayBundle::expires_at`),
  enforced client-side in `crates/config`; an expired bundle is rejected,
  never "silently remains trusted" (spec §33 invariant, tested).
- **Signed, layered keys**: offline root key → release signing key (signs
  bundle-signing-key certificates) → bundle signing key (signs individual
  `RelayBundle`s, rotated more frequently, and is the only key the
  always-online rendezvous process holds). Compromising the rendezvous
  process yields at most the current bundle signing key, which can be
  revoked via a `revoked_key_ids` list shipped in future release-signed
  config without needing the offline root key to re-sign every relay
  bundle (see ADR-0008 and `crates/config::revocation`).
- **Emergency/cached bundle**: `rendezvous-client` persists the last valid
  bundle and will serve it (still expiry-checked) if the rendezvous
  service is unreachable, satisfying spec §52's "rendezvous temporarily
  unavailable, cached signed recovery information still works" test
  (`tests/tests/failure_independence.rs::rendezvous_outage_uses_cached_bundle`).

## What is explicitly not solved this session

- Multiple independent discovery channels (e.g. domain fronting, out of
  band bundle distribution) — only the direct HTTPS channel is
  implemented; the client interface (`RendezvousSource` trait) allows more
  channels to be added later.
- Cross-client corroboration before a relay is fully retired from the
  rendezvous pool (needs the control-plane aggregation service, deferred —
  see Phase 7/ADR-0006). Today, retiring a relay from the rendezvous pool
  is an operator action (config edit), not automatic.
- Rate limiting is implemented as a simple in-memory token bucket per
  source IP in `services/rendezvous` — adequate for the local test slice,
  explicitly documented in DEPLOYMENT.md as needing a real edge/CDN rate
  limiter in front of any public deployment.
