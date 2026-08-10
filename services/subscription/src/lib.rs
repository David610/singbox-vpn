//! Subscription HTTP service: `GET /sub/{token}` returns a per-user
//! client configuration (sing-box JSON, or a plain URI list) covering
//! both VLESS+REALITY and Hysteria2 endpoints. Deliberately not part of
//! the native `services/rendezvous` protocol (spec §17) — third-party
//! clients have a different trust model (a bearer token over HTTPS, not
//! a signed bundle chain).
//!
//! Binds loopback only; a reverse proxy terminates public HTTPS in front
//! of it (spec §27) — this service performs no TLS termination itself.

use axum::extract::{ConnectInfo, Path as AxumPath, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use compat_config::model::CompatEndpoint;
use compat_config::{credentials, render, CompatUser};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

pub struct AppState {
    pub users_file: std::path::PathBuf,
    pub endpoints: Vec<CompatEndpoint>,
    pub rate_limiter: Mutex<RateLimiter>,
}

/// Same per-source-IP token-bucket approach as `services/rendezvous`
/// (see `docs/RENDEZVOUS_DESIGN.md`) — adequate for a single VPS, an
/// edge/CDN limiter is the documented follow-up for a larger deployment.
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

/// Looks up the user whose token hash matches, without ever comparing
/// against a specific user by index/short-circuiting early — every
/// active user's hash is checked via constant-time comparison so timing
/// cannot reveal how close a guessed token got, and there is no
/// enumerable "which user does this token belong to" side channel
/// (spec §26: no user enumeration).
fn find_user_by_token(users: &[CompatUser], token: &str, now_unix: i64) -> Option<CompatUser> {
    let mut found = None;
    for u in users {
        let matches = credentials::verify_token(token, &u.subscription_token_hash_hex);
        if matches && u.is_active(now_unix) {
            found = Some(u.clone());
        }
    }
    found
}

#[derive(serde::Deserialize)]
pub struct SubQuery {
    pub format: Option<String>,
}

async fn get_subscription(
    State(state): State<std::sync::Arc<AppState>>,
    AxumPath(token): AxumPath<String>,
    Query(query): Query<SubQuery>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
) -> Response {
    {
        // A poisoned lock (some other request's handler panicked while
        // holding it) does not mean the limiter's buckets are corrupt —
        // recover the guard instead of panicking every request forever.
        let mut limiter = state
            .rate_limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !limiter.allow(addr.ip()) {
            return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
        }
    }

    // Bound token length before any comparison work — reject obviously
    // malformed input cheaply (spec §26 hardening, same pattern as the
    // native config/rendezvous parsers' max-length checks).
    if token.is_empty() || token.len() > 128 {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let users = match compat_config::store::load_users(&state.users_file) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = %e, "failed to load user store");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };
    let now = common::UnixSeconds::now().0 as i64;
    let user = match find_user_by_token(&users, &token, now) {
        // Generic 404 whether the token is unknown, disabled, or
        // expired — an attacker cannot distinguish "wrong token" from
        // "right token, disabled user" (spec §26/§29).
        None => return (StatusCode::NOT_FOUND, "not found").into_response(),
        Some(u) => u,
    };

    tracing::info!(user_id = %user.id, format = ?query.format, "subscription served");

    let format = query.format.as_deref().unwrap_or("singbox");
    match format {
        "singbox" => match render::render_singbox_client_subscription(&user, &state.endpoints) {
            Ok(doc) => (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::to_string_pretty(&doc).unwrap(),
            )
                .into_response(),
            Err(e) => {
                tracing::error!(error = %e, "failed to render singbox subscription");
                (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
            }
        },
        "uri" | "hiddify" => match render::render_uri_list(&user, &state.endpoints) {
            Ok(body) => (
                StatusCode::OK,
                [("content-type", "text/plain; charset=utf-8")],
                body,
            )
                .into_response(),
            Err(e) => {
                tracing::error!(error = %e, "failed to render uri list");
                (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
            }
        },
        _ => (StatusCode::BAD_REQUEST, "unknown format").into_response(),
    }
}

async fn health() -> &'static str {
    "ok"
}

/// `/sub/*` responses embed the VLESS UUID and Hysteria2 password, so
/// they must never be cached by an intermediate proxy or the client's
/// disk cache. Applied to every response on this route — success and
/// error alike — not just the 200 path.
async fn no_store_headers(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("pragma", HeaderValue::from_static("no-cache"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    resp
}

pub fn build_router(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/sub/:token",
            get(get_subscription).layer(axum::middleware::from_fn(no_store_headers)),
        )
        .route("/healthz", get(health))
        .route("/internal/state-fingerprint", get(state_fingerprint))
        .with_state(state)
}

/// Re-exported so existing `use subscription::standard_endpoints` call
/// sites (and this crate's own tests) keep working unchanged. Moved to
/// `compat_config::render` so `apps/admin`'s `doctor` can build the
/// EXACT same endpoint set this service builds — a coherence check that
/// hand-rolled its own equivalent construction on the `vpn-admin` side
/// would only prove two different implementations agree by
/// coincidence, not that they're actually the same computation.
pub use compat_config::render::standard_endpoints;

/// `GET /internal/state-fingerprint` — loopback-only (same trust
/// boundary as `/healthz`; this whole service never binds a public
/// interface), reveals a SHA-256 fingerprint of this ALREADY-RUNNING
/// process's in-memory `state.endpoints`, never the underlying values.
/// This is the only way to detect the split-brain incident class this
/// mechanism exists for: `vpn-subscription` reads REALITY public
/// key/short_id from disk once at startup and caches it for its entire
/// lifetime (no config-reload path — see `main.rs`), so re-reading the
/// current files from a *different*, freshly-run process (`vpn-admin
/// doctor`) can only prove what a fresh restart *would* serve, never
/// what this specific running process actually *is* serving right now.
/// `vpn-admin doctor` fetches this and compares it against a fingerprint
/// of a fresh disk read — a mismatch means this process needs a real
/// restart, not just that the files changed.
async fn state_fingerprint(State(state): State<std::sync::Arc<AppState>>) -> Response {
    let fingerprint = compat_config::render::endpoints_fingerprint(&state.endpoints);
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::json!({ "endpoints_fingerprint_sha256": fingerprint }).to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use compat_config::secret::SecretString;

    fn user_with_token(token: &str, enabled: bool) -> CompatUser {
        CompatUser {
            id: "u1".into(),
            name: "test".into(),
            enabled,
            vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
            hysteria2_password: SecretString::new("pw"),
            subscription_token_hash_hex: credentials::hash_token(token),
            created_at: 0,
            expires_at: None,
        }
    }

    fn make_state(users: Vec<CompatUser>) -> std::sync::Arc<AppState> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        compat_config::store::save_users_atomic(&path, &users).unwrap();
        std::mem::forget(dir); // keep temp dir alive for the test's duration
        std::sync::Arc::new(AppState {
            users_file: path,
            endpoints: standard_endpoints(
                "vpn.example.com",
                443,
                443,
                "pub",
                "short",
                "www.microsoft.com",
            ),
            rate_limiter: Mutex::new(RateLimiter::new(1000.0, 1000.0)),
        })
    }

    // axum 0.7's `ConnectInfo` extractor requires the router to be served
    // via `into_make_service_with_connect_info`; drive it directly rather
    // than `oneshot` so a real per-connection `SocketAddr` is available
    // to the rate limiter under test.
    async fn oneshot_with_addr(state: std::sync::Arc<AppState>, uri: &str) -> Response {
        use tower::Service;
        let mut app =
            build_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
        let addr: std::net::SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let mut svc = app.call(addr).await.unwrap();
        svc.call(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn valid_token_returns_singbox_subscription() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(state, "/sub/goodtoken").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let s = String::from_utf8(body.to_vec()).unwrap();
        assert!(s.contains("vless"));
        assert!(s.contains("hysteria2"));
    }

    #[tokio::test]
    async fn unknown_token_returns_generic_404() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(state, "/sub/wrongtoken").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn disabled_user_token_returns_404_not_a_distinguishable_error() {
        let state = make_state(vec![user_with_token("goodtoken", false)]);
        let resp = oneshot_with_addr(state, "/sub/goodtoken").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn uri_format_returns_plaintext_share_links() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(state, "/sub/goodtoken?format=uri").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let s = String::from_utf8(body.to_vec()).unwrap();
        assert!(s.starts_with("vless://"));
    }

    #[tokio::test]
    async fn oversized_token_is_rejected_cheaply() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let long_token = "a".repeat(500);
        let resp = oneshot_with_addr(state, &format!("/sub/{long_token}")).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn subscription_responses_are_never_cacheable_success_or_error() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let ok_resp = oneshot_with_addr(state.clone(), "/sub/goodtoken").await;
        assert_eq!(ok_resp.headers().get("cache-control").unwrap(), "no-store");
        let err_resp = oneshot_with_addr(state, "/sub/wrongtoken").await;
        assert_eq!(err_resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(err_resp.headers().get("cache-control").unwrap(), "no-store");
        assert_eq!(
            err_resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
    }

    #[tokio::test]
    async fn healthz_responds_ok() {
        let state = make_state(vec![]);
        let resp = oneshot_with_addr(state, "/healthz").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // A panic anywhere else while some *other* request holds the rate
    // limiter lock (e.g. a future bug in an unrelated handler sharing this
    // state) poisons the std::sync::Mutex. Requests must keep being served
    // afterwards, not 500/panic forever — a poisoned lock does not mean the
    // limiter's data is actually corrupt, just that a thread died holding it.
    #[tokio::test]
    async fn subscription_survives_a_poisoned_rate_limiter_lock() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state.rate_limiter.lock().unwrap();
            panic!("simulated panic while holding the rate limiter lock");
        })
        .join();
        assert!(state.rate_limiter.is_poisoned());

        let resp = oneshot_with_addr(state, "/sub/goodtoken").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// The whole point of `/internal/state-fingerprint` is that it
    /// reflects THIS process's in-memory `state.endpoints` — reconstruct
    /// the expected fingerprint independently (via the same shared
    /// `endpoints_fingerprint` function `vpn-admin doctor` uses) and
    /// confirm the route reports exactly that, never a raw key/short_id
    /// value.
    #[tokio::test]
    async fn state_fingerprint_matches_in_memory_endpoints_and_never_leaks_raw_values() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let expected = compat_config::render::endpoints_fingerprint(&state.endpoints);

        let resp = oneshot_with_addr(state, "/internal/state-fingerprint").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["endpoints_fingerprint_sha256"], expected);

        let s = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !s.contains("pub"),
            "must never leak the raw public key value"
        );
        assert!(
            !s.contains("short"),
            "must never leak the raw short_id value"
        );
    }
}
