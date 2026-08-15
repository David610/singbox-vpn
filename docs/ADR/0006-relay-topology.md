# ADR-0006: Configurable ingress/egress topology, not forced 3-hop

## Context
Spec §15 explicitly warns against forcing three hops for every connection
(latency, timing-correlation surface) while still wanting an optional
separation between "receives client transport" and "dials the Internet".

## Decision
`relay-agent` takes a `role` config: `combined` (one process is both
ingress and egress) or `ingress`/`egress` (two processes, a second
`direct-tls` hop between them). Client routing policy picks which relay
bundle (combined vs. split) to use; nothing in the client or relay code
hardcodes hop count.

## Alternatives considered
- Always 2-hop: rejected per spec §15 (unnecessary latency/timing surface
  for users who don't need operator-separation).
- Always 1-hop: rejected as the only option — removes the operator-trust
  diversification some users need (THREAT_MODEL.md malicious-relay case).

## Consequences
Two topologies must be tested (both are, in `tests/tests/e2e.rs` and
`tests/tests/failure_independence.rs`). Cross-client aggregation/global relay
health policy (deferred, Phase 7) will need to be topology-aware later.
