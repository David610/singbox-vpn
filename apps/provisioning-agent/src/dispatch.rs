use crate::config::AgentConfig;
use crate::worker_client::Job;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::process::Command;

const MAX_ATTEMPTS: u32 = 3;
const RETRY_BACKOFF: Duration = Duration::from_secs(2);
const VPN_ADMIN_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs the vpn-admin subcommand for one job, retrying transient failures
/// up to MAX_ATTEMPTS times before giving up. Returns the result payload
/// to report to the Worker's /complete endpoint.
pub async fn run_job(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let mut last_err = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match run_job_once(cfg, job).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                tracing::warn!(job_id = job.id, attempt, error = %err, "vpn-admin invocation failed");
                last_err = Some(err);
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(RETRY_BACKOFF).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("job failed with no recorded error")))
}

async fn run_job_once(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    match job.job_type.as_str() {
        "CREATE_USER" => create_user(cfg, job).await,
        "SET_EXPIRY" => set_expiry(cfg, job).await,
        "ENABLE_USER" => enable_or_disable(cfg, job, "enable").await,
        "DISABLE_USER" => enable_or_disable(cfg, job, "disable").await,
        "ROTATE_SUBSCRIPTION_TOKEN" => rotate_token(cfg, job).await,
        other => bail!("unknown job_type {other:?} (job {})", job.id),
    }
}

/// Every vpn-admin invocation goes through this one helper, so the
/// binary path / config path / working-directory setup happens in
/// exactly one place.
fn vpn_admin_command(cfg: &AgentConfig) -> Command {
    let mut cmd = Command::new(&cfg.vpn_admin_binary);
    cmd.arg("--config").arg(&cfg.vpn_admin_config);
    cmd
}

fn payload_str<'a>(job: &'a Job, key: &str) -> Result<&'a str> {
    job.payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("job {} payload missing string field {key:?}", job.id))
}

/// Job payloads carry RFC3339 timestamps (e.g. "2027-01-01T00:00:00Z",
/// as written by the Stripe webhook's `new Date(...).toISOString()`);
/// vpn-admin's --expires-at takes unix seconds.
fn payload_expires_at_unix(job: &Job) -> Result<i64> {
    let raw = payload_str(job, "expires_at")?;
    let parsed = OffsetDateTime::parse(raw, &Rfc3339)
        .with_context(|| format!("parsing expires_at {raw:?} as RFC3339 (job {})", job.id))?;
    Ok(parsed.unix_timestamp())
}

async fn create_user(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let user_id = payload_str(job, "user_id")?;
    let expires_at = payload_expires_at_unix(job)?;

    let output = tokio::time::timeout(
        VPN_ADMIN_TIMEOUT,
        vpn_admin_command(cfg)
            .args([
                "user",
                "create",
                "--name",
                user_id,
                "--expires-at",
                &expires_at.to_string(),
                "--json",
            ])
            .output(),
    )
    .await
    .context("vpn-admin command timed out after 60s")?
    .context("spawning vpn-admin user create")?;
    let parsed = parse_json_output(&output, "user create")?;

    let vpn_user_id = parsed
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing id"))?;
    let subscription_url = parsed
        .get("subscription_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing subscription_url"))?;

    Ok(serde_json::json!({
        "vpn_user_id": vpn_user_id,
        "subscription_url": subscription_url,
    }))
}

async fn set_expiry(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;
    let expires_at = payload_expires_at_unix(job)?;

    let output = tokio::time::timeout(
        VPN_ADMIN_TIMEOUT,
        vpn_admin_command(cfg)
            .args([
                "user",
                "set-expiry",
                vpn_user_id,
                "--expires-at",
                &expires_at.to_string(),
            ])
            .output(),
    )
    .await
    .context("vpn-admin command timed out after 60s")?
    .context("spawning vpn-admin user set-expiry")?;
    require_success(&output, "user set-expiry")?;
    Ok(serde_json::json!({}))
}

