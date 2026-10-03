use crate::config::AgentConfig;
use crate::worker_client::{Job, WorkerClient};
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
pub async fn run_job(cfg: &AgentConfig, client: &WorkerClient, job: &Job) -> Result<Value> {
    let mut last_err = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match run_job_once(cfg, client, job).await {
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

async fn run_job_once(cfg: &AgentConfig, client: &WorkerClient, job: &Job) -> Result<Value> {
    match job.job_type.as_str() {
        "CREATE_USER" => create_user(cfg, job).await,
        "SET_EXPIRY" => set_expiry(cfg, job).await,
        "CLEAR_EXPIRY" => clear_expiry(cfg, job).await,
        "ENABLE_USER" => enable_or_disable(cfg, job, "enable").await,
        "DISABLE_USER" => enable_or_disable(cfg, job, "disable").await,
        "ROTATE_SUBSCRIPTION_TOKEN" => rotate_token(cfg, job).await,
        "ROTATE_CREDENTIALS" => rotate_credentials(cfg, job).await,
        "APPLY_NODE_REVISION" => apply_node_revision(cfg, client, job).await,
        other => bail!("unknown job_type {other:?} (job {})", job.id),
    }
}

fn payload_u64(job: &Job, key: &str) -> Result<u64> {
    job.payload.get(key).and_then(Value::as_u64).ok_or_else(|| {
        anyhow!(
            "job {} payload missing non-negative integer field {key:?}",
            job.id
        )
    })
}

/// Phase 6 (fleet platform): fetches the revision's config document from
/// vpn-web, writes it to a private temp file, and hands that path to
/// `vpn-admin apply-revision`. The fetch happens HERE (in the agent, which
/// already holds the HTTP client/auth for every other Worker call) rather
/// than inside `vpn-admin`, which is — and stays — a purely local-file CLI
/// with no HTTP client dependency of its own (see
/// `docs/FLEET_PLATFORM_PLAN.md` §Phase 6 and `apps/admin/Cargo.toml`,
/// which has no HTTP crate). This keeps the existing division of
/// responsibility intact: the agent talks to vpn-web, `vpn-admin` only
/// ever touches local state.
async fn apply_node_revision(cfg: &AgentConfig, client: &WorkerClient, job: &Job) -> Result<Value> {
    let revision = payload_u64(job, "revision")?;
    let config = client
        .fetch_revision_config(revision)
        .await
        .with_context(|| format!("fetching revision {revision} config (job {})", job.id))?;

    // The fetched document carries per-user secrets (vless_uuid,
    // hysteria2_password, etc. — see the config-shape decision in
    // `worker_client::WorkerClient::fetch_revision_config`'s doc comment),
    // so it must never land on disk with the process umask's default
    // (potentially world/group-readable) permissions, even briefly.
    // `tempfile`'s directory/file permissions are not guaranteed
    // restrictive across platforms, so set them explicitly here rather
    // than relying on the umask, matching `write_secret_file`'s intent on
    // the `vpn-admin` side of this same pipeline.
    let tmp_dir = tempfile::Builder::new()
        .prefix("vpn-revision-")
        .tempdir()
        .context("creating a temp dir for the fetched revision document")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp_dir.path(), std::fs::Permissions::from_mode(0o700))
            .context("restricting permissions on the revision temp dir")?;
    }
    let input_path = tmp_dir.path().join("revision.json");
    let payload = serde_json::to_vec(&config).context("serializing fetched revision config")?;
    tokio::fs::write(&input_path, &payload)
        .await
        .with_context(|| {
            format!("writing fetched revision {revision} document to {input_path:?}")
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&input_path, std::fs::Permissions::from_mode(0o600))
            .await
            .context("restricting permissions on the fetched revision document")?;
    }

    let mut command = vpn_admin_command(cfg);
    command
        .args([
            "apply-revision",
            "--revision",
            &revision.to_string(),
            "--input",
        ])
        .arg(&input_path);
    let output = run_vpn_admin(command, VPN_ADMIN_TIMEOUT, "vpn-admin apply-revision").await?;
    require_success(&output, "apply-revision")?;
    Ok(serde_json::json!({ "revision": revision }))
}

/// Every vpn-admin invocation goes through this one helper, so the
/// binary path / config path / working-directory setup happens in
/// exactly one place.
pub(crate) fn vpn_admin_command(cfg: &AgentConfig) -> Command {
    let mut cmd = Command::new(&cfg.vpn_admin_binary);
    cmd.arg("--config").arg(&cfg.vpn_admin_config);
    cmd
}

