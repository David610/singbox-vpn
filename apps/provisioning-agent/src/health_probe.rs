use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

const PROBE_TIMEOUT_MS: u64 = 5_000;
const PROBE_TEST_URL: &str = "https://www.gstatic.com/generate_204";
/// The outbound tag every server-side sing-box document actually renders
/// (`crates/compat-config/src/server.rs::render_singbox_server_config`
/// always emits `{"type": "direct", "tag": "direct"}` and nothing else on
/// a relay; `"PROXY"` is a client-side selector tag that never exists on a
/// server config). Overridable via `AgentConfig::clash_probe_outbound` for
/// an operator whose server-side outbound topology later changes.
const DEFAULT_PROBE_OUTBOUND: &str = "direct";

#[derive(Deserialize)]
struct DelayResponse {
    delay: u64,
}

/// Drives sing-box's own Clash API delay-test endpoint, which actively
/// fetches a real URL through the configured outbound. `None` (not
/// `Some(false)`) when no Clash API is configured for this node — an
/// unconfigured node must never look identical to a genuinely failing one
/// to the control plane (see docs/superpowers/specs/2026-09-25-fleet-
/// phase8-health-failover-design.md §4.1).
pub async fn probe_data_plane(
    http: &reqwest::Client,
    clash_api_url: Option<&str>,
    clash_api_secret: Option<&str>,
    probe_outbound: Option<&str>,
) -> (Option<bool>, Option<u64>) {
    let Some(base_url) = clash_api_url else {
        return (None, None);
    };
    let outbound = probe_outbound.unwrap_or(DEFAULT_PROBE_OUTBOUND);

    match run_probe(http, base_url, clash_api_secret, outbound).await {
        Ok(delay) => (Some(true), Some(delay)),
        Err(err) => {
            // Secret-safe: the URL is local (127.0.0.1) and carries no
            // credential — the bearer secret goes through `.bearer_auth()`
            // and is never formatted into the URL or this error chain.
            tracing::warn!(error = %err, "data-plane probe failed");
            (Some(false), None)
        }
    }
}

