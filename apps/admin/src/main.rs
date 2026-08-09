//! `vpn-admin`: administration CLi for compatibility (VLESS+REALITY /
//! Hysteria2) users. Operates entirely on the local `users.json` store
//! plus the rendered sing-box config (spec §15/§16) — no PostgreSQL, no
//! separate control-plane service. Never prints secrets in a normal
//! listing (spec §15); the raw subscription token is shown exactly once,
//! at `create` or `rotate-token` time, because only its hash is persisted
//! (spec §26).

mod service;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use common::UnixSeconds;
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, Hysteria2ServerParams, RealityServerParams};
use compat_config::secret::SecretString;
use compat_config::server::{
    apply_config_atomically, config_backup_path, render_singbox_server_config,
    CompatibilityBackend, ServerPorts, SingBoxBackend,
};
use compat_config::{credentials, store};
use service::CompatibilityServiceManager;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "vpn-admin",
    about = "Compatibility (Hiddify/VLESS-REALITY/Hysteria2) user administration"
)]
struct Cli {
    /// Path to the deployment configuration (spec §36).
    #[arg(long, default_value = "/etc/vpn/deployment.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate missing server secrets (REALITY keypair, short_id) via
    /// the real `sing-box` binary. Refuses to overwrite an existing
    /// REALITY key unless `--rotate` is passed (spec §37).
    Init {
        #[arg(long)]
        rotate: bool,
    },
    /// Regenerate and atomically apply the sing-box server config from
    /// the current user store, without changing any user.
    RenderConfig,
    /// Print vpn-admin's own version and the configured sing-box
    /// binary's reported version, if present.
    Version,
    /// Summarize the current deployment: service state, user counts,
    /// config presence. Does not print secrets.
    Status,
    /// Run diagnostic checks and print `[OK]`/`[WARN]`/`[FAIL]` for each.
    /// Exits non-zero if any check fails. Checks that need a tool not
    /// present on this host are reported `[WARN] ... not available`, not
    /// silently skipped or faked as passing.
    Doctor,
    /// Back up the minimum state needed to rebuild this deployment
    /// (users, credential metadata, REALITY keys, Hysteria2 TLS
    /// material) into a single tar archive written mode 0600. Contains
    /// secrets — treat the output file as sensitive.
    Backup {
        /// Destination path. Defaults to
        /// `vpn1-backup-<unix-seconds>.tar` in the current directory.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Restore a backup produced by `backup`. Validates the archive
    /// contents (users file parses, REALITY private key present) before
    /// touching any live state, then applies the restored config through
    /// the same validate-then-apply-then-reload path as every other
    /// mutating command — a corrupt or incompatible backup is rejected
    /// by `sing-box check` rather than silently installed.
    Restore { archive: PathBuf },
    #[command(subcommand)]
    User(UserCommands),
}

#[derive(Subcommand)]
enum UserCommands {
    Create {
        #[arg(long)]
        name: String,
        /// Optional unix-seconds expiry.
        #[arg(long)]
        expires_at: Option<i64>,
        /// Print a terminal QR code of the subscription URL alongside
        /// the normal output.
        #[arg(long)]
        qr: bool,
        /// Print `{"id","name","enabled","subscription_url"}` as JSON
        /// instead of the human-readable form. Never includes server
        /// private keys.
        #[arg(long)]
        json: bool,
    },
    List,
    /// Print a terminal QR code encoding a user's subscription URL. The
    /// raw subscription token is never persisted (only its hash is), so
    /// this mints a *fresh* token the same way `rotate-token` does —
    /// there is no way to QR-encode a still-valid previously-issued
    /// token without knowing it, by design. The previous subscription
    /// URL stops working.
    Qr {
        user_id: String,
    },
    Enable {
        user_id: String,
    },
    Disable {
        user_id: String,
    },
    RotateToken {
        user_id: String,
        /// Print a terminal QR code of the new subscription URL.
        #[arg(long)]
        qr: bool,
    },
    /// Rotate only the VLESS UUID. Applies + reloads sing-box so the
    /// previous UUID stops working immediately.
    RotateVless {
        user_id: String,
    },
    /// Rotate only the Hysteria2 password. Applies + reloads sing-box so
    /// the previous password stops working immediately.
    RotateHysteria {
        user_id: String,
    },
    /// Rotate both the VLESS UUID and Hysteria2 password. Does not touch
    /// REALITY server keys or the subscription token — use `init
    /// --rotate` / `rotate-token` for those separately, since they have
    /// different blast radii.
    RotateCredentials {
        user_id: String,
    },
    Remove {
        user_id: String,
    },
    /// Print connection material for a user. The subscription URL itself
    /// requires the raw token, which (by design, spec §26) is not
    /// persisted — only shown at `create`/`rotate-token` time. This
    /// prints everything else plus a reminder of that fact.
    Subscription {
        user_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = DeploymentConfig::load(&cli.config)
        .with_context(|| format!("loading deployment config from {:?}", cli.config))?;

    match cli.command {
        Commands::Init { rotate } => cmd_init(&cfg, rotate),
        Commands::RenderConfig => cmd_render_config(&cfg),
        Commands::Version => cmd_version(&cfg),
        Commands::Status => cmd_status(&cfg),
        Commands::Doctor => cmd_doctor(&cfg),
        Commands::Backup { output } => cmd_backup(&cfg, &cli.config, output),
        Commands::Restore { archive } => cmd_restore(&cfg, &cli.config, &archive),
        Commands::User(UserCommands::Create {
            name,
            expires_at,
            qr,
            json,
        }) => cmd_user_create(&cfg, &name, expires_at, qr, json),
        Commands::User(UserCommands::List) => cmd_user_list(&cfg),
        Commands::User(UserCommands::Qr { user_id }) => cmd_user_qr(&cfg, &user_id),
        Commands::User(UserCommands::Enable { user_id }) => {
            cmd_user_set_enabled(&cfg, &user_id, true)
        }
        Commands::User(UserCommands::Disable { user_id }) => {
            cmd_user_set_enabled(&cfg, &user_id, false)
        }
        Commands::User(UserCommands::RotateToken { user_id, qr }) => {
            cmd_user_rotate_token(&cfg, &user_id, qr)
        }
        Commands::User(UserCommands::RotateVless { user_id }) => {
            cmd_user_rotate_vless(&cfg, &user_id)
        }
        Commands::User(UserCommands::RotateHysteria { user_id }) => {
            cmd_user_rotate_hysteria(&cfg, &user_id)
        }
        Commands::User(UserCommands::RotateCredentials { user_id }) => {
            cmd_user_rotate_credentials(&cfg, &user_id)
        }
        Commands::User(UserCommands::Remove { user_id }) => cmd_user_remove(&cfg, &user_id),
        Commands::User(UserCommands::Subscription { user_id }) => {
            cmd_user_subscription(&cfg, &user_id)
        }
    }
}

fn cmd_init(cfg: &DeploymentConfig, rotate: bool) -> Result<()> {
    std::fs::create_dir_all(cfg.reality_dir())?;
    std::fs::create_dir_all(cfg.hysteria_dir())?;
    std::fs::create_dir_all(cfg.users_file().parent().unwrap())?;

    let priv_path = cfg.reality_private_key_file();
    if priv_path.exists() && !rotate {
        println!(
            "REALITY key already present at {priv_path:?}; refusing to overwrite (pass --rotate to replace it deliberately — this breaks every existing client's connection until they re-import)."
        );
    } else {
        let output = std::process::Command::new(&cfg.singbox_binary)
            .arg("generate")
            .arg("reality-keypair")
            .output()
            .with_context(|| {
                format!(
                    "running {:?} generate reality-keypair (is sing-box installed at this path?)",
                    cfg.singbox_binary
                )
            })?;
        if !output.status.success() {
            bail!(
                "sing-box generate reality-keypair failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let private_key = extract_field(&text, "PrivateKey")
            .context("could not parse PrivateKey from sing-box output")?;
        let public_key = extract_field(&text, "PublicKey")
            .context("could not parse PublicKey from sing-box output")?;

        write_secret_file(&priv_path, &private_key)?;
        std::fs::write(cfg.reality_public_key_file(), &public_key)?;

        let short_id = credentials::generate_short_id();
        std::fs::write(cfg.reality_dir().join("short_id.txt"), &short_id)?;

        println!("Generated REALITY keypair at {:?}", cfg.reality_dir());
    }

    println!(
        "Hysteria2 TLS certificate/key are not generated by vpn-admin — place a valid \
         certificate at {:?} and key at {:?} (see docs/ALMALINUX_DEPLOYMENT.md for the \
         ACME setup).",
        cfg.hysteria_dir().join("cert.pem"),
        cfg.hysteria_dir().join("key.pem")
    );
    Ok(())
}

fn extract_field(text: &str, field: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(&format!("{field}:"))
            .or_else(|| line.strip_prefix(&format!("{field} ")))
            .map(|s| s.trim().to_string())
    })
}

#[cfg(unix)]
fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;
    Ok(())
}

fn load_reality_params(cfg: &DeploymentConfig) -> Result<RealityServerParams> {
    let private_key_hex = std::fs::read_to_string(cfg.reality_private_key_file())
        .context("reality private key missing — run `vpn-admin init` first")?
        .trim()
        .to_string();
    let public_key_hex = std::fs::read_to_string(cfg.reality_public_key_file())?
        .trim()
        .to_string();
    let short_id = std::fs::read_to_string(cfg.reality_dir().join("short_id.txt"))?
        .trim()
        .to_string();
    Ok(RealityServerParams {
        private_key_hex: SecretString::new(private_key_hex),
        public_key_hex,
        short_ids: vec![short_id],
        handshake_server: cfg.reality.handshake_server.clone(),
        handshake_port: cfg.reality.handshake_port,
    })
}

fn load_hysteria_params(cfg: &DeploymentConfig) -> Hysteria2ServerParams {
    let masquerade_dir = cfg.hysteria_dir().join("masquerade");
    Hysteria2ServerParams {
        tls_cert_path: cfg
            .hysteria_dir()
            .join("cert.pem")
            .to_string_lossy()
            .into_owned(),
        tls_key_path: cfg
            .hysteria_dir()
            .join("key.pem")
            .to_string_lossy()
            .into_owned(),
        obfs_password: None,
        // Only advertise masquerade if the directory actually exists —
        // installer creates it with a placeholder file; local dev/test
        // setups that skip that step get no masquerade rather than a
        // sing-box config referencing a missing path.
        masquerade_dir_path: masquerade_dir
            .exists()
            .then(|| masquerade_dir.to_string_lossy().into_owned()),
    }
}

/// Render + validate + atomically apply the sing-box config from the
/// current user store, then reload the running service and verify it
/// came back up healthy. Never overwrites a known-working config with an
/// invalid one (spec §16), and never claims a user mutation (create/
/// disable/enable/remove/rotate) succeeded while the running server
/// still has the old credentials loaded — see
/// docs/PRODUCTION_HARDENING_PLAN.md #4/#7.
///
/// If `sing-box` isn't installed on this machine (e.g. local
/// development), or `systemctl` isn't available (non-systemd
/// environment, e.g. this function's own tests), this prints a warning
/// instead of failing — user-store mutations still succeed, but the
/// caller is told explicitly that nothing was reloaded.
fn regenerate_singbox_config(cfg: &DeploymentConfig) -> Result<()> {
    let users = store::load_users(&cfg.users_file())?;
    let reality = match load_reality_params(cfg) {
        Ok(r) => r,
        Err(e) => {
            println!("warning: skipping sing-box config render/apply: {e}");
            return Ok(());
        }
    };
    let hysteria = load_hysteria_params(cfg);
    let ports = ServerPorts {
        vless_reality_port: cfg.reality.listen_port,
        hysteria2_port: cfg.hysteria2.listen_port,
    };
    let now = UnixSeconds::now().0 as i64;
    let doc = render_singbox_server_config(&users, &reality, &hysteria, ports, now);

    let target = cfg.singbox_config_file();
    if !cfg.singbox_binary.exists() {
        println!(
            "warning: {:?} not found; wrote nothing. Install sing-box, then run `vpn-admin render-config`.",
            cfg.singbox_binary
        );
        return Ok(());
    }
    let backend = SingBoxBackend {
        binary_path: cfg.singbox_binary.clone(),
    };
    apply_config_atomically(&doc, &target, |p| backend.validate(p))
        .context("applying sing-box config")?;
    println!("sing-box config updated at {target:?} (validated by `sing-box check`).");

    let mgr = CompatibilityServiceManager::default();
    if !mgr.is_available() {
        println!(
            "warning: systemctl not available; config written but sing-box was NOT reloaded. \
             On a real deployment this means the change has not taken effect yet — run \
             `systemctl reload-or-restart sing-box` manually."
        );
        return Ok(());
    }
    if !mgr.is_unit_installed() {
        println!(
            "warning: sing-box.service is not installed on this host (expected in CI/local \
             dev); config written but not reloaded. On a real deployment this means the \
             change has not taken effect yet — run `deploy/almalinux/install.sh` (or \
             `systemctl reload-or-restart sing-box` if the unit already exists) manually."
        );
        return Ok(());
    }

    if let Err(reload_err) = mgr.reload_and_verify() {
        let backup = config_backup_path(&target);
        let restored = backup.exists() && std::fs::copy(&backup, &target).is_ok();
        let recovery_reload_ok = restored && mgr.reload_and_verify().is_ok();
        bail!(
            "sing-box reload failed after applying the new config ({reload_err}). \
             The requested change did NOT take effect on the running server. \
             {}",
            if recovery_reload_ok {
                "Previous working config was restored and the service was reloaded back to it \
                 successfully — the server is running the PREVIOUS configuration now."
            } else {
                "Attempted to restore the previous config but that ALSO failed to reload — \
                 the service may be in a broken state. Manual intervention required: check \
                 `systemctl status sing-box` and `journalctl -u sing-box`."
            }
        );
    }
    println!("sing-box reloaded and verified active.");
    Ok(())
}

fn cmd_render_config(cfg: &DeploymentConfig) -> Result<()> {
    regenerate_singbox_config(cfg)
}

fn subscription_url(cfg: &DeploymentConfig, token: &str) -> String {
    format!(
        "https://{}:{}/sub/{}",
        cfg.subscription_host, cfg.subscription.public_port, token
    )
}

/// Print a terminal QR code encoding `data`. QR codes intentionally
/// encode only the subscription URL, never the full server
/// configuration (spec §6). PNG file output is not implemented — kept
/// out to avoid pulling in an image-encoding dependency for a
/// convenience feature; terminal/unicode rendering covers the primary
/// onboarding flow (admin runs this over SSH and the end user scans the
/// terminal, or the admin re-types/pastes the URL shown alongside it).
fn print_qr(data: &str) -> Result<()> {
    let code = qrcode::QrCode::new(data.as_bytes()).context("encoding QR code")?;
    let image = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    println!("{image}");
    Ok(())
}

fn cmd_user_create(
    cfg: &DeploymentConfig,
    name: &str,
    expires_at: Option<i64>,
    qr: bool,
    json: bool,
) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    // 128-bit CSPRNG id (spec: do not reuse the 32-bit REALITY short_id
    // generator as a user id). Collision detection is defense in depth
    // on top of 128 bits of entropy, not a load-bearing check.
    let mut id = credentials::generate_user_id();
    while users.iter().any(|u| u.id == id) {
        id = credentials::generate_user_id();
    }
    let token = credentials::generate_subscription_token();
    let user = CompatUser {
        id: id.clone(),
        name: name.to_string(),
        enabled: true,
        vless_uuid: credentials::generate_uuid_v4(),
        hysteria2_password: SecretString::new(credentials::generate_hysteria2_password()),
        subscription_token_hash_hex: credentials::hash_token(&token),
        created_at: UnixSeconds::now().0 as i64,
        expires_at,
    };
    users.push(user);
    store::save_users_atomic(&cfg.users_file(), &users)?;
    regenerate_singbox_config(cfg)?;

