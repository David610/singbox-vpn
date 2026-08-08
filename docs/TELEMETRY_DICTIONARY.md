# TELEMETRY_DICTIONARY.md

Wire type: `crates/telemetry::Event` (closed enum). Every field of every
variant listed exhaustively:

| Variant | Fields | Why it exists |
|---|---|---|
| `ConnectionAttempt` | `transport: TransportId`, `endpoint_tag: EndpointTag` (opaque provider/AS/geo tag, not an IP/hostname) | lets the (future) aggregation service see which transport/tag combos are being tried |
| `ConnectionResult` | `transport: TransportId`, `endpoint_tag: EndpointTag`, `outcome: Outcome` (`Success` \| `Failure(FailureCategory)`), `handshake_ms_bucket: DurationBucket` | success/failure rate and coarse handshake latency per transport/tag |
| `SessionEnded` | `transport: TransportId`, `duration_bucket: DurationBucket`, `reason: SessionEndReason` (`UserDisconnect` \| `Stall` \| `Reset` \| `Migration`) | session survival signal for scoring |
| `StallObserved` | `transport: TransportId`, `endpoint_tag: EndpointTag` | feeds `StallAfterConnect` classification stats |
| `NetworkConditionBucket` | `loss_bucket: LossBucket`, `latency_bucket: LatencyBucket`, `time_bucket: TimeBucket` (hour-of-day, coarse) | coarse network-quality signal, no raw samples, no fine timestamp |

`DurationBucket`, `LossBucket`, `LatencyBucket`, `TimeBucket`,
`EndpointTag` are all closed enums/newtypes over small integers — see
`crates/telemetry/src/lib.rs` doc comments for exact bucket boundaries.

## Explicitly absent (no field exists for any of these, by type)
destination hostname/IP, URL, DNS query, packet payload, raw source IP,
device/account identifier, precise wall-clock timestamp, free-form string
of any kind.

## Current status
No aggregation/collection service consumes these events yet (Phase 7,
deferred) — the enum and its emission points exist and are unit-tested for
"cannot be constructed with disallowed data" (a compile-time property,
demonstrated by a doctest showing there is no constructor taking a
string), but nothing is transmitted off the client host today.