/// Runs the actual delay-test request. Errors (unreachable Clash API,
/// non-success status, unparseable body) are all reported via `Result` here
/// and collapsed to `(Some(false), None)` by the caller — matching
/// `stats::read_traffic`'s style of explicit, contextual errors internally,
/// while keeping the public probe permissive at its boundary since a
/// failing probe is exactly the signal Phase 8 failover needs, not a panic
/// or a lost report.
async fn run_probe(
    http: &reqwest::Client,
    base_url: &str,
    secret: Option<&str>,
    outbound: &str,
) -> Result<u64> {
    let url = format!(
        "{}/proxies/{outbound}/delay",
        base_url.trim_end_matches('/')
    );

    let mut request = http
        .get(&url)
        .query(&[
            ("timeout", PROBE_TIMEOUT_MS.to_string()),
            ("url", PROBE_TEST_URL.to_string()),
        ])
        .timeout(Duration::from_millis(PROBE_TIMEOUT_MS + 1_000));
    if let Some(secret) = secret {
        request = request.bearer_auth(secret);
    }

    let res = request
        .send()
        .await
        // The URL is local (127.0.0.1) and carries no credential, so it is
        // safe in an error message; the secret travels in a header and is
        // never formatted into one.
        .with_context(|| format!("GET {url} failed"))?;

    if !res.status().is_success() {
        bail!("GET {url} returned {}", res.status());
    }

    let parsed: DelayResponse = res
        .json()
        .await
        .with_context(|| format!("parsing {url} response body"))?;

    Ok(parsed.delay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn returns_none_when_clash_api_not_configured() {
        let http = reqwest::Client::new();
        let result = probe_data_plane(&http, None, None, None).await;
        assert_eq!(result, (None, None));
    }

    #[tokio::test]
    async fn returns_true_and_latency_on_a_successful_delay_test() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/proxies/direct/delay$"))
            .and(query_param("timeout", PROBE_TIMEOUT_MS.to_string()))
            .and(query_param("url", PROBE_TEST_URL))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "delay": 42 })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let result = probe_data_plane(&http, Some(&server.uri()), None, None).await;
        assert_eq!(result, (Some(true), Some(42)));
    }

    #[tokio::test]
    async fn returns_false_on_a_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/proxies/direct/delay$"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let result = probe_data_plane(&http, Some(&server.uri()), None, None).await;
        assert_eq!(result, (Some(false), None));
    }

    #[tokio::test]
    async fn returns_false_when_the_server_is_unreachable() {
        let http = reqwest::Client::new();
        // Port 1 is reserved and will refuse the connection immediately.
        let result = probe_data_plane(&http, Some("http://127.0.0.1:1"), None, None).await;
        assert_eq!(result, (Some(false), None));
    }

    #[tokio::test]
    async fn honors_an_explicit_probe_outbound_override() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/proxies/custom-outbound/delay$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "delay": 7 })),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let result =
            probe_data_plane(&http, Some(&server.uri()), None, Some("custom-outbound")).await;
        assert_eq!(result, (Some(true), Some(7)));
    }

    // ---- Level 2 (integration): a real sing-box process -----------------
    //
    // Modeled on stats.rs's own real-sing-box tests. These drive the real
    // binary against sing-box's actual outbound-tag semantics, which is
    // exactly what the original "PROXY" bug (probing a tag that never
    // exists on a server config) would have failed under a mocked test —
    // mocks only prove the client code hits the URL it intends to, never
    // that the URL is one a real server-side config exposes.
    //
    // They skip cleanly when sing-box is absent, matching this repo's
    // other interop tests: CI without the binary should not run them, not
    // fail.

    use std::io::Write;
    use std::process::{Child, Command, Stdio};

    struct SingBox {
        child: Child,
        _dir: tempfile::TempDir,
    }

    impl Drop for SingBox {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn singbox_available() -> bool {
        Command::new("sing-box")
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn start_singbox(clash_port: u16, proxy_port: u16, secret: &str) -> Option<SingBox> {
        let dir = tempfile::tempdir().ok()?;
        let config_path = dir.path().join("config.json");
        let config = serde_json::json!({
            "log": { "level": "error" },
            "inbounds": [{
                "type": "mixed", "tag": "in",
                "listen": "127.0.0.1", "listen_port": proxy_port
            }],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "experimental": {
                "clash_api": {
                    "external_controller": format!("127.0.0.1:{clash_port}"),
                    "secret": secret
                }
            }
        });
        std::fs::File::create(&config_path)
            .ok()?
            .write_all(config.to_string().as_bytes())
            .ok()?;

        let child = Command::new("sing-box")
            .args(["run", "-c"])
            .arg(&config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        Some(SingBox { child, _dir: dir })
    }

    /// Waits until the Clash API actually answers. `probe_data_plane` is not
    /// usable as the readiness check: it maps "connection refused" to
    /// `Some(false)`, so polling it for `is_some()` returned on the first
    /// attempt, before sing-box had bound its port, and the real probe then
    /// raced sing-box's startup (a timing-dependent failure).
    async fn wait_until_probeable(http: &reqwest::Client, base: &str, secret: &str) -> bool {
        for _ in 0..80 {
            let ready = http
                .get(format!("{base}/version"))
                .bearer_auth(secret)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ready {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(125)).await;
        }
        false
    }

    #[tokio::test]
    async fn probes_a_real_singbox_direct_outbound_successfully() {
        if !singbox_available() {
            eprintln!("skipping: sing-box not on PATH");
            return;
        }
        let secret = "interop-secret";
        let Some(_sb) = start_singbox(39_711, 39_712, secret) else {
            eprintln!("skipping: could not start sing-box");
            return;
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("building http client");
        let base = "http://127.0.0.1:39711";

        assert!(
            wait_until_probeable(&http, base, secret).await,
            "sing-box Clash API never became probeable"
        );

        let result = probe_data_plane(&http, Some(base), Some(secret), None).await;
        assert_eq!(
            result.0,
            Some(true),
            "the default outbound is \"direct\", which a real server-side config always renders"
        );
        assert!(
            result.1.is_some(),
            "a successful probe must report a latency"
        );
    }

    #[tokio::test]
    async fn probing_a_nonexistent_outbound_returns_false_against_a_real_singbox() {
        if !singbox_available() {
            eprintln!("skipping: sing-box not on PATH");
            return;
        }
        let secret = "interop-secret";
        let Some(_sb) = start_singbox(39_713, 39_714, secret) else {
            eprintln!("skipping: could not start sing-box");
            return;
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("building http client");
        let base = "http://127.0.0.1:39713";

        assert!(
            wait_until_probeable(&http, base, secret).await,
            "sing-box Clash API never became probeable"
        );

        // This is the case that would have caught the original "PROXY" bug:
        // a tag that a real server-side sing-box config never exposes
        // must 404, not silently succeed.
        let result = probe_data_plane(&http, Some(base), Some(secret), Some("PROXY")).await;
        assert_eq!(result, (Some(false), None));
    }
}
