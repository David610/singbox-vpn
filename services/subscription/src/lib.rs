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

/// Deployment-wide backstop token bucket.
///
/// This is deliberately NOT per-source-IP. In the shipped deployment this
/// service is bound to loopback behind nginx, which does not forward
/// `X-Forwarded-For` (deliberately — see the vhost template), so every
/// request arrives from `127.0.0.1`. A per-IP limiter therefore collapses
/// to a single shared bucket, and a per-IP *rate* becomes a global one: at
/// the previous 20-token/0.5-per-second setting, one client sending 5 r/s
/// — comfortably inside what nginx's own per-IP `limit_req` permits, so
/// nginx never intervenes — held the shared bucket permanently empty and
/// locked every other user out of subscription delivery indefinitely.
///
/// Per-client fairness is nginx's job (`limit_req zone=...` keyed on
/// `$binary_remote_addr`, which does see real client addresses). This
/// limiter's only remaining job is to stop a runaway or a
/// bypassed-the-proxy caller from saturating the backend, so it is sized
/// for the whole deployment and it logs when it engages — a silent global
/// denial is exactly what made the previous behaviour so hard to see.
pub struct RateLimiter {
    /// Retained only so a deployment that binds this service directly to a
    /// public interface (not the shipped configuration) still gets some
    /// per-peer separation. Entries are evicted once refilled to capacity
    /// so this cannot grow without bound.
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
        let allowed = if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        };
        self.evict_full_buckets(now);
        allowed
    }

    /// Number of live per-peer buckets. Test/diagnostic accessor.
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Bound the bucket map.
    ///
    /// It is keyed by peer address, and nothing removed entries: an attacker
    /// with an IPv6 /64 can mint a fresh source address per request and grow
    /// it without limit. Two passes, cheapest first: drop buckets that have
    /// fully refilled and gone idle (the common, honest case), then — if
    /// still over the cap — keep only the most recently seen addresses.
    ///
    /// Evicting a bucket resets that peer to full capacity, which is
    /// fail-open. That is the right trade for a deployment-wide backstop
    /// whose per-client fairness is enforced upstream by nginx: unbounded
    /// memory growth on a long-running service is the worse failure.
    ///
    /// High/low water marks matter: evicting down to the same threshold that
    /// triggers eviction would run this pass on every single request once the
    /// map is full, turning an O(1) limiter into an O(n log n) one. Trimming
    /// to half means it runs once per `MAX_BUCKETS/2` inserts instead.
    fn evict_full_buckets(&mut self, now: Instant) {
        const MAX_BUCKETS: usize = 8192;
        const TRIM_TO: usize = MAX_BUCKETS / 2;
        if self.buckets.len() <= MAX_BUCKETS {
            return;
        }
        let idle_for = (self.capacity / self.refill_per_sec.max(f64::MIN_POSITIVE)).max(1.0);
        let capacity = self.capacity;
        self.buckets.retain(|_, (tokens, last)| {
            *tokens < capacity || now.duration_since(*last).as_secs_f64() < idle_for
        });
        if self.buckets.len() > TRIM_TO {
            let mut seen: Vec<(IpAddr, Instant)> = self
                .buckets
                .iter()
                .map(|(ip, (_, last))| (*ip, *last))
                .collect();
            seen.sort_by(|a, b| b.1.cmp(&a.1));
            let keep: std::collections::HashSet<IpAddr> =
                seen.into_iter().take(TRIM_TO).map(|(ip, _)| ip).collect();
            self.buckets.retain(|ip, _| keep.contains(ip));
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
    /// Which transport the `format=singbox` subscription's manual
    /// selector defaults to: `reliability` (default, unchanged — REALITY),
    /// `performance` (Hysteria2), or `auto` (sing-box's own urltest
    /// group). See `compat_config::render::SelectionProfile`'s doc
    /// comment for exactly what each does and does not change. An
    /// unrecognized value falls back to the default rather than erroring
    /// — a typo in a query parameter must not break someone's VPN
    /// subscription.
    pub profile: Option<String>,
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
            // Log it: this limiter is a deployment-wide backstop, so it
            // firing means the whole service is shedding load, not that one
            // noisy client is being trimmed. Silence here previously made a
            // total subscription outage indistinguishable from normal
            // operation. No token or user id is logged.
            tracing::warn!(
                peer = %addr.ip(),
                "subscription backend rate limit engaged — requests are being shed \
                 service-wide; per-client fairness is enforced by nginx"
            );
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

    // Per-request success metadata (which user fetched their subscription,
    // when, in what format) is not stored by default: the production unit
    // (deploy/almalinux/systemd/vpn-subscription.service) sets
    // `RUST_LOG=warn`, so this `debug` event is filtered out before it ever
    // reaches the journal. An operator troubleshooting a specific
    // deployment can still see it by explicitly raising the level
    // (`RUST_LOG=debug systemctl edit vpn-subscription` or an ad hoc
    // `RUST_LOG=debug vpn-subscription-svc ...` run) — this line is never
    // deleted, only demoted, so that diagnostic path stays available. Never
    // include the token itself here — see `find_user_by_token`'s no-user-
    // enumeration doc comment; `user_id` is not the bearer credential, but
    // it is still per-user activity metadata, so it does not appear in the
    // default-on log path either.
    tracing::debug!(
        user_id = %user.id,
        format = ?query.format,
        profile = ?query.profile,
        "subscription served"
    );

    let format = query.format.as_deref().unwrap_or("singbox");
    let profile = query
        .profile
        .as_deref()
        .and_then(render::SelectionProfile::parse)
        .unwrap_or_default();
    match format {
        "singbox" => match render::render_singbox_client_subscription_with_profile(
            &user,
            &state.endpoints,
            profile,
        ) {
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
                "www.google.com",
                None,
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
    async fn hiddify_format_is_identical_to_uri_format() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let uri_resp = oneshot_with_addr(state.clone(), "/sub/goodtoken?format=uri").await;
        let hiddify_resp = oneshot_with_addr(state, "/sub/goodtoken?format=hiddify").await;
        assert_eq!(uri_resp.status(), StatusCode::OK);
        assert_eq!(hiddify_resp.status(), StatusCode::OK);
        let uri_body = axum::body::to_bytes(uri_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let hiddify_body = axum::body::to_bytes(hiddify_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            uri_body, hiddify_body,
            "?format=hiddify must render exactly what ?format=uri renders"
        );
    }

    #[tokio::test]
    async fn user_id_is_never_accepted_where_a_subscription_token_is_expected() {
        // Regression for the user-id/token confusion: a user's public ID
        // (`user_<uuid>`, e.g. from `vpn-admin user list`) must never work
        // as a `/sub/` path segment — only the opaque high-entropy token
        // returned once at `create`/`rotate-token` time does.
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let resp = oneshot_with_addr(state, "/sub/u1").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
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

    /// Captures real emitted log output at both the production default
    /// filter (`warn`, see `deploy/almalinux/systemd/vpn-subscription.service`)
    /// and an operator-raised filter (`debug`), so this is a behavioral
    /// regression test against the actual `tracing` output, not a
    /// source-grep of the macro name used at the call site.
    fn captured_log_output_for_request(
        filter: &str,
        uri: &str,
        state: std::sync::Arc<AppState>,
    ) -> String {
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
            type Writer = Buf;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Buf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
            .finish();
        // A dedicated current-thread runtime driven from inside
        // `with_default`'s synchronous closure: `tracing`'s default
        // subscriber is thread-local, and a multi-thread runtime (or the
        // ambient `#[tokio::test]` runtime, whose worker thread this
        // function does not control) could hop the request's future onto
        // a thread that never had this subscriber installed, silently
        // capturing nothing and making the test vacuously pass.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let uri = uri.to_string();
        tracing::subscriber::with_default(subscriber, || {
            rt.block_on(oneshot_with_addr(state, &uri));
        });
        let captured = buf.0.lock().unwrap().clone();
        String::from_utf8(captured).unwrap()
    }

    #[test]
    fn subscription_served_event_is_not_logged_at_the_production_default_level() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let warn_output = captured_log_output_for_request("warn", "/sub/goodtoken", state);
        assert!(
            !warn_output.contains("subscription served"),
            "the per-request success line (user_id/format/profile) must NOT appear at the \
             production default (warn) log level — it is normal user activity metadata, not a \
             failure diagnostic:\n{warn_output}"
        );
    }

    #[test]
    fn subscription_served_event_is_still_available_when_an_operator_raises_the_log_level() {
        let state = make_state(vec![user_with_token("goodtoken", true)]);
        let debug_output = captured_log_output_for_request("debug", "/sub/goodtoken", state);
        assert!(
            debug_output.contains("subscription served"),
            "the per-request success line must still be available when an operator explicitly \
             raises RUST_LOG for troubleshooting — it must be demoted, never deleted:\n{debug_output}"
        );
    }

    /// Re-audits every log path this service can emit (rate-limit warning,
    /// user-store load failure, render failures, and the per-request
    /// success line) at the most permissive filter (`trace`) — the level
    /// an operator could plausibly turn on — and confirms none of it ever
    /// contains a live subscription token, VLESS UUID, or Hysteria2
    /// password. REALITY private key / TLS private key / obfuscation
    /// password material is never read by this service at all (only the
    /// REALITY public key/short_id are loaded into `AppState`, see
    /// `main.rs`), so it is structurally unable to appear in any log line
    /// here.
    #[test]
    fn no_secret_material_appears_in_any_log_line_at_any_verbosity() {
        let token = "secret-token-do-not-log-me";
        let vless_uuid = "22222222-2222-4222-8222-222222222222";
        let hysteria_password = "hysteria2-secret-do-not-log-me";
        let user = CompatUser {
            id: "u_secretcheck".into(),
            name: "secret-check".into(),
            enabled: true,
            vless_uuid: vless_uuid.into(),
            hysteria2_password: SecretString::new(hysteria_password),
            subscription_token_hash_hex: credentials::hash_token(token),
            created_at: 0,
            expires_at: None,
        };
        let state = make_state(vec![user]);
        let output = captured_log_output_for_request("trace", &format!("/sub/{token}"), state);
        assert!(
            !output.contains(token),
            "the subscription token must never appear in any log line:\n{output}"
        );
        assert!(
            !output.contains(vless_uuid),
            "the VLESS UUID must never appear in any log line:\n{output}"
        );
        assert!(
            !output.contains(hysteria_password),
            "the Hysteria2 password must never appear in any log line:\n{output}"
        );
    }
}

#[cfg(test)]
mod rate_limit_regression {
    use super::RateLimiter;
    use std::net::IpAddr;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([127, 0, 0, n])
    }

    /// Regression guard for the deployment-wide lockout.
    ///
    /// In the shipped configuration this service sits behind nginx, which
    /// does not forward the client address, so EVERY request arrives from
    /// 127.0.0.1 and the "per-IP" bucket is really one global bucket. With
    /// the old 20-token / 0.5-per-second budget, a single caller sending
    /// ~5 r/s — well inside nginx's own per-IP allowance, so nginx never
    /// intervened — kept that bucket empty and denied subscription delivery
    /// to every user of the deployment, indefinitely.
    ///
    /// The bucket must be sized so that a burst of that shape drains and
    /// recovers within a second, not within minutes.
    #[test]
    fn a_single_source_burst_does_not_lock_out_the_deployment() {
        let mut limiter = RateLimiter::new(200.0, 50.0);
        // Attacker burns a full burst from one source.
        let mut denied = 0;
        for _ in 0..400 {
            if !limiter.allow(ip(1)) {
                denied += 1;
            }
        }
        assert!(denied > 0, "test is not actually exhausting the bucket");

        // Recovery must be on the order of a second, not minutes. At the old
        // 0.5 tokens/sec this needed ~400 seconds; at 50/sec it is ~4s, so
        // 200ms refills roughly ten tokens in the SAME bucket. Checking a
        // different source would create a fresh full bucket and vacuously
        // pass even if the exhausted deployment-facing bucket never recovered.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            limiter.allow(ip(1)),
            "the exhausted deployment-facing bucket did not recover after its documented refill interval"
        );
    }

    /// The bucket map is keyed by peer address, so it must not grow without
    /// bound when addresses are cheap to mint (an IPv6 /64 gives an attacker
    /// effectively unlimited distinct sources).
    #[test]
    fn bucket_map_does_not_grow_without_bound() {
        let mut limiter = RateLimiter::new(200.0, 50.0);
        for i in 0..20_000u32 {
            let octets = i.to_be_bytes();
            limiter.allow(IpAddr::from([10, octets[1], octets[2], octets[3]]));
        }
        assert!(
            limiter.bucket_count() <= 8192,
            "rate-limiter bucket map retained every distinct source address \
             ({} entries) — unbounded memory growth",
            limiter.bucket_count()
        );
    }
}
