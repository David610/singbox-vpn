# ADR-0005: Telemetry is a closed, coarse enum — no free-form fields

## Context
Measurement is necessary to improve routing decisions, but the spec is
explicit that measurement must not become surveillance (§23, design
principle #7).

## Decision
`crates/telemetry::Event` is a Rust enum whose variants only carry closed
enums (`TransportId`, `FailureCategory`) and bucketed numerics (duration
buckets, not raw millisecond timestamps tied to wall-clock; loss/latency
buckets, not raw samples). There is no variant with a `String` or `Vec<u8>`
payload field. This makes "can this carry a destination URL" a type-system
question with an obviously-no answer, not a policy promise that could be
violated by a future careless call site.

## Alternatives considered
- JSON blob event with a schema enforced by convention/docs only: rejected
  — nothing stops a future contributor from adding a `url` field under
  time pressure; the closed-enum approach makes that a compile error in
  the type itself (an aggregation service would still need its own schema
  discipline, noted for Phase 7 follow-up).

## Consequences
Any genuinely new telemetry need requires an explicit new enum variant
(and therefore an explicit, reviewable PR diff, and an update to
TELEMETRY_DICTIONARY.md) rather than silently piggybacking on an existing
free-form field.
