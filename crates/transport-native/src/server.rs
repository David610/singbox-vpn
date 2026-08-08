//! Server-side helpers shared by `services/relay-agent` to accept both
//! transport families on a pinned, self-signed relay certificate.

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct RelayIdentity {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
    pub cert_sha256: [u8; 32],
}

impl RelayIdentity {
    pub fn generate(subject_alt_name: &str) -> Self {
        let (cert, key_pair) = crate::cert::generate_self_signed(subject_alt_name);
        let cert_der = cert.der().clone();
        let cert_sha256 = crate::cert::sha256_of_cert(cert_der.as_ref());
        let key_der = PrivateKeyDer::Pkcs8(key_pair.serialize_der().into());
        Self {
            cert_der,
            key_der,
            cert_sha256,
        }
    }
}

pub fn tls_server_config(identity: &RelayIdentity) -> Arc<rustls::ServerConfig> {
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.cert_der.clone()],
            identity.key_der.clone_key(),
        )
        .expect("valid relay certificate/key");
    Arc::new(config)
}

/// Combines a QUIC `SendStream`/`RecvStream` pair into one
/// `AsyncRead + AsyncWrite` value so it can be driven with
/// `tokio::io::copy_bidirectional` like any other duplex stream.
pub struct QuicBiStream {
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
}

impl AsyncRead for QuicBiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for QuicBiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

pub fn quic_server_config(identity: &RelayIdentity) -> quinn::ServerConfig {
    let tls = tls_server_config(identity);
    let mut tls = (*tls).clone();
    tls.max_early_data_size = u32::MAX;
    let quic_tls: quinn::crypto::rustls::QuicServerConfig =
        tls.try_into().expect("quic-compatible tls server config");
    quinn::ServerConfig::with_crypto(Arc::new(quic_tls))
}
