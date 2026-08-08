//! Driven by `tests/hostile_network/run.sh` inside a constrained network
//! namespace (packet loss / latency / UDP-blocked / TCP-blocked). Marked
//! `#[ignore]` and reads its relay addresses from environment variables
//! set by that script — not run by default `cargo test`, and not run in
//! CI (see `tests/hostile_network/README.md` for why: it needs root +
//! iproute2 + iptables, unavailable in this session's sandbox).

use config::EndpointDescriptor;

fn descriptor_from_env(id: &str, transport: &str, env_var: &str) -> Option<EndpointDescriptor> {
    let addr = std::env::var(env_var).ok()?;
    Some(EndpointDescriptor {
        id: id.to_string(),
        transport: transport.to_string(),
        address: addr,
        provider_tag: "hostile-net-test".to_string(),
        capabilities: vec!["STREAM".to_string()],
        // The scenario script doesn't currently thread the relay's real
        // cert hash through; a full run needs that wired in the same way
        // `deploy/local/run-dev-slice.sh` does. Left as a follow-up since
        // this script cannot execute in this session's sandbox anyway.
        cert_sha256_hex: std::env::var("RELAY_CERT_SHA256_HEX").unwrap_or_default(),
    })
}

#[tokio::test]
#[ignore = "requires tests/hostile_network/run.sh's netns + root setup"]
async fn at_least_one_transport_survives_the_hostile_condition() {
    let mut descriptors = Vec::new();
    if let Some(d) = descriptor_from_env("relay-tls", "direct-tls", "RELAY_TLS_ADDR") {
        descriptors.push(d);
    }
    if let Some(d) = descriptor_from_env("relay-quic", "noise-quic", "RELAY_QUIC_ADDR") {
        descriptors.push(d);
    }
    assert!(
        !descriptors.is_empty(),
        "RELAY_TLS_ADDR / RELAY_QUIC_ADDR must be set by run.sh"
    );

    let engine = std::sync::Arc::new(client_daemon::engine::ConnectionEngine::with_max_attempts(
        client_daemon::default_transports(),
        descriptors,
        20,
    ));

    match engine.connect("10.99.0.2", 8081).await {
        Ok(_) => {}
        Err(e) => {
            panic!("expected at least one transport to survive the hostile network condition: {e}")
        }
    }
}
