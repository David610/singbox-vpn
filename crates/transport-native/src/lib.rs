//! Concrete transport implementations. Only `apps/client-daemon` and
//! `services/relay-agent` depend on this crate directly — everything else
//! in the workspace depends only on `transport-api`.

pub mod cert;
pub mod direct_tls;
pub mod noise_quic;
pub mod server;

pub use direct_tls::DirectTlsTransport;
pub use noise_quic::NoiseQuicTransport;