/// Spawns `command`, waits up to `timeout` for it to exit, and returns its
/// captured output.
///
/// This is the ONLY place a vpn-admin child is spawned. Every call site
/// used to do `tokio::time::timeout(dur, command.output()).await`, which
/// on timeout just drops the `Output` future — the underlying
/// `tokio::process::Child` was never given `kill_on_drop`, so on timeout
/// the vpn-admin process (and anything it had spawned) kept running in
/// the background, unsupervised, and could still mutate node state
/// *after* the agent had already reported the job as failed. That is the
/// bug this function fixes: on timeout it terminates the whole process
/// group and reaps it before returning, so a caller that sees an `Err`
/// here has a proof, not just a hope, that nothing further will happen.
async fn run_vpn_admin(
    mut command: Command,
    timeout: Duration,
    context: &str,
) -> Result<std::process::Output> {
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    #[cfg(unix)]
    {
        // New process group (pgid = pid): lets a timeout kill vpn-admin
        // *and* any descendant it spawned, not just the direct child.
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("spawning {context}"))?;

    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => {
            let status = status.with_context(|| format!("waiting for {context}"))?;
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut out) = child.stdout.take() {
                use tokio::io::AsyncReadExt;
                let _ = out.read_to_end(&mut stdout).await;
            }
            if let Some(mut err) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = err.read_to_end(&mut stderr).await;
            }
            Ok(std::process::Output {
                status,
                stdout,
                stderr,
            })
        }
        Err(_) => {
            terminate_child(&mut child).await;
            bail!("{context} timed out after {timeout:?} and was terminated");
        }
    }
}

/// Terminates a timed-out child (and, on Unix, its whole process group)
/// and blocks until it has been reaped, so the caller can rely on the
/// process being dead — not merely signalled — once this returns.
#[cfg(unix)]
async fn terminate_child(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        // SAFETY: `pid` is this child's own pid, and it was spawned with
        // `process_group(0)` so its pgid equals its pid. `killpg` with a
        // valid, still-referenced pgid has no memory-safety implications.
        let rc = unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            // ESRCH just means it had already exited between the timeout
            // firing and us getting here; anything else is worth logging.
            if err.raw_os_error() != Some(libc::ESRCH) {
                tracing::warn!(pid, error = %err, "killpg on timed-out vpn-admin child failed");
            }
        }
    }
    // Reap so the process never lingers as a zombie. SIGKILL is
    // unblockable, so this should resolve almost immediately; the bound
    // is just so a pathological wait() can't wedge the agent forever.
    if tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .is_err()
    {
        tracing::error!(
            pid = child.id(),
            "timed-out vpn-admin child did not reap within 5s of SIGKILL"
        );
    }
}

#[cfg(not(unix))]
async fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
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
    payload_optional_expires_at_unix(job)?
        .ok_or_else(|| anyhow!("job {} payload missing string field \"expires_at\"", job.id))
}

fn payload_optional_expires_at_unix(job: &Job) -> Result<Option<i64>> {
    let Some(value) = job.payload.get("expires_at") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value.as_str().ok_or_else(|| {
        anyhow!(
            "job {} payload field \"expires_at\" is not a string",
            job.id
        )
    })?;
    let parsed = OffsetDateTime::parse(raw, &Rfc3339)
        .with_context(|| format!("parsing expires_at {raw:?} as RFC3339 (job {})", job.id))?;
    Ok(Some(parsed.unix_timestamp()))
}

async fn create_user(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let user_id = payload_str(job, "user_id")?;
    let expires_at = payload_optional_expires_at_unix(job)?;

    // A paid/trial entitlement supplies an expiry. An indefinite support
    // grant intentionally omits it, which vpn-admin represents as a user
    // with no expiry rather than by inventing a far-future timestamp.
    let mut command = vpn_admin_command(cfg);
    command.args(["user", "create", "--name", user_id]);
    if let Some(expires_at) = expires_at {
        command.arg("--expires-at").arg(expires_at.to_string());
    }
    command.arg("--json");

    let output = run_vpn_admin(command, VPN_ADMIN_TIMEOUT, "vpn-admin user create").await?;
    let parsed = parse_json_output(&output, "user create")?;

    let vpn_user_id = parsed
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing id"))?;
    let subscription_url = parsed
        .get("subscription_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing subscription_url"))?;
    // provisioning_url is the first-party /v1/provision/<token> endpoint
    // used by the Tamara client. It is present in the --json output
    // alongside subscription_url; include it in the completion payload so
    // vpn-web can store and serve the correct URL to each client type.
    let provisioning_url = parsed
        .get("provisioning_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user create --json output missing provisioning_url"))?;

    Ok(serde_json::json!({
        "vpn_user_id": vpn_user_id,
        "subscription_url": subscription_url,
        "provisioning_url": provisioning_url,
    }))
}

async fn set_expiry(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;
    let expires_at = payload_expires_at_unix(job)?;

    let mut command = vpn_admin_command(cfg);
    command.args([
        "user",
        "set-expiry",
        vpn_user_id,
        "--expires-at",
        &expires_at.to_string(),
    ]);
    let output = run_vpn_admin(command, VPN_ADMIN_TIMEOUT, "vpn-admin user set-expiry").await?;
    require_success(&output, "user set-expiry")?;
    Ok(serde_json::json!({}))
}

async fn clear_expiry(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let mut command = vpn_admin_command(cfg);
    command.args(["user", "clear-expiry", vpn_user_id]);
    let output = run_vpn_admin(command, VPN_ADMIN_TIMEOUT, "vpn-admin user clear-expiry").await?;
    require_success(&output, "user clear-expiry")?;
    Ok(serde_json::json!({}))
}

