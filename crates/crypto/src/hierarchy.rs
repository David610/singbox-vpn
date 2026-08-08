//! Three-tier signing hierarchy (ADR-0008):
//!
//! ```text
//! offline root key -> release signing key -> bundle signing key
//! ```
//!
//! A `KeyCertificate` binds a child public key to a parent's signature so
//! verification can walk the chain up to a hardcoded root fingerprint
//! without the online service ever holding the root or release key.

use crate::{verify, CryptoError, KeyPair, PublicKey};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct KeyCertificate {
    pub child_key: PublicKey,
    pub issued_at: u64,
    /// Signature over `(child_key || issued_at)` by the parent key.
    #[serde(with = "crate::fixed_bytes")]
    pub signature: [u8; 64],
}

impl KeyCertificate {
    pub fn issue(parent: &KeyPair, child_key: PublicKey, issued_at: u64) -> Self {
        let signature = parent.sign(&Self::signing_bytes(&child_key, issued_at));
        Self {
            child_key,
            issued_at,
            signature,
        }
    }

    fn signing_bytes(child_key: &PublicKey, issued_at: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(40);
        buf.extend_from_slice(&child_key.0);
        buf.extend_from_slice(&issued_at.to_be_bytes());
        buf
    }

    pub fn verify_against(&self, parent_public: &PublicKey) -> Result<(), CryptoError> {
        verify(
            parent_public,
            &Self::signing_bytes(&self.child_key, self.issued_at),
            &self.signature,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_chain_verifies() {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let cert = KeyCertificate::issue(&root, release.public_key(), 1);
        assert!(cert.verify_against(&root.public_key()).is_ok());
    }

    #[test]
    fn certificate_rejects_wrong_root() {
        let root = KeyPair::generate();
        let other_root = KeyPair::generate();
        let release = KeyPair::generate();
        let cert = KeyCertificate::issue(&root, release.public_key(), 1);
        assert!(cert.verify_against(&other_root.public_key()).is_err());
    }
}
