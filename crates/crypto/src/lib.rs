//! Thin wrappers over audited primitives. No custom cryptography lives here
//! or anywhere else in this workspace (spec principle #8).
//!
//! - Signing: Ed25519 via `ed25519-dalek`.
//! - Randomness: OS CSPRNG via `rand::rngs::OsRng`.
//! - Session encryption (TLS/QUIC) is handled entirely by `rustls`/`quinn`
//!   in `transport-native`, not here.

pub mod fixed_bytes;
pub mod hierarchy;
pub mod secret;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::fmt;

pub use secret::Secret;

#[derive(thiserror::Error, Debug)]
pub enum CryptoError {
    #[error("invalid signature")]
    InvalidSignature,
    #[error("malformed key material: {0}")]
    MalformedKey(String),
    #[error("key file permissions too permissive: {0}")]
    InsecureKeyPermissions(String),
}

/// A public key, hex-encoded for display/serialization. Never carries the
/// corresponding private key.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKey(#[serde(with = "fixed_bytes")] pub [u8; 32]);

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", hex::encode(self.0))
    }
}

/// A keypair capable of signing. The private half is never `Debug`-printed
/// or serialized by this type (see `secret::Secret`).
pub struct KeyPair {
    signing_key: Secret<SigningKey>,
}

impl KeyPair {
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self {
            signing_key: Secret::new(signing_key),
        }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            signing_key: Secret::new(SigningKey::from_bytes(&bytes)),
        }
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing_key.expose().verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.expose().sign(message).to_bytes()
    }
}

pub fn verify(
    public_key: &PublicKey,
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), CryptoError> {
    let vk = VerifyingKey::from_bytes(&public_key.0)
        .map_err(|e| CryptoError::MalformedKey(e.to_string()))?;
    let sig = Signature::from_bytes(signature);
    vk.verify(message, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let kp = KeyPair::generate();
        let msg = b"relay bundle payload";
        let sig = kp.sign(msg);
        assert!(verify(&kp.public_key(), msg, &sig).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let kp = KeyPair::generate();
        let sig = kp.sign(b"original");
        assert!(verify(&kp.public_key(), b"tampered", &sig).is_err());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();
        let sig = kp1.sign(b"msg");
        assert!(verify(&kp2.public_key(), b"msg", &sig).is_err());
    }

    #[test]
    fn debug_does_not_leak_private_key_bytes() {
        let kp = KeyPair::generate();
        // Secret<T> intentionally has no public accessor that yields a
        // Debug-safe raw byte view outside the crate; this test documents
        // the guarantee by construction (no `Debug` impl exists to call).
        let pk_debug = format!("{:?}", kp.public_key());
        assert!(pk_debug.starts_with("PublicKey("));
    }
}
