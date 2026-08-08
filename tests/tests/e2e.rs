//! Vertical slice: client engine -> relay (ingress+egress) -> test-service,
//! fully local, over both real transports and both relay topologies. This
//! is the spec §51 "traffic flows end-to-end" proof.

use common::framing;
use tests::fixtures;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn assert_tunnel_reaches_test_service(
    engine: std::sync::Arc<client_daemon::engine::ConnectionEngine>,
    test_service_port: u16,
) {
    let (transport_id, session) = engine
        .connect("127.0.0.1", test_service_port)
        .await
        .expect("engine should establish a session");
    tracing::info!(%transport_id, "connected");

    let (mut reader, mut writer) = session.split();
    framing::write_destination(&mut writer, "127.0.0.1", test_service_port)
        .await
        .unwrap();
    writer.write_all(b"GET / \r\n").await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), reader.read(&mut buf))
        .await
        .expect("read timed out")
        .expect("read failed");
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.contains("hello from test-service"),
        "unexpected response: {response}"
    );
}

#[tokio::test]
async fn combined_relay_direct_tls_end_to_end() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
    let relay = fixtures::spawn_combined_relay().await.unwrap();
    let descriptors = vec![fixtures::descriptor(
        "relay-1",
        "direct-tls",
        relay.tls_addr,
        &relay.cert_sha256_hex,
    )];
    let engine = fixtures::engine_from_descriptors(descriptors);
    assert_tunnel_reaches_test_service(engine, test_port).await;
}

#[tokio::test]
async fn combined_relay_noise_quic_end_to_end() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
    let relay = fixtures::spawn_combined_relay().await.unwrap();
    let descriptors = vec![fixtures::descriptor(
        "relay-1",
        "noise-quic",
        relay.quic_addr,
        &relay.cert_sha256_hex,
    )];
    let engine = fixtures::engine_from_descriptors(descriptors);
    assert_tunnel_reaches_test_service(engine, test_port).await;
}

#[tokio::test]
async fn split_ingress_egress_topology_end_to_end() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
    let relay = fixtures::spawn_split_relay().await.unwrap();
    let descriptors = vec![fixtures::descriptor(
        "ingress-1",
        "direct-tls",
        relay.ingress_addr,
        &relay.ingress_cert_sha256_hex,
    )];
    let engine = fixtures::engine_from_descriptors(descriptors);
    assert_tunnel_reaches_test_service(engine, test_port).await;
    // The egress hop's cert hash is used internally by the ingress relay,
    // not by the client — asserting it's non-empty documents that it's
    // actually wired in (see fixtures::spawn_split_relay).
    assert_eq!(relay.egress_cert_sha256_hex.len(), 64);
}

#[tokio::test]
async fn signed_rendezvous_bundle_drives_real_connection() {
    let (test_port, _svc) = test_service::spawn_ephemeral().await.unwrap();
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

    let source = rendezvous_client::HttpSource::new(rz.base_url);
    let cache_dir = tempfile::tempdir().unwrap();
    let cache_path = cache_dir.path().join("bundle.json");
    let client = rendezvous_client::RendezvousClient::new(source, rz.trust_root, &cache_path);
    let revoked = config::revocation::RevocationList::empty();
    let payload = client.get_bundle(&revoked).await.unwrap();
    assert_eq!(payload.endpoints.len(), 1);

    let engine = fixtures::engine_from_descriptors(payload.endpoints);
    assert_tunnel_reaches_test_service(engine, test_port).await;
}
