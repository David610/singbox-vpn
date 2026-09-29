mod config;
mod dispatch;
mod health_probe;
mod lease_pool;
mod protocol_probe;
mod report_queue;
mod stats;
mod telemetry;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use report_queue::ReportQueue;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use worker_client::WorkerClient;

const TRAFFIC_INTERVAL: Duration = Duration::from_secs(15);

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
    // Phase 8: /complete and /fail reports are delivered by an independent
    // background task so a wedged Worker endpoint can never block the rest
    // of this loop (heartbeat, health probe, next-job poll, lease expiry).
    let report_queue = ReportQueue::spawn(PathBuf::from(&cfg.report_queue_file), client.clone());
    // Never permit a bad config value to become a CPU-burning busy loop or a
    // multi-minute provisioning delay.
    let poll_interval = Duration::from_secs(cfg.poll_interval_secs.clamp(1, 60));
    let mut telemetry = telemetry::TelemetrySampler::new();
    let mut next_heartbeat = Instant::now();
    let mut next_traffic = Instant::now();
    let heartbeat_interval = Duration::from_secs(cfg.heartbeat_interval_secs.clamp(10, 60));
    let protocol_report: protocol_probe::SharedReport = Default::default();
    if cfg.protocol_probe.is_some() {
        tokio::spawn(protocol_probe::run_loop(
            cfg.clone(),
            protocol_report.clone(),
        ));
    }

    let mut lease_pool = match lease_pool::LeasePool::open(&cfg) {
        Ok(pool) => Some(pool),
        Err(err) => {
            // Never start without the persisted table: forgetting live
            // secrets would stop them from being rotated. Keep the rest of
            // the agent running and surface the problem loudly.
            tracing::error!(error = %err, "lease pool state unreadable; lease pool disabled until fixed");
            None
        }
    };

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
                cfg.clash_probe_outbound.as_deref(),
            )
            .await;
            let mut payload = telemetry.collect(&cfg, probe_ok, probe_latency_ms);
            if let Some(report) = protocol_probe::take_report(&protocol_report) {
                payload["protocol_probe"] = report;
            }
            if let Err(err) = client.heartbeat(&payload).await {
                tracing::warn!(error = %err, "node heartbeat failed");
            }
            next_heartbeat = Instant::now() + heartbeat_interval;
        }

        if now >= next_traffic {
            report_traffic_once(&cfg, &client).await;
            next_traffic = Instant::now() + TRAFFIC_INTERVAL;
        }

        if let Some(pool) = lease_pool.as_mut() {
            // Runs every iteration: expiry enforcement is bounded by the
            // poll interval, and does not depend on the control plane.
            if let Err(err) = pool.tick(&cfg, &client).await {
                tracing::warn!(error = %err, "lease pool tick failed");
            }
        }

        match poll_once(&cfg, &client, &report_queue).await {
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
async fn poll_once(
    cfg: &AgentConfig,
    client: &WorkerClient,
    report_queue: &ReportQueue,
) -> Result<bool> {
    let Some(job) = client.claim().await.context("claiming a job")? else {
        return Ok(false);
    };
    tracing::info!(job_id = job.id, job_type = %job.job_type, "claimed job");

    match dispatch::run_job(cfg, client, &job).await {
        Ok(result) => {
            // The side effect already happened; only the Worker's
            // acknowledgement is still pending. Hand it to the
            // independently-retrying report queue and move straight on to
            // the next job — a slow/wedged Worker /complete endpoint must
            // not stall provisioning for every other customer on this node
            // (Phase 8).
            if let Err(err) = report_queue.enqueue_complete(job.id, result) {
                tracing::error!(job_id = job.id, error = %err, "failed to enqueue job completion report");
            }
            tracing::info!(job_id = job.id, "job completed");
        }
        Err(err) => {
            let message = err.to_string();
            tracing::error!(job_id = job.id, error = %message, "job failed after retries");
            if let Err(queue_err) = report_queue.enqueue_fail(job.id, &message) {
                tracing::error!(job_id = job.id, error = %queue_err, "failed to enqueue job failure report");
            }
        }
    }

    Ok(true)
}
