//! Rendezvous service: issues small, signed, expiring relay subsets rather
//! than a full fleet listing. See `docs/RENDEZVOUS_DESIGN.md`.

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use axum::Router;
use common::UnixSeconds;
use config::{EndpointDescriptor, RelayBundlePayload, SignedBundle, CURRENT_SCHEMA_VERSION};
use crypto::hierarchy::KeyCertificate;
use crypto::KeyPair;
use rand::seq::SliceRandom;
use rand::RngCore;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone, serde::Deserialize)]
pub struct RelayPoolEntry {
    pub id: String,
    pub transport: String,
    pub address: String,
    pub provider_tag: String,
    pub capabilities: Vec<String>,
    pub cert_sha256_hex: String,
}

impl From<RelayPoolEntry> for EndpointDescriptor {
    fn from(e: RelayPoolEntry) -> Self {
        EndpointDescriptor {
            id: e.id,
            transport: e.transport,
            address: e.address,
            provider_tag: e.provider_tag,
            capabilities: e.capabilities,
            cert_sha256_hex: e.cert_sha256_hex,
        }
    }
}

pub struct AppState {
    pub bundle_key: KeyPair,
    pub bundle_key_cert: KeyCertificate,
    pub release_key_cert: KeyCertificate,
    pub pool: Vec<RelayPoolEntry>,
    pub subset_size: usize,
    pub ttl_secs: u64,
    pub rate_limiter: Mutex<RateLimiter>,
}

/// Simple per-source-IP token bucket. Adequate for the local dev slice —
/// DEPLOYMENT.md documents that a real deployment needs an edge/CDN rate
/// limiter in front of this.
pub struct RateLimiter {
    buckets: HashMap<IpAddr, (f64, Instant)>,
    capacity: f64,
    refill_per_sec: f64,
}

impl RateLimiter {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            buckets: HashMap::new(),
            capacity,
            refill_per_sec,
        }
    }

    pub fn allow(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let (tokens, last) = self.buckets.entry(ip).or_insert((self.capacity, now));
        let elapsed = now.duration_since(*last).as_secs_f64();
        *tokens = (*tokens + elapsed * self.refill_per_sec).min(self.capacity);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

async fn get_relay_bundle(
    State(state): State<std::sync::Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
) -> impl IntoResponse {
    {
        let mut limiter = state.rate_limiter.lock().unwrap();
        if !limiter.allow(addr.ip()) {
            return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
        }
    }

    let mut rng = rand::thread_rng();
    let n = state.subset_size.min(state.pool.len());
    let subset: Vec<EndpointDescriptor> = state
        .pool
        .choose_multiple(&mut rng, n)
        .cloned()
        .map(Into::into)
        .collect();

    let mut nonce_bytes = [0u8; 16];
    rng.fill_bytes(&mut nonce_bytes);
    let now = UnixSeconds::now();

    let payload = RelayBundlePayload {
        schema_version: CURRENT_SCHEMA_VERSION,
        issued_at: now.0,
        expires_at: now.saturating_add_secs(state.ttl_secs).0,
        nonce: hex::encode(nonce_bytes),
        endpoints: subset,
    };

    let bundle = match SignedBundle::sign(
        &payload,
        &state.bundle_key,
        state.bundle_key_cert.clone(),
        state.release_key_cert.clone(),
    ) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "failed to sign relay bundle");
            return (StatusCode::INTERNAL_SERVER_ERROR, "signing error").into_response();
        }
    };

    Json(bundle).into_response()
}

async fn health() -> &'static str {
    "ok"
}

pub fn build_router(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/relay-bundle", get(get_relay_bundle))
        .route("/healthz", get(health))
        .with_state(state)
}

pub fn dev_key_hierarchy() -> (crypto::PublicKey, KeyPair, KeyCertificate, KeyCertificate) {
    let root = KeyPair::generate();
    let release = KeyPair::generate();
    let bundle_key = KeyPair::generate();
    let release_cert = KeyCertificate::issue(&root, release.public_key(), UnixSeconds::now().0);
    let bundle_cert =
        KeyCertificate::issue(&release, bundle_key.public_key(), UnixSeconds::now().0);
    (root.public_key(), bundle_key, bundle_cert, release_cert)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use config::{verify_bundle, TrustRoot};
    use tower::ServiceExt;

    fn make_state(
        pool_size: usize,
        subset_size: usize,
    ) -> (std::sync::Arc<AppState>, crypto::PublicKey) {
        let (root_pub, bundle_key, bundle_cert, release_cert) = dev_key_hierarchy();
        let pool = (0..pool_size)
            .map(|i| RelayPoolEntry {
                id: format!("relay-{i}"),
                transport: "direct-tls".into(),
                address: format!("127.0.0.1:{}", 9000 + i),
                provider_tag: "dev".into(),
                capabilities: vec!["STREAM".into()],
                cert_sha256_hex: "00".repeat(32),
            })
            .collect();
        let state = std::sync::Arc::new(AppState {
            bundle_key,
            bundle_key_cert: bundle_cert,
            release_key_cert: release_cert,
            pool,
            subset_size,
            ttl_secs: 900,
            rate_limiter: Mutex::new(RateLimiter::new(1000.0, 1000.0)),
        });
        (state, root_pub)
    }

    #[tokio::test]
    async fn healthz_responds_ok() {
        let (state, _root_pub) = make_state(5, 5);
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn subset_never_exceeds_configured_size_across_many_samples() {
        let (state, _root_pub) = make_state(50, 5);
        let mut rng = rand::thread_rng();
        for _ in 0..200 {
            let n = state.subset_size.min(state.pool.len());
            let subset: Vec<_> = state.pool.choose_multiple(&mut rng, n).collect();
            assert!(subset.len() <= 5);
        }
    }

    #[test]
    fn bundle_round_trip_verification() {
        let (state, root_pub) = make_state(3, 3);
        let now = UnixSeconds::now();
        let payload = RelayBundlePayload {
            schema_version: CURRENT_SCHEMA_VERSION,
            issued_at: now.0,
            expires_at: now.saturating_add_secs(state.ttl_secs).0,
            nonce: "abc".into(),
            endpoints: state.pool.iter().cloned().map(Into::into).collect(),
        };
        let bundle = SignedBundle::sign(
            &payload,
            &state.bundle_key,
            state.bundle_key_cert.clone(),
            state.release_key_cert.clone(),
        )
        .unwrap();
        let trust_root = TrustRoot {
            root_public_key: root_pub,
        };
        let revoked = config::revocation::RevocationList::empty();
        let got = verify_bundle(&bundle, &trust_root, &revoked, now).unwrap();
        assert_eq!(got, payload);
    }
}
