# ADR-0001: Language choice — Rust for client and relay components

## Context
Client and relay code sit on a security boundary (untrusted network input,
key material, process trust). Memory-safety bugs in this position are
directly exploitable (RCE, info leak).

## Decision
Rust for all client-core, transport, relay, and policy code. Services
(rendezvous, relay-agent) are also Rust for a single toolchain and shared
crates (`config`, `crypto`, `transport-api`) between client and server.

## Alternatives considered
- Go: simpler concurrency model, but GC pauses affect latency-sensitive
  transport switching and its type system is weaker for encoding the
  capability/state-machine invariants we want the compiler to enforce.
- C/C++: rejected outright — the whole point is removing memory-safety
  attack surface from a security-critical, network-facing component.

## Consequences
Slower initial development than Go for the services; async ecosystem
(tokio) adds complexity. Cross-platform mobile targets (Phase "future")
will need `uniffi` or similar FFI bridging — noted as future work, not
solved here.