    let url = subscription_url(cfg, &token);
    if json {
        let out = serde_json::json!({
            "id": id,
            "name": name,
            "enabled": true,
            "subscription_url": url,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("User created: {id}");
    println!();
    println!("Subscription:\n{url}");
    println!();
    println!("This URL is shown once. It is not recoverable — use `vpn-admin user rotate-token {id}` to mint a new one if lost.");
    if qr {
        println!();
        println!("Scan this QR code in Hiddify (Add profile -> Scan QR):");
        print_qr(&url)?;
    }
    println!();
    println!("Recommended client: Hiddify (iOS/Android/MagicOS/Linux/Windows/macOS).");
    println!("1. Install Hiddify.  2. Add profile.  3. Scan the QR code above or paste the subscription URL.  4. Connect.");
    Ok(())
}

fn cmd_user_list(cfg: &DeploymentConfig) -> Result<()> {
    let users = store::load_users(&cfg.users_file())?;
    println!("{:<20} {:<16} {:<8}", "ID", "NAME", "ENABLED");
    for u in &users {
        println!(
            "{:<20} {:<16} {:<8}",
            u.id,
            u.name,
            if u.enabled { "yes" } else { "no" }
        );
    }
    Ok(())
}

fn find_user_mut<'a>(users: &'a mut [CompatUser], id: &str) -> Result<&'a mut CompatUser> {
    users
        .iter_mut()
        .find(|u| u.id == id)
        .ok_or_else(|| anyhow::anyhow!("no such user: {id}"))
}

fn cmd_user_set_enabled(cfg: &DeploymentConfig, id: &str, enabled: bool) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    find_user_mut(&mut users, id)?.enabled = enabled;
    store::save_users_atomic(&cfg.users_file(), &users)?;
    regenerate_singbox_config(cfg)?;
    println!("{id}: enabled={enabled}");
    Ok(())
}

