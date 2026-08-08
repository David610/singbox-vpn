//! Demonstrates spec §52's failure-independence requirements: one blocked
//! transport/endpoint must not prevent connectivity when an independent
//! one still works, and a rendezvous outage must not prevent connectivity
//! when a cached signed bundle is available.

use tests::fixtures;

/// An address nothing is listening on, to simulate a blocked/unreachable
/// endpoint deterministically (connection refused).
fn blocked_addr() -> std::net::SocketAddr {
    "127.0.0.1:1".parse().unwrap() // low port, nothing listens here in tests
}

#[tokio::test]
async fn transport_blocked_independent_transport_still_succeeds() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
    let relay = fixtures::spawn_combined_relay().await.unwrap();

    // direct-tls candidate points nowhere (simulates that transport being
    // blocked); noise-quic candidate points at the real relay.
    let descriptors = vec![
        fixtures::descriptor(
            "blocked-tls",
            "direct-tls",
            blocked_addr(),
            &relay.cert_sha256_hex,
        ),
        fixtures::descriptor(
            "good-quic",
            "noise-quic",
            relay.quic_addr,
            &relay.cert_sha256_hex,
        ),
    ];
    let engine = fixtures::engine_from_descriptors_with_attempts(descriptors, 12);

    let (transport_id, session) = engine
        .connect("127.0.0.1", test_port)
        .await
        .expect("should succeed via the independent transport despite the other being blocked");
    assert_eq!(transport_id.as_str(), "noise-quic");
    drop(session);
}

#[tokio::test]
async fn endpoint_blocked_independent_endpoint_same_transport_still_succeeds() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
    let relay = fixtures::spawn_combined_relay().await.unwrap();

    let descriptors = vec![
        fixtures::descriptor(
            "blocked-relay",
            "direct-tls",
            blocked_addr(),
            &relay.cert_sha256_hex,
        ),
        fixtures::descriptor(
            "good-relay",
            "direct-tls",
            relay.tls_addr,
            &relay.cert_sha256_hex,
        ),
    ];
    let engine = fixtures::engine_from_descriptors_with_attempts(descriptors, 12);

    let (transport_id, session) = engine
        .connect("127.0.0.1", test_port)
        .await
        .expect("should succeed via the independent endpoint despite the other being blocked");
    assert_eq!(transport_id.as_str(), "direct-tls");
    drop(session);
}

/// A single client-local endpoint failure must not zero out a *transport's*
/// eligibility globally — this asserts the corresponding invariant from
/// FAILURE_CLASSIFICATION.md by checking the working transport (used via
/// a *different* endpoint) is still selectable after the blocked one has
/// failed repeatedly.
#[tokio::test]
async fn repeated_endpoint_failure_does_not_disable_the_transport_family() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
    let relay = fixtures::spawn_combined_relay().await.unwrap();

    let descriptors = vec![
        fixtures::descriptor(
            "blocked-relay",
            "direct-tls",
            blocked_addr(),
            &relay.cert_sha256_hex,
        ),
        fixtures::descriptor(
            "good-relay",
            "direct-tls",
            relay.tls_addr,
            &relay.cert_sha256_hex,
        ),
    ];
    let engine = fixtures::engine_from_descriptors_with_attempts(descriptors, 12);

    for _ in 0..3 {
        let (transport_id, session) = engine.connect("127.0.0.1", test_port).await.unwrap();
        assert_eq!(transport_id.as_str(), "direct-tls");
        drop(session);
    }
}

#[tokio::test]
async fn rendezvous_outage_uses_cached_bundle() {
    let relay = fixtures::spawn_combined_relay().await.unwrap();

    let pool = vec![rendezvous::RelayPoolEntry {
        id: "relay-1".into(),
        transport: "direct-tls".into(),
        address: relay.tls_addr.to_string(),
        provider_tag: "dev".into(),
        capabilities: vec!["STREAM".into()],
        cert_sha256_hex: relay.cert_sha256_hex.clone(),
    }];
    let rz = fixtures::spawn_rendezvous(pool).await.unwrap();

    let cache_dir = tempfile::tempdir().unwrap();
    let cache_path = cache_dir.path().join("bundle.json");

    // First fetch succeeds and populates the cache.
    {
        let source = rendezvous_client::HttpSource::new(rz.base_url.clone());
        let client = rendezvous_client::RendezvousClient::new(source, rz.trust_root, &cache_path);
        let revoked = config::revocation::RevocationList::empty();
        client.get_bundle(&revoked).await.unwrap();
    }

    // Simulate rendezvous being unreachable: point at a closed port.
    {
        let dead_source = rendezvous_client::HttpSource::new("http://127.0.0.1:1".to_string());
        let client =
            rendezvous_client::RendezvousClient::new(dead_source, rz.trust_root, &cache_path);
        let revoked = config::revocation::RevocationList::empty();
        let payload = client
            .get_bundle(&revoked)
            .await
            .expect("cached bundle should still verify and be usable during an outage");
        assert_eq!(payload.endpoints.len(), 1);
    }
}
