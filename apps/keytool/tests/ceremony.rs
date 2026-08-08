//! End-to-end test of the offline signing ceremony against the real
//! `vpn-keytool` binary and real files on disk: root init -> release
//! issue -> bundle issue -> sign a real bundle with the persisted bundle
//! key -> verify via `config::verify_bundle` -> rotate the bundle key ->
//! revoke the old one -> confirm the old signature is now rejected while
//! the newly rotated-in key still verifies.

use config::TrustRoot;
use crypto::hierarchy::KeyCertificate;
use crypto::{KeyPair, PublicKey};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vpn-keytool"))
}

#[derive(serde::Deserialize)]
struct CertFile {
    public_key: PublicKey,
    cert: KeyCertificate,
}

fn read_cert_file(path: &std::path::Path) -> CertFile {
    let bytes = std::fs::read(path).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn full_ceremony_and_rotation_chain() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("root");
    let release_dir = dir.path().join("release");
    let bundle_dir = dir.path().join("bundle");
    let bundle2_dir = dir.path().join("bundle2");

    let status = bin()
        .args(["root-init", "--out-dir"])
        .arg(&root_dir)
        .status()
        .unwrap();
    assert!(status.success());

    let status = bin()
        .args(["release-issue", "--root-key"])
        .arg(root_dir.join("root.key"))
        .arg("--out-dir")
        .arg(&release_dir)
        .status()
        .unwrap();
    assert!(status.success());

    let status = bin()
        .args(["bundle-issue", "--release-key"])
        .arg(release_dir.join("release.key"))
        .arg("--release-cert")
        .arg(release_dir.join("release.cert.json"))
        .arg("--out-dir")
        .arg(&bundle_dir)
        .status()
        .unwrap();
    assert!(status.success());

    // `verify-chain` subcommand: an operator sanity check that also
    // exercises the same read paths a real deployment script would use.
    let status = bin()
        .args(["verify-chain", "--root-pub"])
        .arg(root_dir.join("root.pub"))
        .arg("--release-cert")
        .arg(release_dir.join("release.cert.json"))
        .arg("--bundle-cert")
        .arg(bundle_dir.join("bundle.cert.json"))
        .status()
        .unwrap();
    assert!(status.success());

    // Load everything back like rendezvous / a client would.
    let root_pub_hex: String =
        serde_json::from_slice(&std::fs::read(root_dir.join("root.pub")).unwrap()).unwrap();
    let trust_root = TrustRoot {
        root_public_key: PublicKey::from_hex(&root_pub_hex).unwrap(),
    };
    let release_cert_file = read_cert_file(&release_dir.join("release.cert.json"));
    let bundle_cert_file = read_cert_file(&bundle_dir.join("bundle.cert.json"));
    let bundle_key = KeyPair::load_from_file(&bundle_dir.join("bundle.key")).unwrap();

    let payload = config::RelayBundlePayload {
        schema_version: config::CURRENT_SCHEMA_VERSION,
        issued_at: 100,
        expires_at: 1_000_000_000,
        nonce: "n".into(),
        endpoints: vec![],
    };
    let signed = config::SignedBundle::sign(
        &payload,
        &bundle_key,
        bundle_cert_file.cert.clone(),
        release_cert_file.cert.clone(),
    )
    .unwrap();

    let empty_revocation = config::revocation::RevocationList::empty();
    config::verify_bundle(
        &signed,
        &trust_root,
        &empty_revocation,
        common::UnixSeconds(500),
    )
    .expect("bundle signed by fresh hierarchy must verify");

    // --- rotate: issue a second bundle key, revoke the first ---
    let status = bin()
        .args(["bundle-issue", "--release-key"])
        .arg(release_dir.join("release.key"))
        .arg("--release-cert")
        .arg(release_dir.join("release.cert.json"))
        .arg("--out-dir")
        .arg(&bundle2_dir)
        .status()
        .unwrap();
    assert!(status.success());

    let revocation_path = dir.path().join("revocation.json");
    let status = bin()
        .args(["revoke-issue", "--release-key"])
        .arg(release_dir.join("release.key"))
        .arg("--release-cert")
        .arg(release_dir.join("release.cert.json"))
        .arg("--revoke")
        .arg(bundle_cert_file.public_key.to_hex())
        .arg("--out")
        .arg(&revocation_path)
        .status()
        .unwrap();
    assert!(status.success());

    let revocation_bytes = std::fs::read(&revocation_path).unwrap();
    let signed_revocation: config::revocation::SignedRevocationList =
        serde_json::from_slice(&revocation_bytes).unwrap();
    let revoked = signed_revocation.verify(&trust_root).unwrap();

    // Old bundle key's signature must now be rejected.
    let err = config::verify_bundle(&signed, &trust_root, &revoked, common::UnixSeconds(500))
        .unwrap_err();
    assert_eq!(err, config::ConfigError::RevokedKey);

    // New bundle key must still verify fine.
    let bundle2_cert_file = read_cert_file(&bundle2_dir.join("bundle.cert.json"));
    let bundle2_key = KeyPair::load_from_file(&bundle2_dir.join("bundle.key")).unwrap();
    let signed2 = config::SignedBundle::sign(
        &payload,
        &bundle2_key,
        bundle2_cert_file.cert.clone(),
        release_cert_file.cert.clone(),
    )
    .unwrap();
    config::verify_bundle(&signed2, &trust_root, &revoked, common::UnixSeconds(500))
        .expect("rotated-in bundle key must still verify");
}

#[test]
fn root_init_refuses_to_overwrite_existing_key() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("root");
    let status = bin()
        .args(["root-init", "--out-dir"])
        .arg(&root_dir)
        .status()
        .unwrap();
    assert!(status.success());

    // Second run against the same directory must fail rather than
    // silently clobber the existing root key.
    let status = bin()
        .args(["root-init", "--out-dir"])
        .arg(&root_dir)
        .status()
        .unwrap();
    assert!(!status.success());
}