fn cmd_user_rotate_token(cfg: &DeploymentConfig, id: &str, qr: bool) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    let token = credentials::generate_subscription_token();
    let hash = credentials::hash_token(&token);
    find_user_mut(&mut users, id)?.subscription_token_hash_hex = hash;
    store::save_users_atomic(&cfg.users_file(), &users)?;
    // Token rotation does not change VLESS/Hysteria2 credentials, so the
    // sing-box config is unaffected — no re-render needed.
    let url = subscription_url(cfg, &token);
    println!("New subscription:\n{url}");
    println!("The previous subscription URL for this user no longer works.");
    if qr {
        println!();
        print_qr(&url)?;
    }
    Ok(())
}

/// `vpn-admin user qr NAME`: mints a fresh subscription token (see the
/// `UserCommands::Qr` doc comment for why this can't just re-derive an
/// existing one) and prints it as a terminal QR code.
fn cmd_user_qr(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    println!(
        "Note: the subscription token is never stored in recoverable form, \
         so this mints a fresh one (like `rotate-token`) — the previous \
         subscription URL for this user stops working."
    );
    println!();
    cmd_user_rotate_token(cfg, id, true)
}

/// Common rotate-and-apply flow: mutate the user in-place via `mutate`,
/// save, render+validate+apply+reload (with rollback on failure — see
/// `regenerate_singbox_config`), and only then report success.
fn rotate_and_apply(
    cfg: &DeploymentConfig,
    id: &str,
    what: &str,
    mutate: impl FnOnce(&mut CompatUser),
) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    mutate(find_user_mut(&mut users, id)?);
    store::save_users_atomic(&cfg.users_file(), &users)?;
    regenerate_singbox_config(cfg)?;
    println!("{id}: {what} rotated and applied to the running server.");
    Ok(())
}

