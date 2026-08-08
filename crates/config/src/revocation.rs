//! Revocation list for keys in the signing hierarchy (ADR-0008). Kept as a
//! simple in-memory set for the actual lookups; `SignedRevocationList` is
//! the on-the-wire/on-disk form, signed by the release key so an attacker
//! who only compromises the always-online rendezvous host cannot un-revoke
//! a key (revoking still doesn't need the offline root — only the release
//! key, matching ADR-0008's rotation story).

use crate::{ConfigError, TrustRoot};
use crypto::hierarchy::KeyCertificate;
use crypto::{verify, KeyPair, PublicKey};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub struct RevocationList {
    revoked: HashSet<[u8; 32]>,
}

impl RevocationList {
    pub fn empty() -> Self {
        Self {
            revoked: HashSet::new(),
        }
    }

    pub fn from_keys(keys: impl IntoIterator<Item = PublicKey>) -> Self {
        Self {
            revoked: keys.into_iter().map(|k| k.0).collect(),
        }
    }

    pub fn revoke(&mut self, key: PublicKey) {
        self.revoked.insert(key.0);
    }

    pub fn contains(&self, key: &PublicKey) -> bool {
        self.revoked.contains(&key.0)
    }
}

impl Default for RevocationList {
    fn default() -> Self {
        Self::empty()
    }
}

/// The persisted, distributable form of a revocation list: signed by a
/// release key whose certificate chains to the pinned root, so it carries
/// its own authenticity independent of the channel that delivered it
/// (rendezvous HTTP response, file copied by an operator, etc).
#[derive(Clone, Serialize, Deserialize)]
pub struct SignedRevocationList {
    pub issued_at: u64,
    pub revoked_keys: Vec<PublicKey>,
    pub release_key_cert: KeyCertificate,
    #[serde(with = "crypto::fixed_bytes")]
    pub signature: [u8; 64],
}

impl SignedRevocationList {
    pub fn issue(
        release_key: &KeyPair,
        release_key_cert: KeyCertificate,
        revoked_keys: Vec<PublicKey>,
        issued_at: u64,
    ) -> Self {
        let signature = release_key.sign(&Self::signing_bytes(&revoked_keys, issued_at));
        Self {
            issued_at,
            revoked_keys,
            release_key_cert,
            signature,
        }
    }

    fn signing_bytes(revoked_keys: &[PublicKey], issued_at: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + revoked_keys.len() * 32);
        buf.extend_from_slice(&issued_at.to_be_bytes());
        for k in revoked_keys {
            buf.extend_from_slice(&k.0);
        }
        buf
    }

    /// Verify the release key chains to the pinned root and the list is
    /// signed by that (now-validated) release key, then materialize a
    /// lookup-able `RevocationList`.
    pub fn verify(&self, trust_root: &TrustRoot) -> Result<RevocationList, ConfigError> {
        self.release_key_cert
            .verify_against(&trust_root.root_public_key)
            .map_err(|_| ConfigError::InvalidCertChain)?;
        verify(
            &self.release_key_cert.child_key,
            &Self::signing_bytes(&self.revoked_keys, self.issued_at),
            &self.signature,
        )
        .map_err(|_| ConfigError::InvalidSignature)?;
        Ok(RevocationList::from_keys(self.revoked_keys.iter().copied()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hierarchy::KeyCertificate;

    #[test]
    fn signed_revocation_list_verifies_and_revokes() {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
        let revoked_key = KeyPair::generate().public_key();

        let list = SignedRevocationList::issue(&release, release_cert, vec![revoked_key], 100);
        let trust_root = TrustRoot {
            root_public_key: root.public_key(),
        };
        let materialized = list.verify(&trust_root).unwrap();
        assert!(materialized.contains(&revoked_key));
    }

    #[test]
    fn signed_revocation_list_rejects_wrong_root() {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
        let list = SignedRevocationList::issue(&release, release_cert, vec![], 100);

        let other_root = TrustRoot {
            root_public_key: KeyPair::generate().public_key(),
        };
        assert!(list.verify(&other_root).is_err());
    }

    #[test]
    fn signed_revocation_list_rejects_tampered_entries() {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
        let mut list = SignedRevocationList::issue(&release, release_cert, vec![], 100);
        list.revoked_keys.push(KeyPair::generate().public_key());

        let trust_root = TrustRoot {
            root_public_key: root.public_key(),
        };
        assert!(list.verify(&trust_root).is_err());
    }
}
