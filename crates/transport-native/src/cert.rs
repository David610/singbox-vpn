//! Certificate pinning shared by both transports. Relays are operator-run,
//! not public web servers, so we pin the exact certificate distributed in
//! the signed rendezvous bundle rather than trusting a public CA — this
//! avoids depending on the public WebPKI (and its own set of trust/
//! revocation issues) for a private relay fleet. The handshake signature
//! itself is still fully verified (not skipped) — only the "is this cert
//! in a CA chain" check is replaced with "is this the exact pinned cert".

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use sha2::{Digest, Sha256};

pub fn sha256_of_cert(der: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(der);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[derive(Debug)]
pub struct PinnedCertVerifier {
    expected_sha256: [u8; 32],
    supported_algs: WebPkiSupportedAlgorithms,
}

impl PinnedCertVerifier {
    pub fn new(expected_sha256: [u8; 32]) -> Self {
        let provider = rustls::crypto::ring::default_provider();
        Self {
            expected_sha256,
            supported_algs: provider.signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let digest = sha256_of_cert(end_entity.as_ref());
        if digest == self.expected_sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General("certificate pin mismatch".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

/// Generates a fresh self-signed keypair+certificate for a relay to serve.
/// Dev/local-slice tooling only — a real deployment provisions relay
/// certificates out of band and distributes the pin via the signed
/// rendezvous bundle (see RENDEZVOUS_DESIGN.md); this function is what
/// `deploy/local/gen-keys.sh`-equivalent test fixtures call.
pub fn generate_self_signed(subject_alt_name: &str) -> (rcgen::Certificate, rcgen::KeyPair) {
    let key_pair = rcgen::KeyPair::generate().expect("keygen");
    let params = rcgen::CertificateParams::new(vec![subject_alt_name.to_string()]).expect("params");
    let cert = params.self_signed(&key_pair).expect("self-sign");
    (cert, key_pair)
}
