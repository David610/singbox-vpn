use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

const PROBE_TIMEOUT_MS: u64 = 5_000;
const PROBE_TEST_URL: &str = "https://www.gstatic.com/generate_204";
const PROBE_PROXY_GROUP: &str = "PROXY";

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
) -> (Option<bool>, Option<u64>) {
    let Some(base_url) = clash_api_url else {
        return (None, None);
    };

    match run_probe(http, base_url, clash_api_secret).await {
        Ok(delay) => (Some(true), Some(delay)),
        Err(_) => (Some(false), None),
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
) -> Result<u64> {
    let url = format!(
        "{}/proxies/{PROBE_PROXY_GROUP}/delay?timeout={PROBE_TIMEOUT_MS}&url={PROBE_TEST_URL}",
        base_url.trim_end_matches('/')
    );

    let mut request = http
        .get(&url)
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
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn returns_none_when_clash_api_not_configured() {
        let http = reqwest::Client::new();
        let result = probe_data_plane(&http, None, None).await;
        assert_eq!(result, (None, None));
    }

    #[tokio::test]
    async fn returns_true_and_latency_on_a_successful_delay_test() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/proxies/PROXY/delay$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "delay": 42 })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let result = probe_data_plane(&http, Some(&server.uri()), None).await;
        assert_eq!(result, (Some(true), Some(42)));
    }

    #[tokio::test]
    async fn returns_false_on_a_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/proxies/PROXY/delay$"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let result = probe_data_plane(&http, Some(&server.uri()), None).await;
        assert_eq!(result, (Some(false), None));
    }

    #[tokio::test]
    async fn returns_false_when_the_server_is_unreachable() {
        let http = reqwest::Client::new();
        // Port 1 is reserved and will refuse the connection immediately.
        let result = probe_data_plane(&http, Some("http://127.0.0.1:1"), None).await;
        assert_eq!(result, (Some(false), None));
    }
}
