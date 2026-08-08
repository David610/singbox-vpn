//! Shared fixtures for spinning up a full local vertical slice
//! (rendezvous + relay + test-service + client engine) in-process, purely
//! on loopback, for integration tests.

use client_daemon::engine::ConnectionEngine;
use config::{EndpointDescriptor, TrustRoot};
use relay_agent::{RelayLimits, Role};
use std::sync::Arc;
use transport_native::server::RelayIdentity;

pub struct RelayHandle {
    pub tls_addr: std::net::SocketAddr,
    pub quic_addr: std::net::SocketAddr,
    pub cert_sha256_hex: String,
}

/// Starts a combined ingress+egress relay bound to ephemeral ports on both
/// direct-tls and noise-quic, returning its addresses and pinned-cert hash.
pub async fn spawn_combined_relay() -> anyhow::Result<RelayHandle> {
    let identity = RelayIdentity::generate("relay");
    let cert_sha256_hex = hex::encode(identity.cert_sha256);

    let tls_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let tls_addr = tls_listener.local_addr()?;
    let identity_tls = clone_identity(&identity);
    tokio::spawn(async move {
        let _ = relay_agent::serve_tls_on(
            tls_listener,
            &identity_tls,
            Role::Combined,
            RelayLimits::default(),
        )
        .await;
    });

    let quic_endpoint = relay_agent::quic_listen("127.0.0.1:0".parse()?, &identity)?;
    let quic_addr = quic_endpoint.local_addr()?;
    tokio::spawn(async move {
        let _ =
            relay_agent::serve_quic_on(quic_endpoint, Role::Combined, RelayLimits::default()).await;
    });

    // Give the accept loops a tick to actually start listening.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    Ok(RelayHandle {
        tls_addr,
        quic_addr,
        cert_sha256_hex,
    })
}

/// Starts a two-hop ingress -> egress topology, both on direct-tls.
pub struct SplitRelayHandle {
    pub ingress_addr: std::net::SocketAddr,
    pub egress_addr: std::net::SocketAddr,
    pub egress_cert_sha256_hex: String,
    pub ingress_cert_sha256_hex: String,
}

pub async fn spawn_split_relay() -> anyhow::Result<SplitRelayHandle> {
    let egress_identity = RelayIdentity::generate("egress");
    let egress_cert_sha256_hex = hex::encode(egress_identity.cert_sha256);
    let egress_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let egress_addr = egress_listener.local_addr()?;
    let egress_identity_clone = clone_identity(&egress_identity);
    tokio::spawn(async move {
        let _ = relay_agent::serve_tls_on(
            egress_listener,
            &egress_identity_clone,
            Role::Egress,
            RelayLimits::default(),
        )
        .await;
    });

    let mut pinned = [0u8; 32];
    pinned.copy_from_slice(&egress_identity.cert_sha256);
    let next_hop = transport_api::Endpoint {
        id: common::EndpointId("egress".into()),
        address: egress_addr,
        provider_tag: "dev".into(),
        pinned_cert_sha256: pinned,
    };

    let ingress_identity = RelayIdentity::generate("ingress");
    let ingress_cert_sha256_hex = hex::encode(ingress_identity.cert_sha256);
    let ingress_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let ingress_addr = ingress_listener.local_addr()?;
    tokio::spawn(async move {
        let _ = relay_agent::serve_tls_on(
            ingress_listener,
            &ingress_identity,
            Role::Ingress { next_hop },
            RelayLimits::default(),
        )
        .await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    Ok(SplitRelayHandle {
        ingress_addr,
        egress_addr,
        egress_cert_sha256_hex,
        ingress_cert_sha256_hex,
    })
}

fn clone_identity(identity: &RelayIdentity) -> RelayIdentity {
    RelayIdentity {
        cert_der: identity.cert_der.clone(),
        key_der: identity.key_der.clone_key(),
        cert_sha256: identity.cert_sha256,
    }
}

pub fn descriptor(
    id: &str,
    transport: &str,
    addr: std::net::SocketAddr,
    cert_sha256_hex: &str,
) -> EndpointDescriptor {
    EndpointDescriptor {
        id: id.to_string(),
        transport: transport.to_string(),
        address: addr.to_string(),
        provider_tag: "dev".to_string(),
        capabilities: vec!["STREAM".to_string()],
        cert_sha256_hex: cert_sha256_hex.to_string(),
    }
}

pub fn engine_from_descriptors(descriptors: Vec<EndpointDescriptor>) -> Arc<ConnectionEngine> {
    client_daemon::build_engine(descriptors)
}

pub fn engine_from_descriptors_with_attempts(
    descriptors: Vec<EndpointDescriptor>,
    max_attempts: usize,
) -> Arc<ConnectionEngine> {
    Arc::new(ConnectionEngine::with_max_attempts(
        client_daemon::default_transports(),
        descriptors,
        max_attempts,
    ))
}

/// Rendezvous fixture: real signing hierarchy + HTTP server on loopback.
pub struct RendezvousHandle {
    pub base_url: String,
    pub trust_root: TrustRoot,
}

pub async fn spawn_rendezvous(
    pool: Vec<rendezvous::RelayPoolEntry>,
) -> anyhow::Result<RendezvousHandle> {
    let (root_pub, bundle_key, bundle_cert, release_cert) = rendezvous::dev_key_hierarchy();
    let state = Arc::new(rendezvous::AppState {
        bundle_key,
        bundle_key_cert: bundle_cert,
        release_key_cert: release_cert,
        pool,
        subset_size: 5,
        ttl_secs: 900,
        rate_limiter: std::sync::Mutex::new(rendezvous::RateLimiter::new(1000.0, 1000.0)),
        revocation_list_bytes: None,
    });
    let app = rendezvous::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    Ok(RendezvousHandle {
        base_url: format!("http://{addr}"),
        trust_root: TrustRoot {
            root_public_key: root_pub,
        },
    })
}
