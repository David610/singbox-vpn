//! Diagnostic CLI (spec §37). This session implements the subset that
//! does not require a running daemon control-plane IPC (not built this
//! session — see TASKS.md): bundle verification, static transport
//! listing, and basic local-network diagnostics. A `client status`/
//! `client connect` command talking to a live `client-daemon` over a
//! control socket is the natural next addition once that IPC exists.

use clap::{Parser, Subcommand};
use common::UnixSeconds;
use config::revocation::RevocationList;
use config::{verify_bundle, SignedBundle, TrustRoot};
use std::net::ToSocketAddrs;

#[derive(Parser)]
#[command(name = "vpn-cli")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List transport families compiled into this build.
    Transports,
    /// Verify a signed relay bundle file against a trust root.
    ConfigVerify {
        #[arg(long)]
        bundle_file: String,
        #[arg(long)]
        root_pub_hex: String,
    },
    /// Basic local network diagnostics: can we resolve/reach anything at
    /// all locally? Distinguishes "local network is down" from
    /// "circumvention failed" per spec §39 — this command only checks the
    /// former; the latter requires the running daemon's engine state.
    Diagnostics,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Transports => {
            println!("direct-tls   family A: TCP + TLS 1.3 (rustls)");
            println!("noise-quic   family B: UDP + QUIC (quinn)");
        }
        Commands::ConfigVerify {
            bundle_file,
            root_pub_hex,
        } => {
            let bytes = std::fs::read(&bundle_file)?;
            let bundle: SignedBundle = serde_json::from_slice(&bytes)?;
            let root_bytes = hex::decode(root_pub_hex.trim())?;
            let mut root_arr = [0u8; 32];
            if root_bytes.len() != 32 {
                anyhow::bail!("root public key must be 32 bytes hex-encoded");
            }
            root_arr.copy_from_slice(&root_bytes);
            let trust_root = TrustRoot {
                root_public_key: crypto::PublicKey(root_arr),
            };
            let revoked = RevocationList::empty();
            match verify_bundle(&bundle, &trust_root, &revoked, UnixSeconds::now()) {
                Ok(payload) => {
                    println!(
                        "VALID: schema_version={}, {} endpoint(s), expires_at={}",
                        payload.schema_version,
                        payload.endpoints.len(),
                        payload.expires_at
                    );
                }
                Err(e) => {
                    println!("INVALID: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Diagnostics => {
            print!("local DNS resolution: ");
            match "localhost:0".to_socket_addrs() {
                Ok(_) => println!("ok"),
                Err(e) => println!(
                    "FAILED ({e}) — possible local network failure, not necessarily censorship"
                ),
            }
        }
    }
    Ok(())
}
