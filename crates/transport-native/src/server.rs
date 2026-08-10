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

    /// Persist to `dir` as `relay.cert.der` (public) and `relay.key.der`
    /// (PKCS8, mode 0600). Restarting the relay against the same
    /// directory then keeps the same certificate — and therefore the same
    /// `cert_sha256_hex` pin already handed out in signed relay bundles —
    /// instead of invalidating every bundle issued before the restart.
    #[cfg(unix)]
    pub fn save_to_dir(&self, dir: &std::path::Path) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("relay.cert.der"), self.cert_der.as_ref())?;
        let mut key_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dir.join("relay.key.der"))?;
        key_file.write_all(self.key_der.secret_der())?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn save_to_dir(&self, dir: &std::path::Path) -> io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("relay.cert.der"), self.cert_der.as_ref())?;
        let mut key_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("relay.key.der"))?;
        key_file.write_all(self.key_der.secret_der())
    }

    /// Load a previously-persisted identity, if `dir` contains one.
    /// Returns `Ok(None)` (not an error) when the directory has no
    /// identity yet, so callers can fall back to `generate` + `save_to_dir`
    /// on first boot.
    #[cfg(unix)]
    pub fn load_from_dir(dir: &std::path::Path) -> io::Result<Option<Self>> {
        use std::os::unix::fs::PermissionsExt;
        let cert_path = dir.join("relay.cert.der");
        let key_path = dir.join("relay.key.der");
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        let mode = std::fs::metadata(&key_path)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(io::Error::other(format!(
                "{key_path:?} has mode {mode:o}, expected 0600 or stricter"
            )));
        }
        let cert_bytes = std::fs::read(&cert_path)?;
        let key_bytes = std::fs::read(&key_path)?;
        let cert_der = CertificateDer::from(cert_bytes);
        let cert_sha256 = crate::cert::sha256_of_cert(cert_der.as_ref());
        let key_der = PrivateKeyDer::Pkcs8(key_bytes.into());
        Ok(Some(Self {
            cert_der,
            key_der,
            cert_sha256,
        }))
    }

    #[cfg(not(unix))]
    pub fn load_from_dir(dir: &std::path::Path) -> io::Result<Option<Self>> {
        let cert_path = dir.join("relay.cert.der");
        let key_path = dir.join("relay.key.der");
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        let cert_der = CertificateDer::from(std::fs::read(cert_path)?);
        let cert_sha256 = crate::cert::sha256_of_cert(cert_der.as_ref());
        let key_der = PrivateKeyDer::Pkcs8(std::fs::read(key_path)?.into());
        Ok(Some(Self {
            cert_der,
            key_der,
            cert_sha256,
        }))
    }

    /// Load a persisted identity from `dir` if present, otherwise
    /// generate a fresh one and persist it for next time.
    #[cfg(unix)]
    pub fn load_or_generate(dir: &std::path::Path, subject_alt_name: &str) -> io::Result<Self> {
        if let Some(identity) = Self::load_from_dir(dir)? {
            return Ok(identity);
        }
        let identity = Self::generate(subject_alt_name);
        identity.save_to_dir(dir)?;
        Ok(identity)
    }

    #[cfg(not(unix))]
    pub fn load_or_generate(dir: &std::path::Path, subject_alt_name: &str) -> io::Result<Self> {
        if let Some(identity) = Self::load_from_dir(dir)? {
            return Ok(identity);
        }
        let identity = Self::generate(subject_alt_name);
        identity.save_to_dir(dir)?;
        Ok(identity)
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
