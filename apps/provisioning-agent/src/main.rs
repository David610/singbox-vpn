mod config;
mod dispatch;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use std::path::PathBuf;
use std::time::Duration;
use worker_client::WorkerClient;

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

    loop {
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
            client
                .complete(job.id, result)
                .await
                .context("reporting job completion")?;
            tracing::info!(job_id = job.id, "job completed");
        }
        Err(err) => {
            let message = err.to_string();
            tracing::error!(job_id = job.id, error = %message, "job failed after retries");
            client
                .fail(job.id, &message)
                .await
                .context("reporting job failure")?;
        }
    }
    Ok(())
}
