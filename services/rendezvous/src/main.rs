use clap::Parser;
use rendezvous::{build_router, dev_key_hierarchy, AppState, RateLimiter, RelayPoolEntry};
use std::sync::{Arc, Mutex};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:9000")]
    bind: String,
    #[arg(long, default_value = "deploy/local/relay-pool.json")]
    pool_file: String,
    #[arg(long, default_value_t = 5)]
    subset_size: usize,
    #[arg(long, default_value_t = 900)]
    ttl_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let args = Args::parse();

    let pool_json = std::fs::read_to_string(&args.pool_file).unwrap_or_else(|_| "[]".to_string());
    let pool: Vec<RelayPoolEntry> = serde_json::from_str(&pool_json)?;

    let (root_pub, bundle_key, bundle_cert, release_cert) = dev_key_hierarchy();
    tracing::info!(
        root_public_key = %hex::encode(root_pub.0),
        "generated ephemeral dev signing hierarchy (NOT for production — see docs/DEPLOYMENT.md)"
    );

    let state = Arc::new(AppState {
        bundle_key,
        bundle_key_cert: bundle_cert,
        release_key_cert: release_cert,
        pool,
        subset_size: args.subset_size,
        ttl_secs: args.ttl_secs,
        rate_limiter: Mutex::new(RateLimiter::new(20.0, 5.0)),
    });

    // The root public key must be distributed to clients out of band
    // (baked into the client build in a real deployment). For local dev,
    // write it next to the pool file so client.toml tooling can pick it up.
    std::fs::write(
        format!("{}.root_pub", args.pool_file),
        hex::encode(root_pub.0),
    )?;

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!(bind = %args.bind, "rendezvous listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
