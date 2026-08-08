# ADR-0004: Rendezvous returns signed, bounded, expiring subsets

## Context
A naive `GET /servers.json` lets any party (censor included) enumerate the
entire relay fleet in one request, and a static list has no revocation or
freshness story.

## Decision
`services/rendezvous` samples a bounded (default 5) random subset from its
relay pool per request, wraps it in a `RelayBundle` signed by a bundle
signing key (itself certified by an offline-adjacent release key), with a
15-minute default expiry. See RENDEZVOUS_DESIGN.md for the full flow.

## Alternatives considered
- Static signed file served over CDN: simpler, but no per-request
  diversity/subsetting — full-fleet enumeration is trivial by definition.
- Per-client persistent relay assignment: better cache-ability, but creates
  a stable identifier linking a client to specific relays over time
  (privacy/fingerprinting regression) — rejected.

## Consequences
Clients must re-fetch periodically (every ~15 min while connected, or on
demand when all cached endpoints fail) rather than caching forever; the
`rendezvous-client` crate implements the cached "emergency bundle" fallback
specifically to avoid this becoming a single point of failure per
RENDEZVOUS_DESIGN.md.
