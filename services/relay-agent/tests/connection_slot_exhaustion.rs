//! Regression test for the connection-slot exhaustion DoS.
//!
//! The relay acquires a semaphore permit BEFORE performing the TLS
//! handshake. When that handshake had no timeout, a peer that completed the
//! TCP handshake and then sent nothing held its permit forever — so
//! `max_connections` silent sockets from a single host took the relay down
//! permanently, and every subsequent client was dropped with no response.
//!
//! This test drives the real `serve_tls_on` accept loop with real sockets:
//! it fills every connection slot with silent TCP connections, then asserts
//! a legitimate client can still get its TLS handshake through once the
//! handshake timeout has elapsed. It fails (times out waiting for a slot)
//! against the unbounded-handshake version.

use common::EndpointId;
use relay_agent::{RelayLimits, Role};
use std::time::Duration;
use transport_api::Endpoint;
use transport_native::server::RelayIdentity;

#[tokio::test]
async fn silent_connections_cannot_permanently_exhaust_the_connection_cap() {
    let identity = RelayIdentity::generate("relay-dos-test");
    let pinned_cert_sha256 = identity.cert_sha256;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Deliberately tiny cap and a short handshake timeout so the test is
    // fast; the defaults (256 / 10s) exhibit identical behaviour.
    let max_connections = 4;
    let limits = RelayLimits {
        max_connections,
        handshake_timeout: Duration::from_millis(300),
        ..RelayLimits::default()
    };

    tokio::spawn(async move {
        let _ = relay_agent::serve_tls_on(listener, &identity, Role::Combined, limits).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Occupy every slot with a connection that completes TCP and then says
    // nothing at all. Hold them open for the rest of the test.
    let mut squatters = Vec::new();
    for _ in 0..max_connections {
        squatters.push(tokio::net::TcpStream::connect(addr).await.unwrap());
    }

    // Give the relay time to notice the silent peers and reclaim the slots.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // A legitimate client must complete a real pinned TLS handshake. An
    // arbitrary-byte probe or EOF can also result from the old
    // capacity-drop path and therefore cannot prove a slot was reclaimed.
    let endpoint = Endpoint {
        id: EndpointId("relay-dos-test".into()),
        address: addr,
        provider_tag: "test".into(),
        pinned_cert_sha256,
    };
    let probe = tokio::time::timeout(
        Duration::from_secs(5),
        transport_native::direct_tls::connect_stream(&endpoint),
    )
    .await;

    assert!(
        matches!(probe, Ok(Ok(_))),
        "relay did not complete a legitimate pinned TLS handshake after {max_connections} silent \
         sockets occupied every slot: {probe:?}"
    );

    drop(squatters);
}
