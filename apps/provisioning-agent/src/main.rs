mod config;
mod dispatch;
mod telemetry;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use worker_client::WorkerClient;

const REPORT_MAX_ATTEMPTS: u32 = 3;
const REPORT_RETRY_BACKOFF: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "/etc/vpn/provisioning-agent.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cfg = AgentConfig::load(&cli.config)
        .with_context(|| format!("loading agent config from {:?}", cli.config))?;
    tracing::info!(node_id = %cfg.node_id, worker_url = %cfg.worker_url, "provisioning agent starting");

    let client = WorkerClient::new(&cfg);
    let poll_interval = Duration::from_secs(cfg.poll_interval_secs);
    let mut telemetry = telemetry::TelemetrySampler::new();
    let mut next_heartbeat = Instant::now();

    loop {
        if Instant::now() >= next_heartbeat {
            let payload = telemetry.collect(&cfg);
            if let Err(err) = client.heartbeat(&payload).await {
                tracing::warn!(error = %err, "node heartbeat failed");
            }
            next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        }

        if let Err(err) = poll_once(&cfg, &client).await {
            // A poll-loop-level error (Worker unreachable, auth failure,
            // etc) is logged and the loop continues — this agent has no
            // "give up" state, since the alternative (crashing) just
            // means systemd restarts it into the same situation. Job-
            // level failures are handled inside poll_once itself, via
            // client.fail(...), and never reach this branch.
            tracing::error!(error = %err, "poll iteration failed");
        }
        tokio::time::sleep(poll_interval).await;
    }
}

async fn poll_once(cfg: &AgentConfig, client: &WorkerClient) -> Result<()> {
    let Some(job) = client.claim().await.context("claiming a job")? else {
        return Ok(());
    };
    tracing::info!(job_id = job.id, job_type = %job.job_type, "claimed job");

    match dispatch::run_job(cfg, &job).await {
        Ok(result) => {
            if report_complete_with_retry(client, &job, result).await {
                tracing::info!(job_id = job.id, "job completed");
            } else {
                // The job itself succeeded (e.g. vpn-admin really did
                // create the VPN user) but we could not tell the Worker
                // after repeated retries. Do NOT propagate this as an
                // error — that would just send poll_once's caller back
                // around to the next claim immediately with the job
                // stuck `claimed` forever and no operator visibility.
                // Logging job id/type (never the result payload, which
                // may hold the one-time subscription_url) is the best
                // recovery signal we can leave behind.
                tracing::error!(
                    job_id = job.id,
                    job_type = %job.job_type,
                    "job succeeded but reporting completion to the Worker failed after retries; \
                     job remains claimed in the Worker's DB and needs manual recovery"
                );
            }
        }
        Err(err) => {
            let message = err.to_string();
            tracing::error!(job_id = job.id, error = %message, "job failed after retries");
            if !report_fail_with_retry(client, &job, &message).await {
                tracing::error!(
                    job_id = job.id,
                    job_type = %job.job_type,
                    "job failed and reporting that failure to the Worker also failed after \
                     retries; job remains claimed in the Worker's DB and needs manual recovery"
                );
            }
        }
    }
    Ok(())
}

/// Retries `client.complete` up to REPORT_MAX_ATTEMPTS times with a fixed
/// backoff between attempts. Returns true if the Worker acknowledged the
/// completion, false if every attempt failed (already logged by the
/// caller, which must not crash the poll loop over this — see finding #1
/// in the final review: a job that actually succeeded must never be lost
/// just because reporting it hit a transient error).
async fn report_complete_with_retry(
    client: &WorkerClient,
    job: &worker_client::Job,
    result: Value,
) -> bool {
    for attempt in 1..=REPORT_MAX_ATTEMPTS {
        match client.complete(job.id, result.clone()).await {
            Ok(()) => return true,
            Err(err) => {
                tracing::warn!(job_id = job.id, attempt, error = %err, "reporting job completion failed");
                if attempt < REPORT_MAX_ATTEMPTS {
                    tokio::time::sleep(REPORT_RETRY_BACKOFF).await;
                }
            }
        }
    }
    false
}

/// Same retry shape as report_complete_with_retry, for client.fail.
async fn report_fail_with_retry(
    client: &WorkerClient,
    job: &worker_client::Job,
    message: &str,
) -> bool {
    for attempt in 1..=REPORT_MAX_ATTEMPTS {
        match client.fail(job.id, message).await {
            Ok(()) => return true,
            Err(err) => {
                tracing::warn!(job_id = job.id, attempt, error = %err, "reporting job failure failed");
                if attempt < REPORT_MAX_ATTEMPTS {
                    tokio::time::sleep(REPORT_RETRY_BACKOFF).await;
                }
            }
        }
    }
    false
}
