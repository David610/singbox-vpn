# PRIVACY_MODEL.md

## Data minimization commitment

The wire schema for telemetry (`crates/telemetry::Event`) is a closed Rust
enum of bucketed, coarse fields. It is structurally impossible to put a
destination URL, DNS query, packet payload, or long-lived identifier into
it, because no variant has an unconstrained string/byte field — see
`docs/TELEMETRY_DICTIONARY.md` for the exhaustive field list.

## Explicitly not collected (by construction, not policy alone)

visited websites, destination URLs/IPs, plaintext DNS history, packet
payloads/captures, browsing history, long-lived per-user identifiers, raw
source IP beyond what TCP/UDP itself requires transiently for the relay to
function (never persisted by `relay-agent`; it forwards bytes and does not
log peer IPs at info level, only at debug for local troubleshooting, and
never to the telemetry pipeline).

## Session identifiers

`crates/telemetry` uses a random per-connection-attempt correlation id that
is not derived from any stable user identifier and is not persisted beyond
the process lifetime in this slice (no aggregation service exists yet —
Phase 7 in TASKS.md). When the aggregation service is built, this id must
remain unlinkable across sessions; that requirement is recorded here so it
isn't lost.

## DNS

`docs/DEPLOYMENT.md` and the client daemon's routing policy document that
DNS resolution for tunneled destinations must go through the tunnel, never
the local resolver, once the TUN/routing integration (Phase 9, deferred)
exists. In the current local-proxy vertical slice, the client explicitly
receives `host:port` from the caller (SOCKS-style) rather than resolving
locally, so there is no local-resolver leak path in what's implemented —
see `apps/client-daemon` proxy handler.
