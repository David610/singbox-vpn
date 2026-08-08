use config::{
    revocation::RevocationList, verify_bundle, EndpointDescriptor, RelayBundlePayload,
    SignedBundle, TrustRoot, CURRENT_SCHEMA_VERSION,
};
use criterion::{criterion_group, criterion_main, Criterion};
use crypto::hierarchy::KeyCertificate;
use crypto::KeyPair;

fn bench_verify(c: &mut Criterion) {
    let root = KeyPair::generate();
    let release = KeyPair::generate();
    let bundle_key = KeyPair::generate();
    let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
    let bundle_cert = KeyCertificate::issue(&release, bundle_key.public_key(), 2);
    let payload = RelayBundlePayload {
        schema_version: CURRENT_SCHEMA_VERSION,
        issued_at: 100,
        expires_at: 1_000_000_000,
        nonce: "bench".into(),
        endpoints: (0..5)
            .map(|i| EndpointDescriptor {
                id: format!("relay-{i}"),
                transport: "direct-tls".into(),
                address: "127.0.0.1:9000".into(),
                provider_tag: "dev".into(),
                capabilities: vec!["STREAM".into()],
                cert_sha256_hex: "00".repeat(32),
            })
            .collect(),
    };
    let bundle = SignedBundle::sign(&payload, &bundle_key, bundle_cert, release_cert).unwrap();
    let trust_root = TrustRoot {
        root_public_key: root.public_key(),
    };
    let revoked = RevocationList::empty();

    c.bench_function("verify_bundle (5 endpoints)", |b| {
        b.iter(|| {
            verify_bundle(&bundle, &trust_root, &revoked, common::UnixSeconds(500)).unwrap();
        })
    });
}

criterion_group!(benches, bench_verify);
criterion_main!(benches);
