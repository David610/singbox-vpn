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

use relay_agent::{RelayLimits, Role};
use std::time::Duration;
use transport_native::server::RelayIdentity;

#[tokio::test]
async fn silent_connections_cannot_permanently_exhaust_the_connection_cap() {
    let identity = RelayIdentity::generate("relay-dos-test");
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

    // A legitimate client must now be able to get a slot and start a TLS
    // handshake. We don't need it to complete a full session — reaching the
    // point where the server writes a ServerHello proves the slot was
    // reclaimed and the accept loop is still serving.
    let probe = tokio::time::timeout(Duration::from_secs(5), async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut sock = tokio::net::TcpStream::connect(addr).await?;
        // Minimal TLS 1.3 ClientHello is unnecessary: any bytes make the
        // server's rustls acceptor respond (with an alert at worst), which
        // is enough to prove it was serviced rather than starved.
        sock.write_all(&[0x16, 0x03, 0x01, 0x00, 0x05, 0x01, 0x00, 0x00, 0x01, 0x00])
            .await?;
        let mut buf = [0u8; 1];
        // Ok(0) (clean EOF/alert) and Ok(1) both mean the server engaged.
        let n = sock.read(&mut buf).await?;
        Ok::<usize, std::io::Error>(n)
    })
    .await;

    assert!(
        probe.is_ok(),
        "relay never serviced a legitimate connection after {max_connections} silent sockets \
         occupied every slot — the connection cap was permanently exhausted"
    );

    drop(squatters);
}
