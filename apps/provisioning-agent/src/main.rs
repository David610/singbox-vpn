mod config;
mod dispatch;
mod health_probe;
mod stats;
mod telemetry;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use worker_client::WorkerClient;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const TRAFFIC_INTERVAL: Duration = Duration::from_secs(15);
const REPORT_BACKOFF_MAX_SECS: u64 = 30;

// `version` makes `--version` print "vpn-provisioning-agent <x.y.z>", the
// shape deploy/lib/binary-version-check.sh verifies for every shipped binary.
#[derive(Parser)]
#[command(name = "vpn-provisioning-agent", version)]
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
    // Never permit a bad config value to become a CPU-burning busy loop or a
    // multi-minute provisioning delay.
    let poll_interval = Duration::from_secs(cfg.poll_interval_secs.clamp(1, 60));
    let mut telemetry = telemetry::TelemetrySampler::new();
    let mut next_heartbeat = Instant::now();
    let mut next_traffic = Instant::now();

    if cfg.clash_api_url.is_none() {
        tracing::info!("clash_api_url not configured — traffic reporting disabled for this node");
    }

    loop {
        let now = Instant::now();

        if now >= next_heartbeat {
            let (probe_ok, probe_latency_ms) = health_probe::probe_data_plane(
                client.http(),
                cfg.clash_api_url.as_deref(),
                cfg.clash_api_secret.as_deref(),
            )
            .await;
            let payload = telemetry.collect(&cfg, probe_ok, probe_latency_ms);
            if let Err(err) = client.heartbeat(&payload).await {
                tracing::warn!(error = %err, "node heartbeat failed");
            }
            next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        }

        if now >= next_traffic {
            report_traffic_once(&cfg, &client).await;
            next_traffic = Instant::now() + TRAFFIC_INTERVAL;
        }

        match poll_once(&cfg, &client).await {
            Ok(true) => {
                // A job was processed. Immediately claim the next one instead
                // of sleeping for the idle poll interval; this lets a single
                // node drain a signup/renewal burst as fast as vpn-admin can
                // safely apply the jobs.
                continue;
            }
            Ok(false) => {
                tokio::time::sleep(poll_interval).await;
            }
            Err(err) => {
                tracing::error!(error = %err, "poll iteration failed");
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}

async fn report_traffic_once(cfg: &AgentConfig, client: &WorkerClient) {
    let Some(clash_url) = cfg.clash_api_url.as_deref() else {
        return;
    };

    let sample = match stats::read_traffic(
        client.http(),
        clash_url,
        cfg.clash_api_secret.as_deref(),
    )
    .await
    {
        Ok(sample) => sample,
        Err(err) => {
            tracing::warn!(error = %err, "reading sing-box traffic counters failed");
            return;
        }
    };

    if let Err(err) = client.report_traffic(&sample).await {
        tracing::warn!(error = %err, "reporting traffic sample failed");
        return;
    }

    tracing::debug!(
        bytes_up = sample.bytes_up,
        bytes_down = sample.bytes_down,
        connections_open = sample.connections_open,
        "reported traffic sample"
    );
}

/// Returns true when a job was claimed/processed and false when the queue was
/// empty. The caller uses this to drain bursts without an artificial sleep.
async fn poll_once(cfg: &AgentConfig, client: &WorkerClient) -> Result<bool> {
    let Some(job) = client.claim().await.context("claiming a job")? else {
        return Ok(false);
    };
    tracing::info!(job_id = job.id, job_type = %job.job_type, "claimed job");

    match dispatch::run_job(cfg, client, &job).await {
        Ok(result) => {
            // Once vpn-admin has changed state, never move on to another job
            // until the Worker acknowledges the result. Retrying the report
            // is safe; rerunning the side effect is not.
            report_complete_until_ack(client, &job, result).await;
            tracing::info!(job_id = job.id, "job completed");
        }
        Err(err) => {
            let message = err.to_string();
            tracing::error!(job_id = job.id, error = %message, "job failed after retries");
            report_fail_until_ack(client, &job, &message).await;
        }
    }

    Ok(true)
}

fn report_backoff(attempt: u32) -> Duration {
    let shift = attempt.min(5);
    Duration::from_secs((1_u64 << shift).min(REPORT_BACKOFF_MAX_SECS))
}

async fn report_complete_until_ack(client: &WorkerClient, job: &worker_client::Job, result: Value) {
    let mut attempt = 1_u32;
    loop {
        match client.complete(job.id, result.clone()).await {
            Ok(()) => return,
            Err(err) => {
                let delay = report_backoff(attempt);
                tracing::warn!(
                    job_id = job.id,
                    attempt,
                    retry_in_seconds = delay.as_secs(),
                    error = %err,
                    "reporting job completion failed; retrying without rerunning vpn-admin"
                );
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

async fn report_fail_until_ack(client: &WorkerClient, job: &worker_client::Job, message: &str) {
    let mut attempt = 1_u32;
    loop {
        match client.fail(job.id, message).await {
            Ok(()) => return,
            Err(err) => {
                let delay = report_backoff(attempt);
                tracing::warn!(
                    job_id = job.id,
                    attempt,
                    retry_in_seconds = delay.as_secs(),
                    error = %err,
                    "reporting job failure failed; retrying"
                );
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_backoff_is_capped() {
        assert_eq!(report_backoff(1), Duration::from_secs(2));
        assert_eq!(report_backoff(2), Duration::from_secs(4));
        assert_eq!(report_backoff(10), Duration::from_secs(30));
    }
}
