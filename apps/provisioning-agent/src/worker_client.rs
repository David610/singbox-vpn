use crate::config::AgentConfig;
use crate::pacing::{parse_retry_after, ApiError, ControlPlaneGate};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value;

/// A job claimed from the Worker's /api/agent/claim endpoint. Field names
/// match that endpoint's response body exactly (see the vpn-web
/// provisioning-worker-api plan, Task 3) — job_type is one of
/// "CREATE_USER" | "SET_EXPIRY" | "CLEAR_EXPIRY" | "ENABLE_USER" |
/// "DISABLE_USER" | "ROTATE_SUBSCRIPTION_TOKEN" | "ROTATE_CREDENTIALS" |
/// "APPLY_NODE_REVISION" (Phase 6 — payload is `{"revision": N}`, never
/// the config content inline; the content is fetched separately via
/// `fetch_revision_config`).
#[derive(Clone, Deserialize)]
pub struct Job {
    pub id: i64,
    pub job_type: String,
    pub payload: Value,
    /// Opaque capability for this claim attempt. Deliberately omitted from Debug/logging.
    pub claim_token: String,
    /// RFC3339 hard deadline after which this claim must not report success.
    pub lease_expires_at: String,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("id", &self.id)
            .field("job_type", &self.job_type)
            .field("payload", &"[REDACTED]")
            .field("claim_token", &"[REDACTED]")
            .field("lease_expires_at", &self.lease_expires_at)
            .finish()
    }
}

impl Job {
    pub fn lease_expires_at(&self) -> Result<time::OffsetDateTime> {
        time::OffsetDateTime::parse(
            &self.lease_expires_at,
            &time::format_description::well_known::Rfc3339,
        )
        .context("claim lease_expires_at is not RFC3339")
    }

    pub fn lease_is_live(&self) -> Result<bool> {
        Ok(self.lease_expires_at()? > time::OffsetDateTime::now_utc())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportDisposition {
    Accepted,
    Terminal,
}

#[derive(Debug, Deserialize)]
struct ClaimResponse {
    job: Option<Job>,
}

/// Result of a conditional `GET /api/agent/authorizations`.
#[derive(Debug)]
pub enum AuthorizationsFetch {
    /// The server confirmed the caller's ETag is still current (304).
    NotModified,
    Fresh {
        body: Value,
        etag: Option<String>,
    },
}

/// Result of one `GET /api/agent/events` long poll.
#[derive(Debug, PartialEq, Eq)]
pub enum EventsOutcome {
    /// `reason` is "change", "resync" or "timeout"; `changed` names the
    /// channels ("jobs", "authorizations", "leases") that moved.
    Event {
        reason: String,
        changed: Vec<String>,
        cursor: String,
    },
    /// The server has no event channel (501): fall back to polling.
    Unavailable,
    /// Another connection from this node replaced this one (409).
    Superseded {
        retry_after: Option<std::time::Duration>,
    },
}

#[derive(Debug, Deserialize)]
struct EventsBody {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    changed: Vec<String>,
    #[serde(default)]
    cursor: String,
}

#[derive(Clone)]
pub struct WorkerClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    gate: ControlPlaneGate,
}

impl WorkerClient {
    pub fn new(cfg: &AgentConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("building reqwest client");
        Self {
            http,
            base_url: cfg.worker_url.clone(),
            api_key: cfg.agent_api_key.clone(),
            gate: ControlPlaneGate::new(
                cfg.control_plane.backoff_base(),
                cfg.control_plane.backoff_max(),
            ),
        }
    }

    #[cfg(test)]
    /// The shared circuit breaker, for the event task and for tests.
    pub fn gate(&self) -> &ControlPlaneGate {
        &self.gate
    }

