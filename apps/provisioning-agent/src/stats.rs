use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// One reading of sing-box's traffic counters.
///
/// `bytes_up` / `bytes_down` are CUMULATIVE since the sing-box process
/// started, and are reported to the Worker as-is. Sending totals rather than
/// deltas is what makes a dropped report harmless: the next successful one
/// still carries the true running total, so a failed POST costs resolution
/// but never bytes. The Worker differences them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficSample {
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub connections_open: u32,
}

/// sing-box's Clash API `/connections` response. Only the three fields this
/// agent needs are modelled; the rest of the payload (per-connection detail,
/// memory) is ignored.
#[derive(Debug, Deserialize)]
struct ConnectionsResponse {
    #[serde(rename = "uploadTotal")]
    upload_total: u64,
    #[serde(rename = "downloadTotal")]
    download_total: u64,
    #[serde(default)]
    connections: Vec<serde_json::Value>,
}

/// Reads sing-box's traffic counters over its Clash API.
///
/// These are per-NODE totals. sing-box's per-user statistics live behind its
/// v2ray_api, which official builds do not compile in
/// (release/DEFAULT_BUILD_TAGS for v1.13.19 lists with_clash_api but not
/// with_v2ray_api, and the binary rejects the config outright). The Clash
/// API that IS available carries no user attribution — a connection from a
/// named VLESS user reports only source/destination/network — so per-user
/// accounting would require running a custom sing-box build. See
/// docs/TRAFFIC_ACCOUNTING.md.
pub async fn read_traffic(
    http: &reqwest::Client,
    clash_api_url: &str,
    secret: Option<&str>,
) -> Result<TrafficSample> {
    let url = format!("{}/connections", clash_api_url.trim_end_matches('/'));
    let mut request = http.get(&url);
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

    let parsed: ConnectionsResponse = res
        .json()
        .await
        .with_context(|| format!("parsing {url} response body"))?;

    Ok(TrafficSample {
        bytes_up: parsed.upload_total,
        bytes_down: parsed.download_total,
        // Clash reports open connections only; closed ones have already been
        // folded into the totals above, so this is a liveness gauge rather
        // than an accounting input.
        connections_open: u32::try_from(parsed.connections.len()).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Result<ConnectionsResponse> {
        Ok(serde_json::from_str(body)?)
    }

    #[test]
    fn parses_a_real_clash_api_payload() {
        // Captured verbatim from sing-box 1.13.19's /connections.
        let parsed =
            parse(r#"{"connections":[],"downloadTotal":5345,"memory":2899968,"uploadTotal":841}"#)
                .unwrap();
        assert_eq!(parsed.upload_total, 841);
        assert_eq!(parsed.download_total, 5345);
        assert_eq!(parsed.connections.len(), 0);
    }

    #[test]
    fn counts_open_connections() {
        let parsed = parse(
            r#"{"connections":[{"id":"a"},{"id":"b"}],"downloadTotal":1,"uploadTotal":2,"memory":0}"#,
        )
        .unwrap();
        assert_eq!(parsed.connections.len(), 2);
    }

    #[test]
    fn tolerates_a_payload_with_no_connections_key() {
        // The field is defaulted rather than required: an idle instance
        // must not make the whole sample unparseable.
        let parsed = parse(r#"{"downloadTotal":10,"uploadTotal":20,"memory":0}"#).unwrap();
        assert_eq!(parsed.connections.len(), 0);
        assert_eq!(parsed.download_total, 10);
    }

    #[test]
    fn rejects_a_payload_missing_the_counters() {
        // Better to fail the sample than to report zero bytes, which the
        // Worker would read as a counter reset.
        assert!(parse(r#"{"connections":[]}"#).is_err());
    }

    #[test]
    fn handles_counters_beyond_u32() {
        let parsed = parse(r#"{"downloadTotal":9007199254740993,"uploadTotal":5000000000,"memory":0}"#)
            .unwrap();
        assert_eq!(parsed.download_total, 9_007_199_254_740_993);
        assert_eq!(parsed.upload_total, 5_000_000_000);
    }

    // ---- Level 2 (integration): a real sing-box process -----------------
    //
    // The cases above parse a payload captured by hand, which proves the
    // deserializer matched what was captured — not that it still matches
    // what sing-box emits. These drive the real binary, so a field rename
    // upstream fails here instead of silently reporting zero bytes forever,
    // which the Worker would record as a counter reset.
    //
    // They skip cleanly when sing-box is absent, matching this repo's other
    // interop tests: CI without the binary should not run them, not fail.

    use std::io::Write;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

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

    async fn wait_readable(
        http: &reqwest::Client,
        base: &str,
        secret: &str,
    ) -> Option<TrafficSample> {
        for _ in 0..40 {
            if let Ok(sample) = read_traffic(http, base, Some(secret)).await {
                return Some(sample);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        None
    }

    #[tokio::test]
    async fn reads_counters_from_a_real_singbox_clash_api() {
        if !singbox_available() {
            eprintln!("skipping: sing-box not on PATH");
            return;
        }
        // Ports above this host's ip_local_port_range so an ephemeral
        // socket cannot already hold them.
        let secret = "interop-secret";
        let Some(_sb) = start_singbox(39_701, 39_702, secret) else {
            eprintln!("skipping: could not start sing-box");
            return;
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("building http client");

        let sample = wait_readable(&http, "http://127.0.0.1:39701", secret)
            .await
            .expect("sing-box Clash API never became readable");

        // A freshly started instance has carried nothing. What matters is
        // that the fields parsed at all — an upstream rename surfaces as a
        // parse error in wait_readable, not as zeroes here.
        assert_eq!(sample.connections_open, 0);
        assert_eq!(sample.bytes_up, 0);
        assert_eq!(sample.bytes_down, 0);
    }

    #[tokio::test]
    async fn rejects_a_wrong_clash_api_secret() {
        if !singbox_available() {
            eprintln!("skipping: sing-box not on PATH");
            return;
        }
        let Some(_sb) = start_singbox(39_703, 39_704, "correct-secret") else {
            eprintln!("skipping: could not start sing-box");
            return;
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("building http client");
        let base = "http://127.0.0.1:39703";

        // Confirm it is up with the right secret first, so the failure
        // below is about authentication and not a slow start.
        wait_readable(&http, base, "correct-secret")
            .await
            .expect("sing-box never became readable with the correct secret");

        assert!(
            read_traffic(&http, base, Some("wrong-secret")).await.is_err(),
            "a wrong Clash API secret must fail rather than silently reporting \
             zeroes, which the Worker would record as a counter reset"
        );
    }
}