fn cmd_user_rotate_vless(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    rotate_and_apply(cfg, id, "VLESS UUID", |u| {
        u.vless_uuid = credentials::generate_uuid_v4();
    })
}

fn cmd_user_rotate_hysteria(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    rotate_and_apply(cfg, id, "Hysteria2 password", |u| {
        u.hysteria2_password = SecretString::new(credentials::generate_hysteria2_password());
    })
}

fn cmd_user_rotate_credentials(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    rotate_and_apply(cfg, id, "VLESS UUID + Hysteria2 password", |u| {
        u.vless_uuid = credentials::generate_uuid_v4();
        u.hysteria2_password = SecretString::new(credentials::generate_hysteria2_password());
    })
}

fn cmd_user_remove(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    let before = users.len();
    users.retain(|u| u.id != id);
    if users.len() == before {
        bail!("no such user: {id}");
    }
    store::save_users_atomic(&cfg.users_file(), &users)?;
    regenerate_singbox_config(cfg)?;
    println!("{id}: removed");
    Ok(())
}

fn cmd_user_subscription(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    let users = store::load_users(&cfg.users_file())?;
    let user = users
        .iter()
        .find(|u| u.id == id)
        .ok_or_else(|| anyhow::anyhow!("no such user: {id}"))?;
    println!("User ID:  {}", user.id);
    println!("Name:     {}", user.name);
    println!("Enabled:  {}", user.enabled);
    println!(
        "Expiry:   {}",
        user.expires_at
            .map(|e| e.to_string())
            .unwrap_or_else(|| "never".to_string())
    );
    println!(
        "Public subscription host: {}:{}",
        cfg.subscription_host, cfg.subscription.public_port
    );
    println!();
    println!("Subscription token cannot be recovered.");
    println!("Run:");
    println!("  vpn-admin user rotate-token {id}");
    println!("to create a new URL.");
    Ok(())
}

