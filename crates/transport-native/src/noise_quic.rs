//! Transport family B: QUIC (quinn) — UDP + QUIC-TLS. Independent failure
//! mode from `direct_tls`: blocked by UDP filtering / QUIC-specific
//! blocking, unaffected by TCP RST injection. See
//! `docs/TRANSPORT_MODEL.md`.

use crate::cert::PinnedCertVerifier;
use async_trait::async_trait;
use common::TransportId;
use network_state::FailureCategory;
use quinn::crypto::rustls::QuicClientConfig;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::timeout;
use transport_api::{
    BoxedReader, BoxedWriter, Capabilities, Endpoint, Session, SessionHealth, Transport,
    TransportError,
};

pub const ID: TransportId = TransportId::new("noise-quic");
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

pub struct NoiseQuicTransport;

impl Default for NoiseQuicTransport {
    fn default() -> Self {
        Self
    }
}

fn client_endpoint(bind: SocketAddr) -> Result<quinn::Endpoint, TransportError> {
    quinn::Endpoint::client(bind).map_err(|e| TransportError::Connect {
        category: FailureCategory::UdpUnavailable,
        detail: format!("failed to bind local UDP socket: {e}"),
    })
}

#[async_trait]
impl Transport for NoiseQuicTransport {
    fn id(&self) -> TransportId {
        ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::STREAM | Capabilities::DATAGRAM
    }

    async fn connect(&self, endpoint: &Endpoint) -> Result<Box<dyn Session>, TransportError> {
        let bind_addr: SocketAddr = if endpoint.address.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };
        let ep = client_endpoint(bind_addr)?;

        let verifier = Arc::new(PinnedCertVerifier::new(endpoint.pinned_cert_sha256));
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let quic_client_config: QuicClientConfig =
            tls_config.try_into().map_err(|e| TransportError::Connect {
                category: FailureCategory::QuicFailure,
                detail: format!("invalid quic tls config: {e}"),
            })?;
        let client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));

        let connecting = ep
            .connect_with(client_config, endpoint.address, "relay")
            .map_err(|e| TransportError::Connect {
                category: FailureCategory::QuicFailure,
                detail: e.to_string(),
            })?;

        let connection = timeout(CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| TransportError::Connect {
                category: FailureCategory::HandshakeTimeout,
                detail: "quic handshake timed out".into(),
            })?
            .map_err(|e| TransportError::Connect {
                category: classify_connection_error(&e),
                detail: e.to_string(),
            })?;

        let (send, recv) = connection
            .open_bi()
            .await
            .map_err(|e| TransportError::Connect {
                category: FailureCategory::QuicFailure,
                detail: e.to_string(),
            })?;

        Ok(Box::new(NoiseQuicSession {
            _endpoint: ep,
            connection,
            send,
            recv,
        }))
    }
}

fn classify_connection_error(e: &quinn::ConnectionError) -> FailureCategory {
    match e {
        quinn::ConnectionError::TimedOut => FailureCategory::HandshakeTimeout,
        quinn::ConnectionError::ConnectionClosed(_)
        | quinn::ConnectionError::ApplicationClosed(_) => FailureCategory::EndpointUnreachable,
        _ => FailureCategory::QuicFailure,
    }
}

pub struct NoiseQuicSession {
    _endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

#[async_trait]
impl Session for NoiseQuicSession {
    async fn send(&mut self, buf: &[u8]) -> Result<(), TransportError> {
        self.send
            .write_all(buf)
            .await
            .map_err(|e| TransportError::Session {
                category: FailureCategory::QuicFailure,
                detail: e.to_string(),
            })
    }

    async fn recv(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.recv
            .read(buf)
            .await
            .map_err(|e| TransportError::Session {
                category: FailureCategory::QuicFailure,
                detail: e.to_string(),
            })?
            .ok_or_else(|| TransportError::Session {
                category: FailureCategory::EndpointUnreachable,
                detail: "quic stream closed".into(),
            })
    }

    async fn health(&self) -> SessionHealth {
        let stats = self.connection.stats();
        SessionHealth {
            alive: self.connection.close_reason().is_none(),
            rtt_estimate_ms: Some(stats.path.rtt.as_millis() as u32),
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.connection.close(0u32.into(), b"close");
        Ok(())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::STREAM | Capabilities::DATAGRAM
    }

    fn split(self: Box<Self>) -> (BoxedReader, BoxedWriter) {
        // `connection`/`_endpoint` are cheap-clone handles (quinn keeps
        // the real state in an Arc internally); cloning them into both
        // halves keeps the underlying QUIC connection alive even though
        // this `Session` object itself is being consumed.
        let keepalive_r = (self.connection.clone(), self._endpoint.clone());
        let keepalive_w = (self.connection, self._endpoint);
        let reader = QuicReadHalf {
            recv: self.recv,
            _keepalive: keepalive_r,
        };
        let writer = QuicWriteHalf {
            send: self.send,
            _keepalive: keepalive_w,
        };
        (Box::new(reader), Box::new(writer))
    }
}

struct QuicReadHalf {
    recv: quinn::RecvStream,
    _keepalive: (quinn::Connection, quinn::Endpoint),
}

impl AsyncRead for QuicReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

struct QuicWriteHalf {
    send: quinn::SendStream,
    _keepalive: (quinn::Connection, quinn::Endpoint),
}

impl AsyncWrite for QuicWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}
