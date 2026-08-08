# PLAN.md — Adaptive Censorship-Resistant Networking Platform

## Scope reality check

The originating spec describes a multi-year, multi-team production platform
(cross-platform clients, WASM transport sandboxing, global rendezvous
infrastructure, mobile support, full measurement pipeline, etc). A single
engineering session cannot deliver that responsibly without faking large
parts of it. This plan commits to building a **smaller number of things for
real** rather than a large surface of stubs:

- A real Rust workspace with the core abstractions the whole system depends
  on: config signing/verification, transport capability model, connection
  state machine, failure classification, endpoint/transport confidence
  scoring.
- A real local vertical slice: client → ingress relay → egress relay →
  local test HTTP service, over an actually-encrypted transport, with
  traffic flowing end to end and integration-tested.
- Two genuinely independent transport families (not two variations of the
  same thing), so failure-independence is real and testable:
  - **Transport A ("direct-tls")**: TLS 1.3 stream transport (rustls),
    standards-faithful, looks like ordinary HTTPS at the TCP/TLS layer.
  - **Transport B ("noise-quic")**: QUIC-based datagram-oriented transport
    (quinn + self-signed cert, independent handshake path from A).
- A signed rendezvous service that hands out a small, expiring, signed
  subset of relay endpoints rather than a full server list.
- Adaptive connection engine: scoring, quarantine, non-deterministic
  fallback, driven by a documented state machine with property/unit tests.
- A diagnostic CLI.
- Hostile-network integration tests using Linux network namespaces + `tc
  netem` (packet loss, latency, blocked port) to prove failure
  independence between transports and endpoints.

Explicitly deferred (documented, not faked): WASM transport sandbox runtime,
mobile/Windows/macOS platform adapters, full measurement/telemetry
pipeline & aggregation service, control-plane release/signing
infrastructure beyond a single offline+online key pair, desktop GUI,
Tor Snowflake/obfs4/REALITY adapters. These are listed as `TASKS.md`
`[!] blocked` / `not started` items with rationale, not implemented as
fake stubs that return `true`.

## Order of work

1. Docs: architecture, threat model, transport model, rendezvous design,
   security/privacy model, failure classification, decision engine,
   deployment, test strategy, ADRs. (this phase)
2. Workspace skeleton + `common`, `crypto`, `config` crates with signed
   config bundle verification, fully tested.
3. `transport-api` capability model + `network-state`/`failure-classifier`
   + connection state machine, property-tested.
4. `connection-engine` scoring/quarantine/fallback policy.
5. `transport-native`: direct-tls and noise-quic implementations.
6. `services/rendezvous`: signed relay-subset issuance.
7. `services/relay-agent`: ingress/egress forwarding.
8. `apps/client-daemon` + `apps/cli`: wire everything together, TUN-free
   local vertical slice first (loopback proxy), so the slice runs without
   root/network namespace requirements; Linux TUN + kill switch as a
   documented follow-on behind a feature flag.
9. Integration tests: local slice, `tc netem` hostile-network tests,
   failure-independence tests (transport blocked / endpoint blocked).
10. Fuzz targets for config + rendezvous response parsing.
11. Final security self-review, report.

## Non-goals for this session

No public deployment, no purchased infra, no push beyond the assigned
branch, no GUI, no mobile/Windows/macOS implementation.