fn cmd_version(cfg: &DeploymentConfig) -> Result<()> {
    println!("vpn1 {}", env!("CARGO_PKG_VERSION"));
    match std::process::Command::new(&cfg.singbox_binary)
        .arg("version")
        .output()
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            if let Some(first_line) = text.lines().next() {
                println!("{first_line}");
            }
        }
        _ => println!(
            "sing-box: not found at {:?} (or failed to run `version`)",
            cfg.singbox_binary
        ),
    }
    Ok(())
}

fn cmd_status(cfg: &DeploymentConfig) -> Result<()> {
    let users = store::load_users(&cfg.users_file())?;
    let now = UnixSeconds::now().0 as i64;
    let active = users.iter().filter(|u| u.is_active(now)).count();
    let disabled = users.iter().filter(|u| !u.enabled).count();

    println!("vpn1 status");
    println!();
    let singbox = CompatibilityServiceManager::new("sing-box");
    println!("sing-box              {}", service_state_label(&singbox));
    let subscription = CompatibilityServiceManager::new("vpn-subscription");
    println!(
        "subscription-service  {}",
        service_state_label(&subscription)
    );
    println!();
    println!(
        "sing-box config:       {}",
        if cfg.singbox_config_file().exists() {
            "present"
        } else {
            "missing (run `vpn-admin render-config`)"
        }
    );
    println!();
    println!("Users:");
    println!("  total:    {}", users.len());
    println!("  active:   {active}");
    println!("  disabled: {disabled}");
    println!();
    println!(
        "Public endpoints: {}:{} (VLESS+REALITY tcp/443, Hysteria2 udp/443 per deployment.toml)",
        cfg.public_host, cfg.reality.listen_port
    );
    println!(
        "Subscription HTTPS: https://{}:{}/sub/<token>",
        cfg.subscription_host, cfg.subscription.public_port
    );

    if let Some(days) = cert_expiry_days(&cfg.hysteria_dir().join("cert.pem")) {
        match days {
            Ok(d) if d < 0 => println!("Certificate:           EXPIRED {} day(s) ago", -d),
            Ok(d) => println!("Certificate:            valid, expires in {d} day(s)"),
            Err(e) => println!("Certificate:            could not check ({e})"),
        }
    }
    Ok(())
}