    /// Sends a polling request through the shared gate: nothing leaves the
    /// node while the control plane is backing off, every outcome feeds the
    /// breaker, and any non-success becomes a typed [`ApiError`].
    async fn polling(
        &self,
        request: reqwest::RequestBuilder,
        what: &'static str,
    ) -> Result<reqwest::Response> {
        self.gate.check()?;
        let res = match request.send().await {
            Ok(res) => res,
            Err(err) => {
                self.gate.record_failure(None);
                return Err(anyhow::Error::new(err).context(format!("{what} request failed")));
            }
        };
        let status = res.status();
        if status.is_success() || status == StatusCode::NOT_MODIFIED {
            self.gate.record_success();
            return Ok(res);
        }
        let retry_after = parse_retry_after(res.headers());
        self.gate.record_failure(retry_after);
        Err(ApiError::Status {
            what,
            status: status.as_u16(),
            retry_after,
        }
        .into())
    }

    /// Feeds the breaker from a call that must never be blocked by it
    /// (job reports and job-time fetches own their retry policy).
    fn observe(&self, status: StatusCode, headers: &reqwest::header::HeaderMap) {
        if status.is_success() {
            self.gate.record_success();
        } else if status.as_u16() == 429 || status.is_server_error() {
            self.gate.record_failure(parse_retry_after(headers));
        }
    }

    /// Claims the oldest pending job for this agent's node, or None if
    /// nothing is pending right now.
    pub async fn claim(&self) -> Result<Option<Job>> {
        let res = self
            .polling(
                self.http
                    .post(format!("{}/api/agent/claim", self.base_url))
                    .bearer_auth(&self.api_key),
                "POST /api/agent/claim",
            )
            .await?;

        let parsed: ClaimResponse = res
            .json()
            .await
            .context("parsing /api/agent/claim response body")?;
        Ok(parsed.job)
    }

