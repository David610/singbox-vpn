//! Revocation list for keys in the signing hierarchy (ADR-0008). Kept as a
//! simple in-memory set here; a real deployment ships this list itself
//! signed by the release key so an attacker who only compromises the
//! always-online rendezvous host cannot un-revoke a key.

use crypto::PublicKey;
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