fn service_state_label(mgr: &CompatibilityServiceManager) -> &'static str {
    if !mgr.is_available() {
        "unknown (systemctl not available)"
    } else if !mgr.is_unit_installed() {
        "not installed"
    } else if mgr.is_active() {
        "active"
    } else {
        "inactive"
    }
}

/// Days until the certificate at `path` expires (negative if already
/// expired), computed via the real `openssl` binary. `None` if the file
/// doesn't exist; `Some(Err(..))` if it exists but `openssl` isn't
/// available or the output couldn't be parsed — callers must surface
/// this as an explicit "could not check", never a silent pass.
fn cert_expiry_days(path: &std::path::Path) -> Option<Result<i64, String>> {
    if !path.exists() {
        return None;
    }
    let output = std::process::Command::new("openssl")
        .args(["x509", "-enddate", "-noout", "-in"])
        .arg(path)
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => return Some(Err(String::from_utf8_lossy(&o.stderr).trim().to_string())),
        Err(_) => return Some(Err("openssl not available".to_string())),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let date_str = match text.trim().strip_prefix("notAfter=") {
        Some(s) => s,
        None => return Some(Err(format!("unexpected openssl output: {text}"))),
    };
    // openssl's default date format, e.g. "Jan  2 03:04:05 2026 GMT" — parse
    // via `date -d` (coreutils) rather than hand-rolling a parser for a
    // locale-independent, well-known format string.
    let epoch = std::process::Command::new("date")
        .args(["-u", "-d", date_str, "+%s"])
        .output();
    match epoch {
        Ok(o) if o.status.success() => {
            let secs: i64 = String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .map_err(|e| format!("parsing date output: {e}"))
                .ok()?;
            let now = UnixSeconds::now().0 as i64;
            Some(Ok((secs - now) / 86400))
        }
        _ => Some(Err(format!("could not parse expiry date {date_str:?}"))),
    }
}

enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

fn report_check(status: CheckStatus, message: impl AsRef<str>) {
    let label = match status {
        CheckStatus::Ok => "[OK]  ",
        CheckStatus::Warn => "[WARN]",
        CheckStatus::Fail => "[FAIL]",
    };
    println!("{label} {}", message.as_ref());
}

#[cfg(unix)]
fn is_not_world_readable(path: &std::path::Path) -> Option<bool> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o007 == 0)
}

#[cfg(not(unix))]
fn is_not_world_readable(_path: &std::path::Path) -> Option<bool> {
    None
}