    /// Reports a job as successfully completed. `result` is whatever
    /// job-type-specific payload the Worker's /complete endpoint expects
    /// (see the vpn-web plan's Task 4) — e.g. for CREATE_USER,
    /// `{"vpn_user_id": ..., "subscription_url": ...}`.
    pub async fn complete(
        &self,
        job_id: i64,
        claim_token: &str,
        result: Value,
    ) -> Result<ReportDisposition> {
        let res = self
            .http
            .post(format!(
                "{}/api/agent/jobs/{job_id}/complete",
                self.base_url
            ))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "claim_token": claim_token, "result": result }))
            .send()
            .await
            .context("POST /api/agent/jobs/:id/complete request failed")?;
        self.observe(res.status(), res.headers());

        if res.status().is_success() {
            return Ok(ReportDisposition::Accepted);
        }
        if matches!(res.status().as_u16(), 401 | 403 | 404 | 409 | 410) {
            return Ok(ReportDisposition::Terminal);
        }
        {
            bail!(
                "POST /api/agent/jobs/{job_id}/complete returned {}",
                res.status()
            );
        }
    }

    /// Reports a job as failed. Delivery is owned by the bounded,
    /// independently backed-off report queue.
    pub async fn fail(
        &self,
        job_id: i64,
        claim_token: &str,
        error: &str,
    ) -> Result<ReportDisposition> {
        let res = self
            .http
            .post(format!("{}/api/agent/jobs/{job_id}/fail", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "claim_token": claim_token, "error": error }))
            .send()
            .await
            .context("POST /api/agent/jobs/:id/fail request failed")?;
        self.observe(res.status(), res.headers());

        if res.status().is_success() {
            return Ok(ReportDisposition::Accepted);
        }
        if matches!(res.status().as_u16(), 401 | 403 | 404 | 409 | 410) {
            return Ok(ReportDisposition::Terminal);
        }
        {
            bail!(
                "POST /api/agent/jobs/{job_id}/fail returned {}",
                res.status()
            );
        }
    }

    /// Sends a privacy-safe operational heartbeat.
    pub async fn heartbeat(&self, payload: &Value) -> Result<()> {
        self.polling(
            self.http
                .post(format!("{}/api/agent/heartbeat", self.base_url))
                .bearer_auth(&self.api_key)
                .json(payload),
            "POST /api/agent/heartbeat",
        )
        .await?;
        Ok(())
    }

    /// Fetches a declarative revision's config document (Phase 6:
    /// `APPLY_NODE_REVISION`). vpn-web's contract is deliberately
    /// schema-agnostic about `config`'s shape — this repo decides what
    /// it expects (see `dispatch::apply_node_revision`) and validates it
    /// once handed to `vpn-admin apply-revision`; this method just fetches
    /// the raw JSON `config` value node-scoped, same Bearer auth as every
    /// other agent endpoint.
    pub async fn fetch_revision_config(&self, revision: u64) -> Result<Value> {
        let res = self
            .http
            .get(format!("{}/api/agent/revision/{revision}", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("GET /api/agent/revision/:revision request failed")?;
        self.observe(res.status(), res.headers());

        if !res.status().is_success() {
            bail!(
                "GET /api/agent/revision/{revision} returned {}",
                res.status()
            );
        }

        let body: Value = res
            .json()
            .await
            .context("parsing /api/agent/revision/:revision response body")?;
        match body.get("config") {
            None | Some(Value::Null) => Err(anyhow!(
                "revision {revision} response body is missing (or has a null) \"config\" — \
                 the server has no config for this revision"
            )),
            Some(config) => Ok(config.clone()),
        }
    }

    /// Reports one traffic sample.
    ///
    /// Counters are sent as sing-box reports them — cumulative since its
    /// process started — and the Worker differences them against the
    /// previous sample. A failed report is therefore safe to drop rather
    /// than retry: the next one carries the same running total, so nothing
    /// is double-counted and nothing is lost but resolution.
    pub async fn report_traffic(&self, sample: &crate::stats::TrafficSample) -> Result<()> {
        self.polling(
            self.http
                .post(format!("{}/api/agent/traffic", self.base_url))
                .bearer_auth(&self.api_key)
                .json(&serde_json::json!({
                    "bytes_up": sample.bytes_up,
                    "bytes_down": sample.bytes_down,
                    "connections_open": sample.connections_open,
                    "sampled_at": time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .context("formatting sampled_at")?,
                })),
            "POST /api/agent/traffic",
        )
        .await?;
        Ok(())
    }

    /// ADR-0003 lease-pool reconciliation (`POST /api/agent/leases/sync`).
    /// The body can carry slot secrets; neither it nor the response is
    /// ever logged, and errors only name the status code.
    pub async fn sync_leases(&self, body: &Value) -> Result<Value> {
        let res = self
            .polling(
                self.http
                    .post(format!("{}/api/agent/leases/sync", self.base_url))
                    .bearer_auth(&self.api_key)
                    .json(body),
                "POST /api/agent/leases/sync",
            )
            .await?;
        res.json()
            .await
            .context("parsing /api/agent/leases/sync response body")
    }

    #[cfg(test)]
    /// Fetch this node's control-plane-owned external compatibility state.
    /// The response is never logged because it contains protocol secrets.
    pub async fn fetch_authorizations(&self) -> Result<Value> {
        match self.fetch_authorizations_conditional(None).await? {
            AuthorizationsFetch::Fresh { body, .. } => Ok(body),
            AuthorizationsFetch::NotModified => {
                bail!("GET /api/agent/authorizations returned 304 to an unconditional request")
            }
        }
    }

    /// Conditional form of the plain authorizations fetch: with the ETag of
    /// the snapshot already held, an unchanged state costs one tiny 304 and
    /// no server-side decryption.
    pub async fn fetch_authorizations_conditional(
        &self,
        etag: Option<&str>,
    ) -> Result<AuthorizationsFetch> {
        let mut request = self
            .http
            .get(format!("{}/api/agent/authorizations", self.base_url))
            .query(&[("schema", "2")])
            .bearer_auth(&self.api_key);
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        let res = self
            .polling(request, "GET /api/agent/authorizations")
            .await?;
        if res.status() == StatusCode::NOT_MODIFIED {
            return Ok(AuthorizationsFetch::NotModified);
        }
        let etag = res
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = res
            .json()
            .await
            .context("parsing /api/agent/authorizations response body")?;
        Ok(AuthorizationsFetch::Fresh { body, etag })
    }

    /// One long poll of the authenticated event channel. The response is a
    /// hint about WHICH state moved, never the state itself.
    pub async fn wait_events(&self, cursor: Option<&str>, wait_secs: u64) -> Result<EventsOutcome> {
        self.gate.check()?;
        let mut request = self
            .http
            .get(format!("{}/api/agent/events", self.base_url))
            .query(&[("wait", wait_secs.to_string())])
            .bearer_auth(&self.api_key)
            .timeout(std::time::Duration::from_secs(wait_secs + 15));
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        let res = match request.send().await {
            Ok(res) => res,
            Err(err) => {
                self.gate.record_failure(None);
                return Err(anyhow::Error::new(err).context("GET /api/agent/events request failed"));
            }
        };
        let status = res.status();
        if status.is_success() {
            self.gate.record_success();
            let body: EventsBody = res
                .json()
                .await
                .context("parsing /api/agent/events response body")?;
            return Ok(EventsOutcome::Event {
                reason: body.reason,
                changed: body.changed,
                cursor: body.cursor,
            });
        }
        match status.as_u16() {
            // The control plane answered; it just has no event channel. Not a failure.
            501 => Ok(EventsOutcome::Unavailable),
            409 => Ok(EventsOutcome::Superseded {
                retry_after: parse_retry_after(res.headers()),
            }),
            code => {
                let retry_after = parse_retry_after(res.headers());
                self.gate.record_failure(retry_after);
                Err(ApiError::Status {
                    what: "GET /api/agent/events",
                    status: code,
                    retry_after,
                }
                .into())
            }
        }
    }

    /// Acknowledges the exact v2 snapshot that was verified live. The body contains no
    /// authorization or protocol secret, and callers must never substitute a latest revision.
    pub async fn ack_external_authorizations(&self, snapshot_revision: u64) -> Result<()> {
        self.polling(
            self.http
                .post(format!("{}/api/agent/authorizations", self.base_url))
                .query(&[("schema", "2")])
                .bearer_auth(&self.api_key)
                .json(&serde_json::json!({
                    "schema_version": 2,
                    "snapshot_revision": snapshot_revision,
                })),
            "POST /api/agent/authorizations acknowledgement",
        )
        .await?;
        Ok(())
    }

    /// Exposes the shared HTTP client so the traffic poller reuses this
    /// agent's one connection pool and timeout policy rather than building
    /// a second client with different behaviour.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

#[cfg(test)]
mod external_authorization_tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(base_url: &str) -> WorkerClient {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("agent.toml");
        std::fs::write(
            &config,
            format!(
                "worker_url = {base_url:?}\nnode_id = \"node_test\"\nagent_api_key = \"test-key\"\nvpn_admin_binary = \"vpn-admin\"\nvpn_admin_config = \"deployment.toml\"\n"
            ),
        )
        .unwrap();
        WorkerClient::new(&AgentConfig::load(&config).unwrap())
    }

    #[tokio::test]
    async fn fetch_requests_v2_and_ack_contains_only_the_exact_revision() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "schema_version": 2,
                "snapshot_revision": 10,
                "authorizations": []
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .and(body_json(serde_json::json!({
                "schema_version": 2,
                "snapshot_revision": 10
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let client = client(&server.uri());
        client.fetch_authorizations().await.unwrap();
        client.ack_external_authorizations(10).await.unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn failed_ack_can_be_retried_with_the_same_exact_revision() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .and(body_json(serde_json::json!({
                "schema_version": 2,
                "snapshot_revision": 10
            })))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert!(client.ack_external_authorizations(10).await.is_err());
        server.verify().await;
        assert!(
            crate::pacing::is_gated(&client.ack_external_authorizations(10).await.unwrap_err()),
            "the failed ACK pauses polling calls until the backoff elapses"
        );
        client.gate().record_success(); // the backoff window has elapsed

        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .and(body_json(serde_json::json!({
                "schema_version": 2,
                "snapshot_revision": 10
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        client.ack_external_authorizations(10).await.unwrap();
        server.verify().await;
    }
}

#[cfg(test)]
mod claim_lease_tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(base_url: &str) -> WorkerClient {
        WorkerClient {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(40))
                .no_proxy()
                .build()
                .unwrap(),
            base_url: base_url.to_string(),
            api_key: "node-key".to_string(),
            gate: ControlPlaneGate::new(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(5),
            ),
        }
    }

    #[tokio::test]
    async fn claim_requires_and_parses_attempt_capability() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/claim"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"job": {
                "id": 7, "job_type": "ENABLE_USER", "payload": {},
                "claim_token": "secret-attempt-token", "lease_expires_at": "2999-01-01T00:00:00Z"
            }})))
            .mount(&server)
            .await;
        let job = client(&server.uri()).claim().await.unwrap().unwrap();
        assert_eq!(job.id, 7);
        assert!(job.lease_is_live().unwrap());
        assert!(!format!("{job:?}").contains("secret-attempt-token"));
    }

    #[tokio::test]
    async fn complete_and_fail_echo_the_claim_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/jobs/7/complete"))
            .and(body_json(
                serde_json::json!({"claim_token":"attempt-1","result":{"ok":true}}),
            ))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/agent/jobs/8/fail"))
            .and(body_json(
                serde_json::json!({"claim_token":"attempt-2","error":"failed safely"}),
            ))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert_eq!(
            client
                .complete(7, "attempt-1", serde_json::json!({"ok":true}))
                .await
                .unwrap(),
            ReportDisposition::Accepted
        );
        assert_eq!(
            client.fail(8, "attempt-2", "failed safely").await.unwrap(),
            ReportDisposition::Accepted
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn stale_gone_cancelled_and_unauthorized_are_terminal() {
        for status in [401, 403, 404, 409, 410] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let client = client(&server.uri());
            assert_eq!(
                client
                    .complete(1, "stale-token", Value::Null)
                    .await
                    .unwrap(),
                ReportDisposition::Terminal
            );
            assert_eq!(
                client.fail(1, "stale-token", "x").await.unwrap(),
                ReportDisposition::Terminal
            );
        }
    }

    #[tokio::test]
    async fn reclaimed_attempt_cannot_complete_but_new_attempt_can() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_json(
                serde_json::json!({"claim_token":"old","result":null}),
            ))
            .respond_with(ResponseTemplate::new(409))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_json(
                serde_json::json!({"claim_token":"new","result":null}),
            ))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert_eq!(
            client.complete(1, "old", Value::Null).await.unwrap(),
            ReportDisposition::Terminal
        );
        assert_eq!(
            client.complete(1, "new", Value::Null).await.unwrap(),
            ReportDisposition::Accepted
        );
    }

    #[tokio::test]
    async fn duplicate_terminal_reports_are_treated_as_accepted_when_server_is_idempotent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(204))
            .expect(4)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        for _ in 0..2 {
            assert_eq!(
                client.complete(1, "token", Value::Null).await.unwrap(),
                ReportDisposition::Accepted
            );
        }
        for _ in 0..2 {
            assert_eq!(
                client.fail(2, "token", "x").await.unwrap(),
                ReportDisposition::Accepted
            );
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn timeout_is_transient_and_expired_lease_is_rejected_locally() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(204).set_delay(std::time::Duration::from_millis(200)),
            )
            .mount(&server)
            .await;
        assert!(client(&server.uri())
            .complete(1, "token", Value::Null)
            .await
            .is_err());
        let expired: Job = serde_json::from_value(serde_json::json!({
            "id":1,"job_type":"x","payload":{},"claim_token":"secret",
            "lease_expires_at":"2000-01-01T00:00:00Z"
        }))
        .unwrap();
        assert!(!expired.lease_is_live().unwrap());
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(base_url: &str) -> WorkerClient {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("agent.toml");
        std::fs::write(
            &config,
            format!(
                "worker_url = {base_url:?}\nnode_id = \"node_test\"\nagent_api_key = \"test-key\"\nvpn_admin_binary = \"vpn-admin\"\nvpn_admin_config = \"deployment.toml\"\n[control_plane]\nbackoff_base_secs = 30\nbackoff_max_secs = 60\n"
            ),
        )
        .unwrap();
        WorkerClient::new(&AgentConfig::load(&config).unwrap())
    }

    #[tokio::test]
    async fn a_429_closes_the_gate_so_the_next_poll_sends_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/claim"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "40"))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        let first = client.claim().await.unwrap_err();
        assert_eq!(first.to_string(), "POST /api/agent/claim returned 429");
        for _ in 0..25 {
            let err = client.claim().await.unwrap_err();
            assert!(crate::pacing::is_gated(&err), "{err}");
        }
        // `expect(1)` verifies on drop that exactly one request reached the server.
    }

    #[tokio::test]
    async fn one_failing_endpoint_pauses_every_polling_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/heartbeat"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/agent/leases/sync"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(0)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert!(client.heartbeat(&serde_json::json!({})).await.is_err());
        assert!(crate::pacing::is_gated(
            &client
                .sync_leases(&serde_json::json!({}))
                .await
                .unwrap_err()
        ));
    }

    #[tokio::test]
    async fn a_success_reopens_the_gate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/claim"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "job": null })),
            )
            .mount(&server)
            .await;
        let client = client(&server.uri());
        client.gate().record_failure_at(
            std::time::Instant::now() - Duration::from_secs(120),
            None,
            0.0,
        );
        assert!(client.claim().await.unwrap().is_none());
        assert_eq!(client.gate().consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn job_reports_are_never_blocked_by_the_gate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/agent/jobs/9/complete"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        client.gate().record_failure(Some(Duration::from_secs(600)));
        let outcome = client
            .complete(9, "tok", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(outcome, ReportDisposition::Accepted);
        assert_eq!(
            client.gate().consecutive_failures(),
            0,
            "a delivered report proves the control plane is up"
        );
    }

    #[tokio::test]
    async fn a_connection_error_counts_as_a_failure() {
        let client = client("http://127.0.0.1:9");
        assert!(client.claim().await.is_err());
        assert_eq!(client.gate().consecutive_failures(), 1);
        assert!(crate::pacing::is_gated(&client.claim().await.unwrap_err()));
    }

    #[tokio::test]
    async fn conditional_fetch_sends_if_none_match_and_understands_304() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/authorizations"))
            .and(header("if-none-match", "\"abc\""))
            .respond_with(ResponseTemplate::new(304).insert_header("ETag", "\"abc\""))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        let outcome = client
            .fetch_authorizations_conditional(Some("\"abc\""))
            .await
            .unwrap();
        assert!(matches!(outcome, AuthorizationsFetch::NotModified));
        assert_eq!(
            client.gate().consecutive_failures(),
            0,
            "304 is a healthy answer"
        );
    }

    #[tokio::test]
    async fn conditional_fetch_returns_the_body_and_new_etag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/authorizations"))
            .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"v2\"").set_body_json(
                serde_json::json!({ "schema_version": 2, "snapshot_revision": 4, "authorizations": [] }),
            ))
            .mount(&server)
            .await;
        let client = client(&server.uri());
        match client.fetch_authorizations_conditional(None).await.unwrap() {
            AuthorizationsFetch::Fresh { body, etag } => {
                assert_eq!(etag.as_deref(), Some("\"v2\""));
                assert_eq!(body["snapshot_revision"], 4);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_report_changes_with_the_cursor_round_trip() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .and(query_param("cursor", "c1"))
            .and(query_param("wait", "25"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "reason": "change", "changed": ["jobs"], "cursor": "c2"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert_eq!(
            client.wait_events(Some("c1"), 25).await.unwrap(),
            EventsOutcome::Event {
                reason: "change".into(),
                changed: vec!["jobs".into()],
                cursor: "c2".into()
            }
        );
    }

    #[tokio::test]
    async fn events_501_is_unavailable_not_a_failure_and_409_is_superseded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(501))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(409).insert_header("Retry-After", "5"))
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert_eq!(
            client.wait_events(None, 25).await.unwrap(),
            EventsOutcome::Unavailable
        );
        assert_eq!(client.gate().consecutive_failures(), 0);
        assert_eq!(
            client.wait_events(None, 25).await.unwrap(),
            EventsOutcome::Superseded {
                retry_after: Some(Duration::from_secs(5))
            }
        );
    }

    #[tokio::test]
    async fn events_5xx_trips_the_gate_like_any_other_poll() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "30"))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri());
        assert!(client.wait_events(None, 25).await.is_err());
        assert!(crate::pacing::is_gated(
            &client.wait_events(None, 25).await.unwrap_err()
        ));
    }
}
