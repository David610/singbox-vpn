# FAILURE_CLASSIFICATION.md

## Categories (`crates/network-state::FailureCategory`)

```
DnsFailure
TcpTimeout
TcpReset
TlsFailure
QuicFailure
UdpUnavailable
HandshakeTimeout
StallAfterConnect        // connected then no data flow
EndpointUnreachable
ProviderDegradation      // many endpoints on one provider fail together
TransportDegradation     // many endpoints across one transport family fail
GeneralRouteFailure
PossibleShutdown         // nearly all transports/endpoints fail together
LocalNetworkFailure      // no local interface / no default route at all
```

`TransportError` returned by `transport-api::Transport::connect` /
`Session` methods is mapped to exactly one `FailureCategory` by each
transport implementation (`transport_native::direct_tls::classify` /
`::noise_quic::classify`), so the classifier never has to guess from a
generic I/O error string.

## Response depends on category (`crates/failure-classifier`)

| Category | Response |
|---|---|
| `TcpReset`, `TlsFailure`, `QuicFailure`, `UdpUnavailable` | transport-specific → try an independent transport family next |
| `EndpointUnreachable`, `TcpTimeout` (single endpoint) | endpoint-specific → try another endpoint on the *same* transport first |
| `ProviderDegradation` | quarantine all endpoints sharing that provider tag |
| `TransportDegradation` | lower that transport's score broadly, don't retry it for a cooldown window |
| `LocalNetworkFailure` | do not touch remote scoring at all — this is not a censorship signal |
| `PossibleShutdown` (≥ threshold fraction of transports+endpoints failing within a window) | stop rotating; surface "no external path detected" to the user instead of endless cycling (spec §7, §39, §33 invariant) |

## State machine

Implemented as `failure_classifier::ConnectionState`:

```
Idle -> Connecting -> {Connected, Failed(FailureCategory)}
Connected -> {Healthy, Degraded(StallAfterConnect), Closed}
Failed(cat) -> Idle   (after policy decides next endpoint/transport)
Degraded -> {Healthy, Closed}
```

Invariants (property-tested in `crates/failure-classifier/tests`):

1. `PossibleShutdown` classification never triggers another automatic
   transport switch within its cooldown window (no infinite cycling).
2. `EndpointUnreachable` never mutates a transport's score.
3. `TcpReset`/`TlsFailure`/`QuicFailure` never mutate a *different*
   transport's score.
4. `LocalNetworkFailure` never mutates any remote endpoint or transport
   score.
5. State transitions are total: every `(State, Event)` pair has a defined
   next state (no panics on unexpected input) — checked by an exhaustive
   proptest over the event enum.
