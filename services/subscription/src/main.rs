use anyhow::{Context, Result};
use clap::Parser;
use compat_config::deployment::DeploymentConfig;
use std::path::PathBuf;
use std::sync::Mutex;
use subscription::{standard_endpoints, AppState, RateLimiter};

#[derive(Parser)]
#[command(
    name = "subscription",
    about = "Compatibility subscription HTTP service"
)]
struct Cli {
    #[arg(long, default_value = "/etc/vpn/deployment.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let cfg = DeploymentConfig::load(&cli.config)
        .with_context(|| format!("loading deployment config from {:?}", cli.config))?;

    let public_key = std::fs::read_to_string(cfg.reality_public_key_file())
        .context("reality public key missing — run `vpn-admin init` on the server first")?
        .trim()
        .to_string();
    let short_id = std::fs::read_to_string(cfg.reality_dir().join("short_id.txt"))
        .context("reality short_id missing — run `vpn-admin init` on the server first")?
        .trim()
        .to_string();

    let endpoints = standard_endpoints(
        &cfg.public_host,
        cfg.reality.listen_port,
        cfg.hysteria2.listen_port,
        &public_key,
        &short_id,
        &cfg.reality.handshake_server,
    );

    let state = std::sync::Arc::new(AppState {
        users_file: cfg.users_file(),
        endpoints,
        rate_limiter: Mutex::new(RateLimiter::new(20.0, 0.5)),
    });

    // Loopback only (spec §8/§27) — a reverse proxy terminates public
    // HTTPS in front of this on `cfg.subscription.public_port`.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], cfg.subscription.listen_port));
    tracing::info!(%addr, "subscription service listening (loopback only)");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        subscription::build_router(state)
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