async fn rotate_credentials(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let mut command = vpn_admin_command(cfg);
    command.args(["user", "rotate-credentials", vpn_user_id]);
    let output = run_vpn_admin(
        command,
        VPN_ADMIN_TIMEOUT,
        "vpn-admin user rotate-credentials",
    )
    .await?;
    require_success(&output, "user rotate-credentials")?;
    Ok(serde_json::json!({}))
}

async fn enable_or_disable(cfg: &AgentConfig, job: &Job, subcommand: &str) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let mut command = vpn_admin_command(cfg);
    command.args(["user", subcommand, vpn_user_id]);
    let output = run_vpn_admin(
        command,
        VPN_ADMIN_TIMEOUT,
        &format!("vpn-admin user {subcommand}"),
    )
    .await?;
    require_success(&output, &format!("user {subcommand}"))?;
    Ok(serde_json::json!({}))
}

async fn rotate_token(cfg: &AgentConfig, job: &Job) -> Result<Value> {
    let vpn_user_id = payload_str(job, "vpn_user_id")?;

    let mut command = vpn_admin_command(cfg);
    command.args(["user", "rotate-token", vpn_user_id, "--json"]);
    let output = run_vpn_admin(command, VPN_ADMIN_TIMEOUT, "vpn-admin user rotate-token").await?;
    let parsed = parse_json_output(&output, "user rotate-token")?;

    let subscription_url = parsed
        .get("subscription_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user rotate-token --json output missing subscription_url"))?;
    let provisioning_url = parsed
        .get("provisioning_url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("user rotate-token --json output missing provisioning_url"))?;

    Ok(serde_json::json!({
        "subscription_url": subscription_url,
        "provisioning_url": provisioning_url,
    }))
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

pub(crate) fn parse_json_output(output: &std::process::Output, what: &str) -> Result<Value> {
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

    /// Phase 9 regression test (F0x: timed-out vpn-admin child mutates
    /// state after the agent already reported failure). Spawns a synthetic
    /// shell "operation" that sleeps well past the timeout and then
    /// touches a marker file — a stand-in for a real vpn-admin state
    /// mutation. Proves two things the pre-fix code could not: (1) the
    /// child is actually killed, not just abandoned, and (2) it stays
    /// dead — checked again after its original sleep would have elapsed,
    /// so a merely-abandoned-future bug (child kept running in the
    /// background) cannot pass this test by accident.
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_child_before_it_can_mutate_state() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("mutated");
        let marker_str = marker.to_string_lossy().to_string();

        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!("sleep 2 && touch '{marker_str}'"));

        let result = run_vpn_admin(command, Duration::from_millis(200), "synthetic op").await;
        assert!(
            result.is_err(),
            "expected the timeout to surface as an error"
        );
        assert!(
            !marker.exists(),
            "child must not have mutated state immediately after the timeout fires"
        );

        // Wait past the child's original sleep. If the fix only dropped an
        // abandoned future (the pre-fix bug) instead of actually killing
        // the process group, the shell would still be alive here and
        // would create the marker once its sleep elapsed.
        tokio::time::sleep(Duration::from_millis(2300)).await;
        assert!(
            !marker.exists(),
            "child must still be dead well after its original sleep duration \
             (proves termination, not merely an abandoned future)"
        );
    }

    fn job(job_type: &str, payload: Value) -> Job {
        Job {
            id: 1,
            job_type: job_type.to_string(),
            payload,
            claim_token: "test-claim-token".to_string(),
            lease_expires_at: "2999-01-01T00:00:00Z".to_string(),
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
    fn optional_expiry_allows_an_indefinite_create_user() {
        let j = job("CREATE_USER", serde_json::json!({"user_id": "user-1"}));
        assert_eq!(payload_optional_expires_at_unix(&j).unwrap(), None);
    }

    #[test]
    fn optional_expiry_still_rejects_a_non_string_value() {
        let j = job(
            "CREATE_USER",
            serde_json::json!({"user_id": "user-1", "expires_at": 123}),
        );
        assert!(payload_optional_expires_at_unix(&j).is_err());
    }

    #[test]
    fn payload_expires_at_unix_errors_on_malformed_timestamp() {
        let j = job(
            "SET_EXPIRY",
            serde_json::json!({"expires_at": "not-a-date"}),
        );
        assert!(payload_expires_at_unix(&j).is_err());
    }

    #[test]
    fn payload_u64_reads_a_present_non_negative_integer() {
        let j = job("APPLY_NODE_REVISION", serde_json::json!({"revision": 7}));
        assert_eq!(payload_u64(&j, "revision").unwrap(), 7);
    }

    #[test]
    fn payload_u64_errors_on_missing_field() {
        let j = job("APPLY_NODE_REVISION", serde_json::json!({}));
        assert!(payload_u64(&j, "revision").is_err());
    }

    #[test]
    fn payload_u64_errors_on_negative_value() {
        let j = job("APPLY_NODE_REVISION", serde_json::json!({"revision": -1}));
        assert!(payload_u64(&j, "revision").is_err());
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
