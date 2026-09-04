use anyhow::{Context, Result};
use clap::Parser;
use compat_config::deployment::DeploymentConfig;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use subscription::{standard_endpoints, AppState, RateLimiter};

/// Reads the deployment-wide Hysteria2 salamander obfuscation password
/// (see `DeploymentConfig::hysteria_obfs_password_file`'s doc comment for
/// why this file may legitimately not exist).
///
/// NotFound is a common, legitimate state — deployments that predate
/// obfuscation support, or that were never rotated to enable it — and
/// obfuscation simply stays off (`Ok(None)`), never a startup failure.
/// Any OTHER read failure (permission error, corrupt filesystem,
/// ownership drift after a botched restore, ...) must NOT collapse into
/// that same "disabled" state: the file's mere existence generally means
/// an operator ran `vpn-admin hysteria-obfs-rotate`, and sing-box's own
/// Hysteria2 listener config (a separate file this process never reads)
/// may already have obfuscation turned on and expect it from clients —
/// so an unreadable-but-present file here would otherwise silently start
/// serving subscriptions missing an obfuscation credential the
/// deployment actually requires, with a real HTTP 200 and no visible
/// error anywhere (docs/FINAL_PRODUCTION_AUDIT.md F-05). This fails
/// closed instead, exactly like the public_key/short_id checks in
/// `main()`.
fn read_hysteria_obfs_password(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let s = s.trim().to_string();
            Ok((!s.is_empty()).then_some(s))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| {
            format!(
                "hysteria obfuscation password file {path:?} exists but could not be read — \
                 refusing to start and possibly serve subscriptions silently missing an \
                 obfuscation credential the deployment expects; run `vpn-admin doctor` to \
                 diagnose"
            )
        }),
    }
}

#[cfg(test)]
mod hysteria_obfs_password_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn missing_file_is_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist");
        assert_eq!(read_hysteria_obfs_password(&path).unwrap(), None);
    }

    #[test]
    fn valid_password_is_trimmed_and_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("obfs");
        std::fs::write(&path, "  s3cr3t-salamander\n").unwrap();
        assert_eq!(
            read_hysteria_obfs_password(&path).unwrap(),
            Some("s3cr3t-salamander".to_string())
        );
    }

    #[test]
    fn empty_file_is_treated_as_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("obfs");
        std::fs::write(&path, "   \n").unwrap();
        assert_eq!(read_hysteria_obfs_password(&path).unwrap(), None);
    }

    #[test]
    fn unreadable_present_file_fails_closed_rather_than_looking_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("obfs");
        std::fs::write(&path, "s3cr3t").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root ignores Unix permission bits, so this assertion only holds
        // when the test itself isn't running as root (CI runs as root for
        // ownership-dependent ownership tests elsewhere in this repo, per
        // deploy/lib/tests/test-certbot-renewal-recovery.sh's comment) —
        // skip rather than false-fail in that environment.
        if unsafe { libc_geteuid() } == 0 {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            return;
        }
        let err = read_hysteria_obfs_password(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("could not be read"),
            "unexpected error message: {err:#}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    // Avoids pulling in the `libc` crate for a single syscall this test
    // module needs to skip correctly under root (CI).
    unsafe fn libc_geteuid() -> u32 {
        extern "C" {
            fn geteuid() -> u32;
        }
        geteuid()
    }
}

#[derive(Parser)]
#[command(
    name = "subscription",
    version,
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
    // This service caches `public_key`/`short_id` in AppState for its
    // entire process lifetime (never rereads them — see AppState's doc
    // comment) — so a corrupt/empty file here doesn't just make ONE
    // request fail, it makes every subscription this process ever
    // serves silently unusable (a `vless://...&pbk=&sid=...` URI at a
    // real HTTP 200) until the process is restarted. Reuse the same
    // canonical shape checks `vpn-admin init`/`doctor`/`restore` already
    // use, rather than trusting these files just because they exist —
    // refuse to start at all instead of binding and serving broken
    // subscriptions.
    compat_config::credentials::validate_reality_public_key_shape(&public_key)
        .map_err(anyhow::Error::msg)
        .context(
            "reality public key is present but invalid — refusing to start and serve broken \
             subscriptions; run `vpn-admin doctor` to diagnose",
        )?;
    let short_id = std::fs::read_to_string(cfg.reality_dir().join("short_id.txt"))
        .context("reality short_id missing — run `vpn-admin init` on the server first")?
        .trim()
        .to_string();
    compat_config::credentials::validate_reality_short_id(&short_id)
        .map_err(anyhow::Error::msg)
        .context(
            "reality short_id is present but invalid — refusing to start and serve broken \
             subscriptions; run `vpn-admin doctor` to diagnose",
        )?;
    // See `read_hysteria_obfs_password`'s doc comment
    // (docs/FINAL_PRODUCTION_AUDIT.md F-05): NotFound legitimately means
    // "obfuscation disabled", any other read failure must fail closed
    // rather than silently look the same.
    let hysteria_obfs_password = read_hysteria_obfs_password(&cfg.hysteria_obfs_password_file())?;

    let endpoints = standard_endpoints(
        &cfg.public_host,
        cfg.reality.listen_port,
        cfg.hysteria2.listen_port,
        &public_key,
        &short_id,
        &cfg.reality.handshake_server,
        hysteria_obfs_password.as_deref(),
    );

    let state = std::sync::Arc::new(AppState {
        users_file: cfg.users_file(),
        endpoints,
        // Sized for the WHOLE deployment, not for one client: behind nginx
        // every request appears to come from 127.0.0.1, so this is one
        // shared bucket (see `RateLimiter`'s doc comment). The previous
        // 20/0.5 was a sensible per-IP budget and a crippling global one —
        // 5 r/s of junk kept it permanently empty and locked out every
        // legitimate user. nginx enforces the per-client rate.
        rate_limiter: Mutex::new(RateLimiter::new(200.0, 50.0)),
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
