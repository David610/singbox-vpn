use clap::Parser;
use rendezvous::{build_router, dev_key_hierarchy, AppState, RateLimiter, RelayPoolEntry};
use std::sync::{Arc, Mutex};

#[derive(serde::Deserialize)]
struct CertFile {
    public_key: crypto::PublicKey,
    cert: crypto::hierarchy::KeyCertificate,
}

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
    /// Directory produced by `vpn-keytool bundle-issue`, containing
    /// `bundle.key`, `bundle.cert.json`. Requires `--release-cert-file`
    /// too. Without this, an ephemeral dev hierarchy is generated on
    /// every boot — fine for local testing, not for production.
    #[arg(long)]
    key_dir: Option<String>,
    /// `release.cert.json` produced by `vpn-keytool release-issue`,
    /// needed alongside `--key-dir` so every issued bundle carries a
    /// verifiable chain to the pinned root.
    #[arg(long)]
    release_cert_file: Option<String>,
    /// Signed revocation list produced by `vpn-keytool revoke-issue`,
    /// served verbatim at `/v1/revocation-list` for clients to fetch and
    /// verify themselves.
    #[arg(long)]
    revocation_list_file: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let args = Args::parse();

    let pool_json = std::fs::read_to_string(&args.pool_file).unwrap_or_else(|_| "[]".to_string());
    let pool: Vec<RelayPoolEntry> = serde_json::from_str(&pool_json)?;

    let (bundle_key, bundle_cert, release_cert) = match (&args.key_dir, &args.release_cert_file) {
        (Some(key_dir), Some(release_cert_file)) => {
            let key_dir = std::path::Path::new(key_dir);
            let bundle_key = crypto::KeyPair::load_from_file(&key_dir.join("bundle.key"))?;
            let bundle_cert_file: CertFile =
                serde_json::from_slice(&std::fs::read(key_dir.join("bundle.cert.json"))?)?;
            if bundle_cert_file.public_key != bundle_key.public_key() {
                anyhow::bail!("bundle.cert.json does not certify bundle.key's public key");
            }
            let release_cert_file: CertFile =
                serde_json::from_slice(&std::fs::read(release_cert_file)?)?;
            tracing::info!(
                bundle_public_key = %bundle_key.public_key().to_hex(),
                "rendezvous: loaded persisted bundle signing key"
            );
            (bundle_key, bundle_cert_file.cert, release_cert_file.cert)
        }
        (None, None) => {
            let (root_pub, bundle_key, bundle_cert, release_cert) = dev_key_hierarchy();
            tracing::warn!(
                root_public_key = %hex::encode(root_pub.0),
                "generated ephemeral dev signing hierarchy (NOT for production — see docs/DEPLOYMENT.md)"
            );
            // Local dev convenience only: write the root pub next to the
            // pool file so client.toml tooling can pick it up. A real
            // deployment pins the root public key into client builds out
            // of band instead (see docs/DEPLOYMENT.md).
            std::fs::write(
                format!("{}.root_pub", args.pool_file),
                hex::encode(root_pub.0),
            )?;
            (bundle_key, bundle_cert, release_cert)
        }
        _ => anyhow::bail!("--key-dir and --release-cert-file must be given together"),
    };

    let revocation_list_bytes = match &args.revocation_list_file {
        Some(path) => Some(std::fs::read(path)?),
        None => None,
    };

    let state = Arc::new(AppState {
        bundle_key,
        bundle_key_cert: bundle_cert,
        release_key_cert: release_cert,
        pool,
        subset_size: args.subset_size,
        ttl_secs: args.ttl_secs,
        rate_limiter: Mutex::new(RateLimiter::new(20.0, 5.0)),
        revocation_list_bytes,
    });

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
