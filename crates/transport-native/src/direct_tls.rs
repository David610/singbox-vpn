//! Transport family A: TCP + TLS 1.3 (rustls). Standards-faithful stream
//! transport — see `docs/TRANSPORT_MODEL.md`.

use crate::cert::PinnedCertVerifier;
use async_trait::async_trait;
use common::TransportId;
use network_state::FailureCategory;
use rustls::pki_types::ServerName;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use transport_api::{
    BoxedReader, BoxedWriter, Capabilities, Endpoint, Session, SessionHealth, Transport,
    TransportError,
};

pub const ID: TransportId = TransportId::new("direct-tls");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

pub struct DirectTlsTransport;

impl Default for DirectTlsTransport {
    fn default() -> Self {
        Self
    }
}

fn classify_connect_io_error(e: &io::Error) -> FailureCategory {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => FailureCategory::EndpointUnreachable,
        io::ErrorKind::TimedOut => FailureCategory::TcpTimeout,
        io::ErrorKind::ConnectionReset => FailureCategory::TcpReset,
        // LOCAL conditions — the client has no usable network path at all
        // (Wi-Fi off, airplane mode, no route, no address yet). These say
        // nothing about the endpoint or the transport, and attributing them
        // to the remote is actively harmful: `GeneralRouteFailure`
        // `is_remote_signal()`s, so a few seconds of local downtime used to
        // tank every endpoint's score, quarantine every candidate, and feed
        // the shutdown guard. `LocalNetworkFailure` is excluded from scoring
        // by design (docs/FAILURE_CLASSIFICATION.md invariant #4) — but
        // nothing in production ever constructed it, so that invariant was
        // being enforced only on a variant that could not occur.
        io::ErrorKind::NetworkUnreachable
        | io::ErrorKind::HostUnreachable
        | io::ErrorKind::NetworkDown
        | io::ErrorKind::AddrNotAvailable => FailureCategory::LocalNetworkFailure,
        _ => FailureCategory::GeneralRouteFailure,
    }
}

#[async_trait]
impl Transport for DirectTlsTransport {
    fn id(&self) -> TransportId {
        ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::STREAM
    }

    async fn connect(&self, endpoint: &Endpoint) -> Result<Box<dyn Session>, TransportError> {
        let stream = connect_stream(endpoint).await?;
        Ok(Box::new(DirectTlsSession { stream }))
    }
}

/// Lower-level connect that returns the raw `TlsStream` instead of a boxed
/// `Session`. `services/relay-agent` uses this directly for the
/// ingress->egress hop so it can drive the stream with
/// `tokio::io::copy_bidirectional` instead of the `Session` trait's
/// serialized send/recv (which would require holding a lock across both
/// directions and can deadlock a true bidirectional relay — see the
/// module doc comment in `relay-agent`).
pub async fn connect_stream(endpoint: &Endpoint) -> Result<TlsStream<TcpStream>, TransportError> {
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(endpoint.address))
        .await
        .map_err(|_| TransportError::Connect {
            category: FailureCategory::TcpTimeout,
            detail: "tcp connect timed out".into(),
        })?
        .map_err(|e| TransportError::Connect {
            category: classify_connect_io_error(&e),
            detail: e.to_string(),
        })?;

    let verifier = Arc::new(PinnedCertVerifier::new(endpoint.pinned_cert_sha256));
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = ServerName::IpAddress(endpoint.address.ip().into());

    timeout(CONNECT_TIMEOUT, connector.connect(server_name, tcp))
        .await
        .map_err(|_| TransportError::Connect {
            category: FailureCategory::HandshakeTimeout,
            detail: "tls handshake timed out".into(),
        })?
        .map_err(|e| TransportError::Connect {
            category: FailureCategory::TlsFailure,
            detail: e.to_string(),
        })
}

pub struct DirectTlsSession {
    stream: TlsStream<TcpStream>,
}

#[async_trait]
impl Session for DirectTlsSession {
    async fn send(&mut self, buf: &[u8]) -> Result<(), TransportError> {
        self.stream
            .write_all(buf)
            .await
            .map_err(|e| TransportError::Session {
                category: classify_connect_io_error(&e),
                detail: e.to_string(),
            })
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.stream
            .read(buf)
            .await
            .map_err(|e| TransportError::Session {
                category: classify_connect_io_error(&e),
                detail: e.to_string(),
            })
    }

    async fn health(&self) -> SessionHealth {
        SessionHealth {
            alive: true,
            rtt_estimate_ms: None,
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.stream
            .shutdown()
            .await
            .map_err(|e| TransportError::Session {
                category: FailureCategory::GeneralRouteFailure,
                detail: e.to_string(),
            })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::STREAM
    }

    fn split(self: Box<Self>) -> (BoxedReader, BoxedWriter) {
        let (r, w) = tokio::io::split(self.stream);
        (Box::new(r), Box::new(w))
    }
}

#[cfg(test)]
mod classify_tests {
    use super::*;

    /// Regression guard: these are LOCAL failures. If they are reported as
    /// remote signals, an ordinary Wi-Fi drop poisons every endpoint's score
    /// and quarantines transports that are perfectly healthy.
    #[test]
    fn local_network_conditions_are_not_blamed_on_the_endpoint() {
        for kind in [
            io::ErrorKind::NetworkUnreachable,
            io::ErrorKind::HostUnreachable,
            io::ErrorKind::NetworkDown,
            io::ErrorKind::AddrNotAvailable,
        ] {
            let category = classify_connect_io_error(&io::Error::new(kind, "local"));
            assert_eq!(
                category,
                FailureCategory::LocalNetworkFailure,
                "{kind:?} must classify as a local failure"
            );
            assert!(
                !category.is_remote_signal(),
                "{kind:?} must not be treated as evidence about the remote endpoint"
            );
        }
    }

    /// ...while genuinely remote conditions must still be attributed to the
    /// endpoint, or the policy engine stops learning anything at all.
    #[test]
    fn remote_conditions_are_still_remote_signals() {
        for (kind, expected) in [
            (
                io::ErrorKind::ConnectionRefused,
                FailureCategory::EndpointUnreachable,
            ),
            (io::ErrorKind::TimedOut, FailureCategory::TcpTimeout),
            (io::ErrorKind::ConnectionReset, FailureCategory::TcpReset),
        ] {
            let category = classify_connect_io_error(&io::Error::new(kind, "remote"));
            assert_eq!(category, expected);
            assert!(category.is_remote_signal(), "{kind:?} is a remote signal");
        }
    }
}
