use clap::Parser;
use client_daemon::build_engine;
use config::revocation::RevocationList;
use config::EndpointDescriptor;
use config::TrustRoot;
use rendezvous_client::{HttpSource, RendezvousClient};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:1080")]
    socks_bind: String,
    /// Static endpoint list (JSON array of EndpointDescriptor) — used
    /// when no rendezvous URL is given, for the simplest local dev loop.
    #[arg(long)]
    static_endpoints_file: Option<String>,
    #[arg(long)]
    rendezvous_url: Option<String>,
    #[arg(long)]
    rendezvous_root_pub_hex_file: Option<String>,
    #[arg(long, default_value = "/tmp/client-daemon-bundle-cache.json")]
    bundle_cache_path: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let args = Args::parse();

    let endpoints: Vec<EndpointDescriptor> = if let Some(url) = args.rendezvous_url {
        let root_pub_hex_file = args.rendezvous_root_pub_hex_file.ok_or_else(|| {
            anyhow::anyhow!("--rendezvous-root-pub-hex-file required with --rendezvous-url")
        })?;
        let root_hex = std::fs::read_to_string(&root_pub_hex_file)?;
        let root_bytes = hex::decode(root_hex.trim())?;
        let mut root_arr = [0u8; 32];
        root_arr.copy_from_slice(&root_bytes);
        let trust_root = TrustRoot {
            root_public_key: crypto::PublicKey(root_arr),
        };
        let source = HttpSource::new(url);
        let client = RendezvousClient::new(source, trust_root, args.bundle_cache_path);
        let revoked = RevocationList::empty();
        let payload = client.get_bundle(&revoked).await?;
        tracing::info!(
            count = payload.endpoints.len(),
            "client-daemon: got relay bundle from rendezvous"
        );
        payload.endpoints
    } else if let Some(file) = args.static_endpoints_file {
        let json = std::fs::read_to_string(file)?;
        serde_json::from_str(&json)?
    } else {
        anyhow::bail!("either --static-endpoints-file or --rendezvous-url must be given");
    };

    let engine = build_engine(endpoints);
    client_daemon::socks::run(&args.socks_bind, engine).await
}
