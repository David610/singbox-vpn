# TRANSPORT_MODEL.md

## Interface

`crates/transport-api` defines:

```rust
#[async_trait]
pub trait Transport: Send + Sync {
    fn id(&self) -> TransportId;
    fn capabilities(&self) -> Capabilities;
    async fn connect(&self, endpoint: &Endpoint) -> Result<Box<dyn Session>, TransportError>;
}

#[async_trait]
pub trait Session: Send + Sync {
    async fn send(&mut self, buf: &[u8]) -> Result<(), TransportError>;
    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    async fn health(&self) -> SessionHealth;
    async fn close(&mut self) -> Result<(), TransportError>;
    fn capabilities(&self) -> Capabilities { Capabilities::empty() } // e.g. migration
}
```

Capabilities are a bitflag set (`STREAM`, `DATAGRAM`, `MIGRATION`,
`MULTIPLEXING`, `ZERO_RTT`, `PROXY_CHAINING`). No transport is required to
implement all of them; `connection-engine` negotiates based on what routing
policy actually needs (e.g. "needs a reliable byte stream" is satisfiable by
either transport here since both are wrapped to present a stream API even
though transport B is datagram-oriented at the QUIC layer — see
`transport-native::noise_quic` which opens a QUIC bidirectional stream, so
higher layers see one uniform `Session` shape today; native datagram mode is
exposed via `capabilities()` for future direct use).

## Implemented transport families (this session)

| Family | Crate module | Layer 4/5 | Independent failure mode vs. the other |
|---|---|---|---|
| A: `direct-tls` | `transport_native::direct_tls` | TCP + TLS 1.3 (rustls, tokio) | Blocked by TCP resets, SNI blocking, TLS fingerprinting |
| B: `noise-quic` | `transport_native::noise_quic` | UDP + QUIC (quinn, self-signed cert pinned via config) | Blocked by UDP filtering, QUIC-specific blocking; unaffected by TCP RST injection |

These satisfy the spec's failure-independence requirement: a censor that
blocks UDP entirely still allows A through; a censor that resets TCP
connections to specific fingerprinted TLS endpoints still allows B through.
Both use well-reviewed libraries (rustls, quinn) — no custom crypto or
custom TLS/QUIC state machine.

## Deferred families (documented, not built)

- **Family C (ephemeral/volunteer relay, e.g. Snowflake-style)** and
  **Family D (probe-resistant authenticated transport, e.g.
  obfs4/REALITY-style)** — evaluated in ADR-0002. Both require either a
  WebRTC/ICE stack or a maintained external Rust crate with a compatible
  license and API stability; neither was integrated this session to avoid
  a half-working, unaudited adapter. The `Transport` trait above is
  designed so either can be added as a new `transport-native` (or
  `transport-runtime`, once sandboxing exists) module without touching
  `connection-engine`.

## Bootstrap vs. steady state

`connect()` performs the full handshake (bootstrap path). The returned
`Session` is the steady-state path. Neither transport implements migration
in this slice (both are single-path); `Capabilities::MIGRATION` exists in
the enum precisely so a future QUIC-connection-migration transport can be
added without changing the trait, per spec §16.
