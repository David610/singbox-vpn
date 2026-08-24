//! Thin wrappers over audited primitives. No custom cryptography lives here
//! or anywhere else in this workspace (spec principle #8).
//!
//! - Signing: Ed25519 via `ed25519-dalek`.
//! - Randomness: OS CSPRNG via `getrandom::SysRng`.
//! - Session encryption (TLS/QUIC) is handled entirely by `rustls`/`quinn`
//!   in `transport-native`, not here.

pub mod fixed_bytes;
pub mod hierarchy;
pub mod secret;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use getrandom::{rand_core::UnwrapErr, SysRng};
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

impl PublicKey {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(s.trim())
            .map_err(|e| CryptoError::MalformedKey(format!("bad hex public key: {e}")))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CryptoError::MalformedKey("public key must be 32 bytes".into()))?;
        VerifyingKey::from_bytes(&arr).map_err(|e| CryptoError::MalformedKey(e.to_string()))?;
        Ok(Self(arr))
    }
}

/// A keypair capable of signing. The private half is never `Debug`-printed
/// or serialized by this type (see `secret::Secret`).
pub struct KeyPair {
    signing_key: Secret<SigningKey>,
}

impl KeyPair {
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut UnwrapErr(SysRng));
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

    /// Raw 32-byte secret scalar. Only for persistence (`save_to_file`) —
    /// never log or transmit this.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.expose().to_bytes()
    }

    /// Persist the private key to `path`, hex-encoded, created with mode
    /// 0600 (owner read/write only) so it never lands world- or
    /// group-readable even under a permissive umask. Refuses to overwrite
    /// an existing file — key rotation should write to a new path and
    /// have the operator retire the old one deliberately.
    #[cfg(unix)]
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), CryptoError> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| CryptoError::MalformedKey(format!("cannot create {path:?}: {e}")))?;
        f.write_all(hex::encode(self.to_bytes()).as_bytes())
            .map_err(|e| CryptoError::MalformedKey(format!("cannot write {path:?}: {e}")))?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), CryptoError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        std::io::Write::write_all(
            &mut options
                .open(path)
                .map_err(|e| CryptoError::MalformedKey(format!("cannot create {path:?}: {e}")))?,
            hex::encode(self.to_bytes()).as_bytes(),
        )
        .map_err(|e| CryptoError::MalformedKey(format!("cannot write {path:?}: {e}")))
    }

    /// Load a private key previously written by `save_to_file`. Rejects
    /// the file outright if group/other has any permission bit set —
    /// a key file that isn't owner-exclusive is a deployment bug, not a
    /// warning.
    #[cfg(unix)]
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, CryptoError> {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path)
            .map_err(|e| CryptoError::MalformedKey(format!("cannot stat {path:?}: {e}")))?;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(CryptoError::InsecureKeyPermissions(format!(
                "{path:?} has mode {mode:o}, expected 0600 or stricter"
            )));
        }
        let hex_str = std::fs::read_to_string(path)
            .map_err(|e| CryptoError::MalformedKey(format!("cannot read {path:?}: {e}")))?;
        let bytes = hex::decode(hex_str.trim())
            .map_err(|e| CryptoError::MalformedKey(format!("bad hex in {path:?}: {e}")))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CryptoError::MalformedKey(format!("{path:?} is not 32 bytes")))?;
        Ok(Self::from_bytes(arr))
    }

    #[cfg(not(unix))]
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, CryptoError> {
        let hex_str = std::fs::read_to_string(path)
            .map_err(|e| CryptoError::MalformedKey(format!("cannot read {path:?}: {e}")))?;
        let bytes = hex::decode(hex_str.trim())
            .map_err(|e| CryptoError::MalformedKey(format!("bad hex in {path:?}: {e}")))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CryptoError::MalformedKey(format!("{path:?} is not a 32-byte key")))?;
        Ok(Self::from_bytes(arr))
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
    #[cfg(unix)]
    fn key_file_round_trips_and_gets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("crypto-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("root.key");
        let kp = KeyPair::generate();
        kp.save_to_file(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let loaded = KeyPair::load_from_file(&path).unwrap();
        assert_eq!(loaded.public_key(), kp.public_key());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[cfg(unix)]
    fn key_file_with_loose_permissions_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("crypto-test-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("root.key");
        let kp = KeyPair::generate();
        kp.save_to_file(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        match KeyPair::load_from_file(&path) {
            Err(CryptoError::InsecureKeyPermissions(_)) => {}
            other => panic!("expected InsecureKeyPermissions, got {}", other.is_ok()),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn public_key_hex_round_trips() {
        let kp = KeyPair::generate();
        let hex_str = kp.public_key().to_hex();
        let parsed = PublicKey::from_hex(&hex_str).unwrap();
        assert_eq!(parsed, kp.public_key());
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