async fn enable_or_disable(cfg: &AgentConfig, job: &Job, subcommand: &str) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let output = tokio::time::timeout(
        VPN_ADMIN_TIMEOUT,
        vpn_admin_command(cfg)
            .args(["user", subcommand, vpn_user_id])
            .output(),
    )
    .await
    .context("vpn-admin command timed out after 60s")?
    .with_context(|| format!("spawning vpn-admin user {subcommand}"))?;
    require_success(&output, &format!("user {subcommand}"))?;
    Ok(serde_json::json!({}))
}

async fn rotate_token(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let output = tokio::time::timeout(
        VPN_ADMIN_TIMEOUT,
        vpn_admin_command(cfg)
            .args(["user", "rotate-token", vpn_user_id, "--json"])
            .output(),
    )
    .await
    .context("vpn-admin command timed out after 60s")?
    .context("spawning vpn-admin user rotate-token")?;
    let parsed = parse_json_output(&output, "user rotate-token")?;

    let subscription_url = parsed
        .get("subscription_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user rotate-token --json output missing subscription_url"))?;

    Ok(serde_json::json!({ "subscription_url": subscription_url }))
}

// Neither vpn-admin's stdout nor stderr is included verbatim in error
// messages here: on failure they may contain a real subscription-URL
// credential (a known pre-existing vpn-admin bug can emit one mixed
// with unexpected prose on malformed --json output), and these error
// messages eventually flow into plaintext storage and an operator
// email alert via the Worker's /fail endpoint. Only lengths/descriptions
// are included, which is enough to diagnose without leaking secrets.
fn require_success(output: &std::process::Output, what: &str) -> Result<()> {
    if !output.status.success() {
        bail!(
            "vpn-admin {what} exited with {} ({} bytes of stderr)",
            output.status,
            output.stderr.len()
        );
    }
    Ok(())
}

fn parse_json_output(output: &std::process::Output, what: &str) -> Result<Value> {
    require_success(output, what)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).with_context(|| {
        format!(
            "parsing vpn-admin {what} --json output ({} bytes): does not look like valid JSON",
            stdout.len()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker_client::Job;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    fn job(job_type: &str, payload: Value) -> Job {
        Job {
            id: 1,
            job_type: job_type.to_string(),
            payload,
        }
    }

    #[test]
    fn payload_str_reads_a_present_string_field() {
        let j = job("SET_EXPIRY", serde_json::json!({"vpn_user_id": "abc123"}));
        assert_eq!(payload_str(&j, "vpn_user_id").unwrap(), "abc123");
    }

    #[test]
    fn payload_str_errors_on_missing_field() {
        let j = job("SET_EXPIRY", serde_json::json!({}));
        assert!(payload_str(&j, "vpn_user_id").is_err());
    }

    #[test]
    fn payload_expires_at_unix_parses_rfc3339() {
        let j = job(
            "SET_EXPIRY",
            serde_json::json!({"expires_at": "2027-01-01T00:00:00Z"}),
        );
        // 2027-01-01T00:00:00Z is a fixed, known unix timestamp.
        assert_eq!(payload_expires_at_unix(&j).unwrap(), 1_798_761_600);
    }

    #[test]
    fn payload_expires_at_unix_errors_on_malformed_timestamp() {
        let j = job("SET_EXPIRY", serde_json::json!({"expires_at": "not-a-date"}));
        assert!(payload_expires_at_unix(&j).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn parse_json_output_errors_on_nonzero_exit() {
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(1i32.wrapping_shl(8)),
            stdout: vec![],
            stderr: b"boom".to_vec(),
        };
        // ExitStatusExt is unix-only; this test module compiles only
        // where that's available (matches this repo's existing pattern
        // of #[cfg(unix)] for exit-status-construction tests elsewhere).
        assert!(parse_json_output(&output, "test").is_err());
    }
}
