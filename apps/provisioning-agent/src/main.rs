mod config;
mod worker_client;

use anyhow::{Context, Result};
use clap::Parser;
use config::AgentConfig;
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "/etc/vpn/provisioning-agent.toml")]
    config: PathBuf,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cfg = AgentConfig::load(&cli.config)
        .with_context(|| format!("loading agent config from {:?}", cli.config))?;
    tracing::info!(node_id = %cfg.node_id, worker_url = %cfg.worker_url, "provisioning agent starting");
    Ok(())
}
