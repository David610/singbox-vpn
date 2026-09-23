use crate::config::AgentConfig;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

/// A job claimed from the Worker's /api/agent/claim endpoint. Field names
/// match that endpoint's response body exactly (see the vpn-web
/// provisioning-worker-api plan, Task 3) — job_type is one of
/// "CREATE_USER" | "SET_EXPIRY" | "CLEAR_EXPIRY" | "ENABLE_USER" |
/// "DISABLE_USER" | "ROTATE_SUBSCRIPTION_TOKEN" | "ROTATE_CREDENTIALS".
#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: i64,
    pub job_type: String,
    pub payload: Value,
}

#[derive(Debug, Deserialize)]
struct ClaimResponse {
    job: Option<Job>,
}

pub struct WorkerClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
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
        }
    }

    /// Claims the oldest pending job for this agent's node, or None if
    /// nothing is pending right now.
    pub async fn claim(&self) -> Result<Option<Job>> {
        let res = self
            .http
            .post(format!("{}/api/agent/claim", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("POST /api/agent/claim request failed")?;

        if !res.status().is_success() {
            bail!("POST /api/agent/claim returned {}", res.status());
        }

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
    pub async fn complete(&self, job_id: i64, result: Value) -> Result<()> {
        let res = self
            .http
            .post(format!(
                "{}/api/agent/jobs/{job_id}/complete",
                self.base_url
            ))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "result": result }))
            .send()
            .await
            .context("POST /api/agent/jobs/:id/complete request failed")?;

        if !res.status().is_success() {
            bail!(
                "POST /api/agent/jobs/{job_id}/complete returned {}",
                res.status()
            );
        }
        Ok(())
    }

    /// Reports a job as failed. The Worker marks it `failed` and sends
    /// the operator alert — this agent does not retry beyond its own
    /// local 3-attempt backoff (see the poll loop in main.rs).
    pub async fn fail(&self, job_id: i64, error: &str) -> Result<()> {
        let res = self
            .http
            .post(format!("{}/api/agent/jobs/{job_id}/fail", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "error": error }))
            .send()
            .await
            .context("POST /api/agent/jobs/:id/fail request failed")?;

        if !res.status().is_success() {
            bail!(
                "POST /api/agent/jobs/{job_id}/fail returned {}",
                res.status()
            );
        }
        Ok(())
    }

    /// Sends a privacy-safe operational heartbeat.
    pub async fn heartbeat(&self, payload: &Value) -> Result<()> {
        let res = self
            .http
            .post(format!("{}/api/agent/heartbeat", self.base_url))
            .bearer_auth(&self.api_key)
            .json(payload)
            .send()
            .await
            .context("POST /api/agent/heartbeat request failed")?;

        if !res.status().is_success() {
            bail!("POST /api/agent/heartbeat returned {}", res.status());
        }
        Ok(())
    }

    /// Reports one traffic sample.
    ///
    /// Counters are sent as sing-box reports them — cumulative since its
    /// process started — and the Worker differences them against the
    /// previous sample. A failed report is therefore safe to drop rather
    /// than retry: the next one carries the same running total, so nothing
    /// is double-counted and nothing is lost but resolution.
    pub async fn report_traffic(&self, sample: &crate::stats::TrafficSample) -> Result<()> {
        let res = self
            .http
            .post(format!("{}/api/agent/traffic", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "bytes_up": sample.bytes_up,
                "bytes_down": sample.bytes_down,
                "connections_open": sample.connections_open,
                "sampled_at": time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .context("formatting sampled_at")?,
            }))
            .send()
            .await
            .context("POST /api/agent/traffic request failed")?;

        if !res.status().is_success() {
            bail!("POST /api/agent/traffic returned {}", res.status());
        }
        Ok(())
    }

    /// Exposes the shared HTTP client so the traffic poller reuses this
    /// agent's one connection pool and timeout policy rather than building
    /// a second client with different behaviour.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}
