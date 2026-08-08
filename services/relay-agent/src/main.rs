use clap::Parser;
use relay_agent::{serve_quic, serve_tls, Role};
use std::net::SocketAddr;
use transport_native::server::RelayIdentity;

#[derive(clap::ValueEnum, Clone, Debug)]
enum RoleArg {
    Combined,
    Egress,
    Ingress,
}

#[derive(Parser)]
struct Args {
    #[arg(long, value_enum, default_value = "combined")]
    role: RoleArg,
    #[arg(long)]
    bind_tls: Option<SocketAddr>,
    #[arg(long)]
    bind_quic: Option<SocketAddr>,
    /// Required when --role ingress: the egress hop's direct-tls address.
    #[arg(long)]
    next_hop_addr: Option<SocketAddr>,
    /// Required when --role ingress: hex sha256 of the egress relay's cert.
    #[arg(long)]
    next_hop_cert_sha256: Option<String>,
    #[arg(long, default_value = "relay")]
    identity_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_ansi(false).init();
    let args = Args::parse();

    let identity = RelayIdentity::generate(&args.identity_name);
    tracing::info!(
        cert_sha256 = %hex::encode(identity.cert_sha256),
        "relay-agent: generated ephemeral relay identity (see docs/DEPLOYMENT.md)"
    );

    let role = match args.role {
        RoleArg::Combined => Role::Combined,
        RoleArg::Egress => Role::Egress,
        RoleArg::Ingress => {
            let addr = args
                .next_hop_addr
                .ok_or_else(|| anyhow::anyhow!("--next-hop-addr required for --role ingress"))?;
            let hash_hex = args.next_hop_cert_sha256.ok_or_else(|| {
                anyhow::anyhow!("--next-hop-cert-sha256 required for --role ingress")
            })?;
            let hash_bytes = hex::decode(&hash_hex)?;
            let mut pinned = [0u8; 32];
            pinned.copy_from_slice(&hash_bytes);
            Role::Ingress {
                next_hop: transport_api::Endpoint {
                    id: common::EndpointId("egress".into()),
                    address: addr,
                    provider_tag: "dev".into(),
                    pinned_cert_sha256: pinned,
                },
            }
        }
    };

    let mut handles = Vec::new();
    if let Some(bind) = args.bind_tls {
        let identity_clone = RelayIdentity {
            cert_der: identity.cert_der.clone(),
            key_der: identity.key_der.clone_key(),
            cert_sha256: identity.cert_sha256,
        };
        let role = role.clone();
        handles.push(tokio::spawn(async move {
            serve_tls(bind, &identity_clone, role, Default::default()).await
        }));
    }
    if let Some(bind) = args.bind_quic {
        let identity_clone = RelayIdentity {
            cert_der: identity.cert_der.clone(),
            key_der: identity.key_der.clone_key(),
            cert_sha256: identity.cert_sha256,
        };
        handles.push(tokio::spawn(async move {
            serve_quic(bind, &identity_clone, role, Default::default()).await
        }));
    }

    if handles.is_empty() {
        anyhow::bail!("at least one of --bind-tls / --bind-quic is required");
    }

    for h in handles {
        h.await??;
    }
    Ok(())
}
