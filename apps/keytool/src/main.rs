//! Offline signing-ceremony tooling for the three-tier key hierarchy
//! (ADR-0008: offline root -> release key -> bundle-signing key).
//!
//! This binary is meant to be run on a machine that never touches a
//! network-connected process once the root key exists on it — ideally an
//! air-gapped machine, or at minimum one that is booted, used for a
//! ceremony, and wiped/powered off. It never opens a socket itself; every
//! subcommand only reads/writes local files. `root init` is the only
//! subcommand that should ever run on such a machine on an ongoing basis
//! — `release issue` also needs the root key, so it belongs there too.
//! `bundle issue` only needs the *release* key and can run on a separate,
//! less precious "release machine" per ADR-0008. `revoke issue` also only
//! needs the release key.
//!
//! Key files are written hex-encoded with mode 0600 (owner read/write
//! only) via `crypto::KeyPair::save_to_file`, which refuses to overwrite
//! an existing file and refuses to load a file whose permissions have
//! been loosened. Certificate/public-key files are not secret and are
//! written world-readable so they can be copied to online hosts freely.

use clap::{Parser, Subcommand};
use common::UnixSeconds;
use config::revocation::SignedRevocationList;
use config::TrustRoot;
use crypto::hierarchy::KeyCertificate;
use crypto::{KeyPair, PublicKey};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "vpn-keytool", about = "Signing-hierarchy ceremony tooling")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate the offline root key. Run this once, on the most
    /// isolated machine you have, and never let the resulting `root.key`
    /// touch a network-connected process again.
    RootInit {
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Issue (or rotate) a release key, signed by the root key.
    ReleaseIssue {
        #[arg(long)]
        root_key: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Issue (or rotate) a bundle-signing key, signed by the release key.
    /// This is the only key the always-online rendezvous process holds.
    BundleIssue {
        #[arg(long)]
        release_key: PathBuf,
        #[arg(long)]
        release_cert: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Issue a signed revocation list naming keys that must no longer be
    /// trusted (e.g. a bundle key rotated out after suspected compromise).
    /// Only needs the release key, per ADR-0008 — no offline root needed
    /// to revoke.
    RevokeIssue {
        #[arg(long)]
        release_key: PathBuf,
        #[arg(long)]
        release_cert: PathBuf,
        /// Hex-encoded public key(s) to revoke. Repeatable.
        #[arg(long = "revoke")]
        revoke: Vec<String>,
        /// Existing signed revocation list to carry forward entries from
        /// (e.g. re-issuing after an expiring cache, or adding one more
        /// key to an already-published list).
        #[arg(long)]
        existing: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Offline sanity check: does this release cert chain to this root,
    /// and does this bundle cert chain to that release key?
    VerifyChain {
        #[arg(long)]
        root_pub: PathBuf,
        #[arg(long)]
        release_cert: PathBuf,
        #[arg(long)]
        bundle_cert: Option<PathBuf>,
    },
}

/// On-disk form of a `KeyCertificate` plus the public key it certifies —
/// everything an online process needs, none of it secret.
#[derive(Serialize, Deserialize)]
struct CertFile {
    public_key: PublicKey,
    cert: KeyCertificate,
}

fn write_public_file<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::bail!("refusing to overwrite existing file: {path:?}");
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn read_cert_file(path: &Path) -> anyhow::Result<CertFile> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn parse_pubkey_hex(s: &str) -> anyhow::Result<PublicKey> {
    Ok(PublicKey::from_hex(s)?)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::RootInit { out_dir } => {
            std::fs::create_dir_all(&out_dir)?;
            let root = KeyPair::generate();
            let key_path = out_dir.join("root.key");
            root.save_to_file(&key_path)?;
            let pub_path = out_dir.join("root.pub");
            write_public_file(&pub_path, &root.public_key().to_hex())?;
            println!("root key written to {key_path:?} (mode 0600)");
            println!(
                "root public key ({pub_path:?}): {}",
                root.public_key().to_hex()
            );
            println!(
                "pin this public key into client builds out of band; never copy root.key off this machine"
            );
        }
        Commands::ReleaseIssue { root_key, out_dir } => {
            let root = KeyPair::load_from_file(&root_key)?;
            std::fs::create_dir_all(&out_dir)?;
            let release = KeyPair::generate();
            let cert = KeyCertificate::issue(&root, release.public_key(), UnixSeconds::now().0);

            let key_path = out_dir.join("release.key");
            release.save_to_file(&key_path)?;
            let cert_path = out_dir.join("release.cert.json");
            write_public_file(
                &cert_path,
                &CertFile {
                    public_key: release.public_key(),
                    cert,
                },
            )?;
            println!("release key written to {key_path:?} (mode 0600)");
            println!("release cert written to {cert_path:?}");
            println!("release public key: {}", release.public_key().to_hex());
        }
        Commands::BundleIssue {
            release_key,
            release_cert,
            out_dir,
        } => {
            let release = KeyPair::load_from_file(&release_key)?;
            let release_cert_file = read_cert_file(&release_cert)?;
            if release_cert_file.public_key != release.public_key() {
                anyhow::bail!(
                    "release-cert does not certify the public key of release-key; wrong file pair?"
                );
            }
            std::fs::create_dir_all(&out_dir)?;
            let bundle = KeyPair::generate();
            let cert = KeyCertificate::issue(&release, bundle.public_key(), UnixSeconds::now().0);

            let key_path = out_dir.join("bundle.key");
            bundle.save_to_file(&key_path)?;
            let cert_path = out_dir.join("bundle.cert.json");
            write_public_file(
                &cert_path,
                &CertFile {
                    public_key: bundle.public_key(),
                    cert,
                },
            )?;
            println!("bundle key written to {key_path:?} (mode 0600)");
            println!("bundle cert written to {cert_path:?}");
            println!("bundle public key: {}", bundle.public_key().to_hex());
            println!(
                "copy bundle.key + bundle.cert.json + release.cert.json to the rendezvous host"
            );
        }
        Commands::RevokeIssue {
            release_key,
            release_cert,
            revoke,
            existing,
            out,
        } => {
            let release = KeyPair::load_from_file(&release_key)?;
            let release_cert_file = read_cert_file(&release_cert)?;
            if release_cert_file.public_key != release.public_key() {
                anyhow::bail!(
                    "release-cert does not certify the public key of release-key; wrong file pair?"
                );
            }

            // Prior entries are carried forward by key bytes only; the
            // release key running this ceremony re-signs the whole list,
            // so the old list's own signature doesn't need re-checking —
            // it's local operator-trusted input, not untrusted network
            // data.
            let mut keys: Vec<PublicKey> = if let Some(existing_path) = &existing {
                let bytes = std::fs::read(existing_path)?;
                let prior: SignedRevocationList = serde_json::from_slice(&bytes)?;
                prior.revoked_keys
            } else {
                Vec::new()
            };
            for hex_key in &revoke {
                keys.push(parse_pubkey_hex(hex_key)?);
            }
            keys.sort_by_key(|k| k.0);
            keys.dedup_by_key(|k| k.0);

            let list = SignedRevocationList::issue(
                &release,
                release_cert_file.cert,
                keys,
                UnixSeconds::now().0,
            );
            if out.exists() {
                anyhow::bail!("refusing to overwrite existing file: {out:?}");
            }
            std::fs::write(&out, serde_json::to_vec_pretty(&list)?)?;
            println!(
                "revocation list with {} key(s) written to {out:?}",
                list.revoked_keys.len()
            );
        }
        Commands::VerifyChain {
            root_pub,
            release_cert,
            bundle_cert,
        } => {
            let root_pub_hex: String = serde_json::from_slice(&std::fs::read(&root_pub)?)?;
            let root_public_key = PublicKey::from_hex(&root_pub_hex)?;
            let trust_root = TrustRoot { root_public_key };

            let release_cert_file = read_cert_file(&release_cert)?;
            release_cert_file
                .cert
                .verify_against(&trust_root.root_public_key)
                .map_err(|_| anyhow::anyhow!("release cert does NOT chain to root"))?;
            println!("release cert chains to root: OK");

            if let Some(bundle_cert_path) = bundle_cert {
                let bundle_cert_file = read_cert_file(&bundle_cert_path)?;
                bundle_cert_file
                    .cert
                    .verify_against(&release_cert_file.public_key)
                    .map_err(|_| anyhow::anyhow!("bundle cert does NOT chain to release key"))?;
                println!("bundle cert chains to release key: OK");
            }
        }
    }
    Ok(())
}