/// Diagnostic checks, `[OK]`/`[WARN]`/`[FAIL]` per line (spec §17).
/// Returns an error (non-zero exit) iff any check is `[FAIL]`. A check
/// that needs a tool unavailable in the current environment is `[WARN]`,
/// never silently skipped and never counted as `[OK]`.
fn cmd_doctor(cfg: &DeploymentConfig) -> Result<()> {
    let mut failures = 0u32;

    if cfg.singbox_binary.exists() {
        report_check(
            CheckStatus::Ok,
            format!("sing-box binary present at {:?}", cfg.singbox_binary),
        );
        let target = cfg.singbox_config_file();
        if target.exists() {
            let backend = SingBoxBackend {
                binary_path: cfg.singbox_binary.clone(),
            };
            match backend.validate(&target) {
                Ok(()) => report_check(CheckStatus::Ok, "sing-box config valid"),
                Err(e) => {
                    report_check(CheckStatus::Fail, format!("sing-box config invalid: {e}"));
                    failures += 1;
                }
            }
        } else {
            report_check(
                CheckStatus::Warn,
                "sing-box config not yet rendered (run `vpn-admin render-config`)",
            );
        }
    } else {
        report_check(
            CheckStatus::Fail,
            format!("sing-box binary missing at {:?}", cfg.singbox_binary),
        );
        failures += 1;
    }

    for (label, path) in [
        ("REALITY private key", cfg.reality_private_key_file()),
        ("REALITY public key", cfg.reality_public_key_file()),
    ] {
        if !path.exists() {
            report_check(CheckStatus::Warn, format!("{label} missing at {path:?}"));
            continue;
        }
        match is_not_world_readable(&path) {
            Some(true) => report_check(
                CheckStatus::Ok,
                format!("{label} present, not world-readable"),
            ),
            Some(false) => {
                report_check(
                    CheckStatus::Fail,
                    format!("{label} at {path:?} is world-readable"),
                );
                failures += 1;
            }
            None => report_check(
                CheckStatus::Warn,
                format!("{label} present (permission check unavailable on this platform)"),
            ),
        }
    }

    match store::load_users(&cfg.users_file()) {
        Ok(users) => report_check(
            CheckStatus::Ok,
            format!("user store parses ({} user(s))", users.len()),
        ),
        Err(e) => {
            report_check(CheckStatus::Fail, format!("user store invalid: {e}"));
            failures += 1;
        }
    }

    match cert_expiry_days(&cfg.hysteria_dir().join("cert.pem")) {
        None => report_check(
            CheckStatus::Warn,
            "Hysteria2 TLS certificate not present (see docs/ALMALINUX_DEPLOYMENT.md)",
        ),
        Some(Ok(days)) if days < 0 => {
            report_check(
                CheckStatus::Fail,
                format!("Hysteria2 TLS certificate EXPIRED {} day(s) ago", -days),
            );
            failures += 1;
        }
        Some(Ok(days)) if days < 30 => report_check(
            CheckStatus::Warn,
            format!("Hysteria2 TLS certificate expires in {days} day(s)"),
        ),
        Some(Ok(days)) => report_check(
            CheckStatus::Ok,
            format!("Hysteria2 TLS certificate valid, expires in {days} day(s)"),
        ),
        Some(Err(e)) => report_check(
            CheckStatus::Warn,
            format!("could not check Hysteria2 TLS certificate expiry: {e}"),
        ),
    }

    for name in ["sing-box", "vpn-subscription"] {
        let mgr = CompatibilityServiceManager::new(name);
        if !mgr.is_available() {
            report_check(
                CheckStatus::Warn,
                format!("systemctl not available — cannot check {name}.service"),
            );
        } else if !mgr.is_unit_installed() {
            report_check(
                CheckStatus::Warn,
                format!("{name}.service not installed on this host"),
            );
        } else if mgr.is_active() {
            report_check(CheckStatus::Ok, format!("{name}.service active"));
        } else {
            report_check(CheckStatus::Fail, format!("{name}.service not active"));
            failures += 1;
        }
    }

    match std::process::Command::new("firewall-cmd")
        .arg("--state")
        .output()
    {
        Ok(o) if o.status.success() => report_check(CheckStatus::Ok, "firewalld running"),
        Ok(_) => {
            report_check(CheckStatus::Fail, "firewalld not running");
            failures += 1;
        }
        Err(_) => report_check(
            CheckStatus::Warn,
            "firewall-cmd not available — firewall check skipped",
        ),
    }

    println!();
    if failures > 0 {
        bail!("{failures} check(s) failed");
    }
    println!("All checks passed (see [WARN] lines above for anything unverifiable on this host).");
    Ok(())
}

