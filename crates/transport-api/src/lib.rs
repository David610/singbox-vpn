//! Transport abstraction. `connection-engine`/`policy`/`client-daemon` only
//! depend on this trait, never on a concrete transport — see
//! `docs/TRANSPORT_MODEL.md`.

use async_trait::async_trait;
use common::{EndpointId, TransportId};
use network_state::FailureCategory;
use std::net::SocketAddr;

bitflags::bitflags! {
    /// What a transport (or a specific session) can do. No transport is
    /// required to implement every capability — callers negotiate.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Capabilities: u16 {
        const STREAM         = 0b0000_0001;
        const DATAGRAM       = 0b0000_0010;
        const MIGRATION      = 0b0000_0100;
        const MULTIPLEXING   = 0b0000_1000;
        const ZERO_RTT       = 0b0001_0000;
        const PROXY_CHAINING = 0b0010_0000;
    }
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub id: EndpointId,
    pub address: SocketAddr,
    pub provider_tag: String,
    /// SHA-256 of the relay's certificate DER, as distributed in the
    /// signed rendezvous bundle. Transports that use TLS/QUIC pin against
    /// this instead of trusting a public CA, since these are
    /// operator-run relays, not public web servers.
    pub pinned_cert_sha256: [u8; 32],
}

#[derive(thiserror::Error, Debug)]
pub enum TransportError {
    #[error("connect failed: {category:?}: {detail}")]
    Connect {
        category: FailureCategory,
        detail: String,
    },
    #[error("session closed: {category:?}: {detail}")]
    Session {
        category: FailureCategory,
        detail: String,
    },
}

impl TransportError {
    pub fn category(&self) -> FailureCategory {
        match self {
            TransportError::Connect { category, .. } => *category,
            TransportError::Session { category, .. } => *category,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SessionHealth {
    pub alive: bool,
    pub rtt_estimate_ms: Option<u32>,
}

pub type BoxedReader = Box<dyn tokio::io::AsyncRead + Unpin + Send>;
pub type BoxedWriter = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

#[async_trait]
pub trait Session: Send + Sync {
    async fn send(&mut self, buf: &[u8]) -> Result<(), TransportError>;
    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    async fn health(&self) -> SessionHealth;
    async fn close(&mut self) -> Result<(), TransportError>;
    fn capabilities(&self) -> Capabilities {
        Capabilities::empty()
    }

    /// Split into independent read/write halves for true concurrent
    /// bidirectional relaying. `send`/`recv` above both require `&mut
    /// self`, so driving a full-duplex relay through them needs a lock
    /// held across an in-flight `.await` on one direction while the other
    /// direction is also trying to make progress — a real deadlock risk.
    /// `client-daemon` and `relay-agent` always use `split()`, never
    /// concurrent `send`/`recv`, for exactly this reason.
    fn split(self: Box<Self>) -> (BoxedReader, BoxedWriter);
}

#[async_trait]
pub trait Transport: Send + Sync {
    fn id(&self) -> TransportId;
    fn capabilities(&self) -> Capabilities;
    async fn connect(&self, endpoint: &Endpoint) -> Result<Box<dyn Session>, TransportError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_negotiation_is_bitwise() {
        let stream_only = Capabilities::STREAM;
        let needs = Capabilities::STREAM;
        assert!(stream_only.contains(needs));
        assert!(!stream_only.contains(Capabilities::MIGRATION));
    }

    #[test]
    fn transport_error_category_matches_variant() {
        let e = TransportError::Connect {
            category: FailureCategory::TcpReset,
            detail: "peer reset".into(),
        };
        assert_eq!(e.category(), FailureCategory::TcpReset);
    }
}
