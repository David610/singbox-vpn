//! Signed configuration bundle schema and verification.
//!
//! Every remotely-supplied artifact (today: relay bundles from rendezvous)
//! must be authenticated before any field is trusted (spec §27, §33
//! invariant "an expired signed bundle must not silently remain trusted
//! forever"). This crate is the single place that enforces:
//!
//! 1. signature chain validity (bundle key -> release key -> pinned root),
//! 2. revocation of any key in that chain,
//! 3. expiry / not-yet-valid bounds,
//! 4. schema version support.

pub mod revocation;

use common::UnixSeconds;
use crypto::hierarchy::KeyCertificate;
use crypto::{verify, KeyPair, PublicKey};
use revocation::RevocationList;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1];

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ConfigError {
    #[error("bundle signature invalid")]
    InvalidSignature,
    #[error("key certificate chain invalid")]
    InvalidCertChain,
    #[error("bundle expired at {expired_at}, now {now}")]
    Expired { expired_at: u64, now: u64 },
    #[error("bundle not yet valid: issued_at {issued_at}, now {now}")]
    NotYetValid { issued_at: u64, now: u64 },
    #[error("unsupported schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("signing key has been revoked")]
    RevokedKey,
    #[error("malformed payload: {0}")]
    MalformedPayload(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointDescriptor {
    pub id: String,
    pub transport: String,
    pub address: String,
    pub provider_tag: String,
    pub capabilities: Vec<String>,
    /// Hex-encoded SHA-256 of the relay's certificate DER (see
    /// `transport_native::cert::PinnedCertVerifier`).
    pub cert_sha256_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayBundlePayload {
    pub schema_version: u32,
    pub issued_at: u64,
    pub expires_at: u64,
    pub nonce: String,
    pub endpoints: Vec<EndpointDescriptor>,
}

#[derive(Clone, Copy)]
pub struct TrustRoot {
    pub root_public_key: PublicKey,
}

/// A fully-assembled, still-unverified signed relay bundle as received from
/// the rendezvous service (or loaded from the local emergency-bundle cache).
#[derive(Clone, Serialize, Deserialize)]
pub struct SignedBundle {
    /// Canonical JSON bytes of the `RelayBundlePayload` that was signed.
    pub payload_bytes: Vec<u8>,
    #[serde(with = "crypto::fixed_bytes")]
    pub bundle_signature: [u8; 64],
    pub bundle_signing_key: PublicKey,
    pub bundle_key_cert: KeyCertificate,
    pub release_key_cert: KeyCertificate,
}

impl SignedBundle {
    /// Build+sign a bundle. Used by `services/rendezvous` and by tests /
    /// the dev key-generation tooling — never by a client, which only
    /// ever verifies.
    pub fn sign(
        payload: &RelayBundlePayload,
        bundle_key: &KeyPair,
        bundle_key_cert: KeyCertificate,
        release_key_cert: KeyCertificate,
    ) -> Result<Self, ConfigError> {
        let payload_bytes = serde_json::to_vec(payload)
            .map_err(|e| ConfigError::MalformedPayload(e.to_string()))?;
        let bundle_signature = bundle_key.sign(&payload_bytes);
        Ok(Self {
            payload_bytes,
            bundle_signature,
            bundle_signing_key: bundle_key.public_key(),
            bundle_key_cert,
            release_key_cert,
        })
    }
}

/// Verify the full chain + expiry + schema version, returning the parsed
/// payload only if every check passes. This is the only way to obtain a
/// `RelayBundlePayload` from untrusted bytes in this workspace.
pub fn verify_bundle(
    bundle: &SignedBundle,
    trust_root: &TrustRoot,
    revoked: &RevocationList,
    now: UnixSeconds,
) -> Result<RelayBundlePayload, ConfigError> {
    // 1. release key must chain to the pinned root.
    bundle
        .release_key_cert
        .verify_against(&trust_root.root_public_key)
        .map_err(|_| ConfigError::InvalidCertChain)?;
    if revoked.contains(&bundle.release_key_cert.child_key) {
        return Err(ConfigError::RevokedKey);
    }

    // 2. bundle signing key must chain to that release key.
    bundle
        .bundle_key_cert
        .verify_against(&bundle.release_key_cert.child_key)
        .map_err(|_| ConfigError::InvalidCertChain)?;
    if bundle.bundle_key_cert.child_key != bundle.bundle_signing_key {
        return Err(ConfigError::InvalidCertChain);
    }
    if revoked.contains(&bundle.bundle_signing_key) {
        return Err(ConfigError::RevokedKey);
    }

    // 3. the payload itself must be signed by that (now-validated) key.
    verify(
        &bundle.bundle_signing_key,
        &bundle.payload_bytes,
        &bundle.bundle_signature,
    )
    .map_err(|_| ConfigError::InvalidSignature)?;

    let payload: RelayBundlePayload = serde_json::from_slice(&bundle.payload_bytes)
        .map_err(|e| ConfigError::MalformedPayload(e.to_string()))?;

    if !SUPPORTED_SCHEMA_VERSIONS.contains(&payload.schema_version) {
        return Err(ConfigError::UnsupportedSchemaVersion(
            payload.schema_version,
        ));
    }
    if now.0 < payload.issued_at {
        return Err(ConfigError::NotYetValid {
            issued_at: payload.issued_at,
            now: now.0,
        });
    }
    if now.0 >= payload.expires_at {
        return Err(ConfigError::Expired {
            expired_at: payload.expires_at,
            now: now.0,
        });
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        trust_root: TrustRoot,
        bundle: SignedBundle,
        payload: RelayBundlePayload,
    }

    fn build_fixture(issued_at: u64, expires_at: u64) -> Fixture {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let bundle_key = KeyPair::generate();

        let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
        let bundle_cert = KeyCertificate::issue(&release, bundle_key.public_key(), 2);

        let payload = RelayBundlePayload {
            schema_version: CURRENT_SCHEMA_VERSION,
            issued_at,
            expires_at,
            nonce: "test-nonce".into(),
            endpoints: vec![EndpointDescriptor {
                id: "relay-1".into(),
                transport: "direct-tls".into(),
                address: "127.0.0.1:9443".into(),
                provider_tag: "dev".into(),
                capabilities: vec!["STREAM".into()],
                cert_sha256_hex: "00".repeat(32),
            }],
        };

        let bundle = SignedBundle::sign(&payload, &bundle_key, bundle_cert, release_cert).unwrap();

        Fixture {
            trust_root: TrustRoot {
                root_public_key: root.public_key(),
            },
            bundle,
            payload,
        }
    }

    #[test]
    fn valid_bundle_verifies_and_round_trips() {
        let f = build_fixture(100, 1000);
        let revoked = RevocationList::empty();
        let got = verify_bundle(&f.bundle, &f.trust_root, &revoked, UnixSeconds(500)).unwrap();
        assert_eq!(got, f.payload);
    }

    #[test]
    fn expired_bundle_is_rejected() {
        let f = build_fixture(100, 1000);
        let revoked = RevocationList::empty();
        let err = verify_bundle(&f.bundle, &f.trust_root, &revoked, UnixSeconds(1000)).unwrap_err();
        assert!(matches!(err, ConfigError::Expired { .. }));
    }

    #[test]
    fn not_yet_valid_bundle_is_rejected() {
        let f = build_fixture(1000, 2000);
        let revoked = RevocationList::empty();
        let err = verify_bundle(&f.bundle, &f.trust_root, &revoked, UnixSeconds(500)).unwrap_err();
        assert!(matches!(err, ConfigError::NotYetValid { .. }));
    }

    #[test]
    fn bad_signature_is_rejected() {
        let f = build_fixture(100, 1000);
        let mut bundle = f.bundle;
        bundle.payload_bytes.push(0xFF); // tamper
        let revoked = RevocationList::empty();
        let err = verify_bundle(&bundle, &f.trust_root, &revoked, UnixSeconds(500)).unwrap_err();
        assert_eq!(err, ConfigError::InvalidSignature);
    }

    #[test]
    fn wrong_root_is_rejected() {
        let f = build_fixture(100, 1000);
        let other_root = TrustRoot {
            root_public_key: KeyPair::generate().public_key(),
        };
        let revoked = RevocationList::empty();
        let err = verify_bundle(&f.bundle, &other_root, &revoked, UnixSeconds(500)).unwrap_err();
        assert_eq!(err, ConfigError::InvalidCertChain);
    }

    #[test]
    fn rejects_bundle_signed_by_revoked_key() {
        let f = build_fixture(100, 1000);
        let mut revoked = RevocationList::empty();
        revoked.revoke(f.bundle.bundle_signing_key);
        let err = verify_bundle(&f.bundle, &f.trust_root, &revoked, UnixSeconds(500)).unwrap_err();
        assert_eq!(err, ConfigError::RevokedKey);
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let bundle_key = KeyPair::generate();
        let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
        let bundle_cert = KeyCertificate::issue(&release, bundle_key.public_key(), 2);
        let payload = RelayBundlePayload {
            schema_version: 999,
            issued_at: 100,
            expires_at: 1000,
            nonce: "n".into(),
            endpoints: vec![],
        };
        let bundle = SignedBundle::sign(&payload, &bundle_key, bundle_cert, release_cert).unwrap();
        let trust_root = TrustRoot {
            root_public_key: root.public_key(),
        };
        let revoked = RevocationList::empty();
        let err = verify_bundle(&bundle, &trust_root, &revoked, UnixSeconds(500)).unwrap_err();
        assert_eq!(err, ConfigError::UnsupportedSchemaVersion(999));
    }

    proptest::proptest! {
        #[test]
        fn signed_bundle_json_parsing_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2000)) {
            // Same property as `fuzz/fuzz_targets/config_bundle.rs`;
            // exercised on every `cargo test` regardless of cargo-fuzz
            // availability — see docs/TEST_STRATEGY.md.
            let _ : Result<SignedBundle, _> = serde_json::from_slice(&bytes);
        }

        #[test]
        fn expiry_check_never_panics(issued_at in 0u64..1_000_000, ttl in 1u64..1_000_000, now_offset in 0u64..2_000_000) {
            let expires_at = issued_at.saturating_add(ttl);
            let f = build_fixture(issued_at, expires_at);
            let revoked = RevocationList::empty();
            let now = UnixSeconds(now_offset);
            // Must never panic regardless of how `now` relates to the window.
            let _ = verify_bundle(&f.bundle, &f.trust_root, &revoked, now);
        }
    }
}