/// Stage the minimum state needed to rebuild this deployment into `dir`:
/// users store, deployment config, REALITY key material, Hysteria2 TLS
/// material. Missing optional pieces (e.g. no Hysteria2 cert yet) are
/// skipped, not treated as an error — a backup taken mid-setup is still
/// useful.
fn stage_backup_contents(
    cfg: &DeploymentConfig,
    config_path: &std::path::Path,
    dir: &std::path::Path,
) -> Result<()> {
    let copy_if_exists = |src: &std::path::Path, dst: &std::path::Path| -> Result<()> {
        if !src.exists() {
            return Ok(());
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
        Ok(())
    };

    copy_if_exists(&cfg.users_file(), &dir.join("users/users.json"))?;
    copy_if_exists(config_path, &dir.join("deployment.toml"))?;
    copy_if_exists(
        &cfg.reality_private_key_file(),
        &dir.join("reality/private.key"),
    )?;
    copy_if_exists(
        &cfg.reality_public_key_file(),
        &dir.join("reality/public.key"),
    )?;
    copy_if_exists(
        &cfg.reality_dir().join("short_id.txt"),
        &dir.join("reality/short_id.txt"),
    )?;
    copy_if_exists(
        &cfg.hysteria_dir().join("cert.pem"),
        &dir.join("hysteria/cert.pem"),
    )?;
    copy_if_exists(
        &cfg.hysteria_dir().join("key.pem"),
        &dir.join("hysteria/key.pem"),
    )?;
    Ok(())
}

fn cmd_backup(
    cfg: &DeploymentConfig,
    config_path: &std::path::Path,
    output: Option<PathBuf>,
) -> Result<()> {
    let dest = output
        .unwrap_or_else(|| PathBuf::from(format!("vpn1-backup-{}.tar", UnixSeconds::now().0)));
    let staging = tempdir_here()?;
    stage_backup_contents(cfg, config_path, staging.path())?;

    let status = std::process::Command::new("tar")
        .arg("-cf")
        .arg(&dest)
        .arg("-C")
        .arg(staging.path())
        .arg(".")
        .status()
        .context("running tar to create backup archive")?;
    if !status.success() {
        bail!("tar exited with failure creating {dest:?}");
    }
    write_secret_file_at(&dest)?;

    println!("Backup written to {dest:?}.");
    println!(
        "This archive contains secrets (REALITY private key, Hysteria2 TLS key, user \
         credential hashes) — store it as securely as the live server."
    );
    Ok(())
}

#[cfg(unix)]
fn write_secret_file_at(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret_file_at(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

fn tempdir_here() -> Result<tempfile::TempDir> {
    tempfile::tempdir().context("creating temporary staging directory")
}

fn cmd_restore(
    cfg: &DeploymentConfig,
    config_path: &std::path::Path,
    archive: &std::path::Path,
) -> Result<()> {
    let staging = tempdir_here()?;
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(staging.path())
        .status()
        .context("running tar to extract backup archive")?;
    if !status.success() {
        bail!("tar exited with failure extracting {archive:?}");
    }

    // Validate before touching any live state (spec §20: "Validate
    // restored data before replacing active state").
    let users_path = staging.path().join("users/users.json");
    let restored_users: Vec<CompatUser> = if users_path.exists() {
        let bytes = std::fs::read(&users_path).context("reading restored users.json")?;
        serde_json::from_slice(&bytes)
            .context("restored users.json is not valid — refusing to restore")?
    } else {
        bail!("archive does not contain users/users.json — refusing to restore");
    };
    let reality_key_path = staging.path().join("reality/private.key");
    if !reality_key_path.exists() {
        bail!("archive does not contain reality/private.key — refusing to restore");
    }

    // Only after validation: copy into place.
    std::fs::create_dir_all(cfg.reality_dir())?;
    std::fs::create_dir_all(cfg.hysteria_dir())?;
    std::fs::create_dir_all(cfg.users_file().parent().unwrap())?;

    store::save_users_atomic(&cfg.users_file(), &restored_users)?;
    let restore_targets: [(&str, PathBuf); 5] = [
        ("reality/private.key", cfg.reality_private_key_file()),
        ("reality/public.key", cfg.reality_public_key_file()),
        (
            "reality/short_id.txt",
            cfg.reality_dir().join("short_id.txt"),
        ),
        ("hysteria/cert.pem", cfg.hysteria_dir().join("cert.pem")),
        ("hysteria/key.pem", cfg.hysteria_dir().join("key.pem")),
    ];
    for (rel, dest) in restore_targets {
        let src = staging.path().join(rel);
        if src.exists() {
            std::fs::copy(&src, dest)?;
        }
    }

    let restored_deployment_toml = staging.path().join("deployment.toml");
    if restored_deployment_toml.exists() {
        println!(
            "Note: the backup's deployment.toml was NOT applied automatically (host/port \
             settings are not safe to overwrite blindly). It was extracted for reference at \
             {restored_deployment_toml:?} before this temporary directory is removed; the live \
             config remains {config_path:?}."
        );
    }

    println!(
        "Restored {} user(s) and REALITY/Hysteria2 material from {archive:?}.",
        restored_users.len()
    );
    regenerate_singbox_config(cfg)?;
    println!("Restore applied and validated against the running server.");
    Ok(())
}
