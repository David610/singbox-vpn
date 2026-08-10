//! `vpn-admin`: administration CLi for compatibility (VLESS+REALITY /
//! Hysteria2) users. Operates entirely on the local `users.json` store
//! plus the rendered sing-box config (spec §15/§16) — no PostgreSQL, no
//! separate control-plane service. Never prints secrets in a normal
//! listing (spec §15); the raw subscription token is shown exactly once,
//! at `create` or `rotate-token` time, because only its hash is persisted
//! (spec §26).

mod lock;
mod service;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use common::UnixSeconds;
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, Hysteria2ServerParams, RealityServerParams};
use compat_config::render::render_singbox_client_subscription;
use compat_config::secret::SecretString;
use compat_config::server::{
    apply_config_atomically, config_backup_path, render_singbox_server_config,
    CompatibilityBackend, ServerPorts, SingBoxBackend,
};
use compat_config::{credentials, store};
use serde_json::json;
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
    /// Run diagnostic checks and print `[OK]`/`[WARN]`/`[FAIL]` for each,
    /// each line tagged with the layer it actually covers (L1 process /
    /// L2 config-key-cert / L3 listeners / L4 subscription-coherence /
    /// L5-6 protocol handshake) so an operator can see at a glance what
    /// was, and was not, actually verified — "service active + config
    /// valid + port open" (L1-L3) is NOT the same claim as "a real
    /// client can authenticate" (L5-6). Exits non-zero if any check
    /// fails. Checks that need a tool not present on this host are
    /// reported `[WARN] ... not available`, not silently skipped or
    /// faked as passing.
    Doctor {
        /// Also run the best-effort L5/L6 protocol self-test: spin up
        /// the real `sing-box` binary as a throwaway client against this
        /// server's own VLESS+REALITY listener on loopback, using the
        /// live REALITY public key/short_id, to prove (not just infer)
        /// that a real client can complete a handshake. Off by default
        /// because it spawns a subprocess and does real network I/O;
        /// the always-on L1-L4 checks are pure file/struct comparisons.
        /// Unavailable or inconclusive checks are warnings unless
        /// `--require-protocol` is also supplied.
        #[arg(long)]
        protocol: bool,
        /// Make an unavailable or inconclusive protocol self-test a hard
        /// failure. The installer uses this after creating the first user so
        /// it cannot bless an untested REALITY decoy.
        #[arg(long, requires = "protocol")]
        require_protocol: bool,
    },
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

/// Every command that reads-then-writes `users.json` and/or
/// `config.json` — i.e. everything except the pure-read commands
/// (`version`, `status`, `doctor`, `user list`, `user subscription`) —
/// must hold the system-wide state lock for its entire duration
/// (docs/FINAL_PRODUCTION_AUDIT.md P0-4). `user qr` mutates (it rotates
/// the token, same as `rotate-token`) and is included.
fn command_mutates_state(cmd: &Commands) -> bool {
    !matches!(
        cmd,
        Commands::Version
            | Commands::Status
            | Commands::Doctor { .. }
            | Commands::User(UserCommands::List)
            | Commands::User(UserCommands::Subscription { .. })
    )
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = DeploymentConfig::load(&cli.config)
        .with_context(|| format!("loading deployment config from {:?}", cli.config))?;

    // Held for the ENTIRE duration of a mutating command — not just the
    // file write — so two concurrent `vpn-admin` invocations can never
    // interleave their load->mutate->persist->apply->reload sequences.
    let _state_lock = if command_mutates_state(&cli.command) {
        Some(lock::acquire_state_lock().context(
            "acquiring vpn1 state lock (another vpn-admin/install/update operation is in progress)",
        )?)
    } else {
        None
    };

    match cli.command {
        Commands::Init { rotate } => cmd_init(&cfg, rotate),
        Commands::RenderConfig => cmd_render_config(&cfg),
        Commands::Version => cmd_version(&cfg),
        Commands::Status => cmd_status(&cfg),
        Commands::Doctor {
            protocol,
            require_protocol,
        } => cmd_doctor(&cfg, protocol, require_protocol),
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
    let pub_path = cfg.reality_public_key_file();
    let sid_path = cfg.reality_dir().join("short_id.txt");
    let deployment_exists = cfg.singbox_config_file().exists();

    if priv_path.exists() {
        if !rotate {
            // A PARTIAL keyset is not a healthy "already initialised" state.
            // Returning Ok here when public.key/short_id.txt are missing left
            // install.sh's subsequent `chown` of those files failing under
            // `set -e` on every re-run, with no way out except deleting the
            // private key (which invalidates every client) — a permanent
            // installer deadlock. Say so instead of reporting success.
            if !pub_path.exists() || !sid_path.exists() {
                bail!(
                    "REALITY key material at {:?} is incomplete: private.key exists but {}. \
                     This is a partially-written keyset (an interrupted `init`), not a healthy \
                     deployment, and the public half cannot be recovered from the private half \
                     here. Re-run with `--rotate` to generate a fresh, coherent keypair — note \
                     that this invalidates every existing client's configuration and they must \
                     re-import their subscription.",
                    cfg.reality_dir(),
                    match (pub_path.exists(), sid_path.exists()) {
                        (false, false) => "public.key and short_id.txt are both missing",
                        (false, true) => "public.key is missing",
                        _ => "short_id.txt is missing",
                    }
                );
            }
            println!(
                "REALITY key already present at {priv_path:?}; refusing to overwrite (pass --rotate to replace it deliberately — this breaks every existing client's connection until they re-import)."
            );
            return Ok(());
        }
        // A key already exists and this is a deliberate rotation: this
        // MUST go through the fully coordinated transactional flow
        // (docs/FINAL_PRODUCTION_AUDIT.md P0-5) — a bare key-file swap
        // here would leave the running sing-box serving the OLD private
        // key (clients still connect fine) while any freshly-restarted
        // subscription service would advertise the NEW public key
        // (clients using it fail REALITY's handshake matching), a silent
        // split-brain that is worse than doing nothing.
        return cmd_reality_rotate(cfg);
    }

    // Key material is absent. Whether a plain generate-and-write is safe
    // depends on whether anything is ALREADY RUNNING on the old material —
    // not on whether the private key file happens to exist.
    //
    // A rendered sing-box config means there is a live deployment: sing-box
    // is enforcing key material from that config and vpn-subscription has
    // the old public key cached in memory. Writing three files and exiting 0
    // here (which is what this path used to do) leaves disk, generated
    // config, and both running processes disagreeing — the exact split-brain
    // the `--rotate` branch above exists to prevent. Route it through the
    // same transactional flow.
    if deployment_exists {
        println!(
            "REALITY key material is missing but a rendered sing-box config already exists at \
             {:?} — treating this as a rotation so the new key is rendered, validated, and \
             loaded by the running services rather than silently diverging from them.",
            cfg.singbox_config_file()
        );
        return cmd_reality_rotate(cfg);
    }

    // First-ever generation on a host with no deployment yet: nothing is
    // running that depends on this material. Still written atomically and
    // durably, because a crash between these three files is what produces
    // the unrecoverable partial keyset handled above.
    let (private_key, public_key, short_id) = generate_reality_keypair(cfg)?;
    install_rotated_key_file(&priv_path, &private_key)?;
    install_rotated_key_file(&pub_path, &public_key)?;
    install_rotated_key_file(&sid_path, &short_id)?;
    // `install_rotated_key_file` preserves an EXISTING target's mode, but
    // these are first writes with no target to inherit from, so they land
    // 0600. The REALITY public key and short_id are not secrets and
    // `vpn-subscription` must be able to read them via its group (the
    // installer chowns them to root:vpn-subscription right after this) —
    // 0600 would leave that service unable to read its own material.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&pub_path, &sid_path] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o640))
                .with_context(|| format!("setting mode 0640 on {p:?}"))?;
        }
    }
    fsync_dir(&cfg.reality_dir());
    println!("Generated REALITY keypair at {:?}", cfg.reality_dir());

    println!(
        "Hysteria2 TLS certificate/key are not generated by vpn-admin — place a valid \
         certificate at {:?} and key at {:?} (see docs/ALMALINUX_DEPLOYMENT.md for the \
         ACME setup).",
        cfg.hysteria_dir().join("cert.pem"),
        cfg.hysteria_dir().join("key.pem")
    );
    Ok(())
}

fn generate_reality_keypair(cfg: &DeploymentConfig) -> Result<(String, String, String)> {
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
    credentials::validate_reality_keypair(&private_key, &public_key)
        .map_err(|error| anyhow::anyhow!(error))
        .context("sing-box generated an incoherent REALITY keypair")?;
    let short_id = credentials::generate_short_id();
    Ok((private_key, public_key, short_id))
}

/// Sibling backup path used only for the duration of one rotate
/// operation (created just before the risky part starts, removed on
/// success, restored-from on failure). Not a long-term backup mechanism
/// — see `vpn-admin backup` for that.
fn rotate_backup_path(p: &std::path::Path) -> std::path::PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".rotate-bak");
    std::path::PathBuf::from(s)
}

/// Copy `src` to its `.rotate-bak` sibling if `src` exists, preserving
/// mode/ownership exactly (needed to restore a byte-for-byte identical
/// file, including the group a service account depends on, if rotation
/// fails partway through).
fn backup_for_rotate(src: &std::path::Path) -> Result<Option<std::path::PathBuf>> {
    use std::io::Write;
    let bak = rotate_backup_path(src);
    if bak.exists() {
        bail!(
            "refusing to overwrite stale transaction backup {bak:?}; recover or remove it after \
             verifying the live file before retrying"
        );
    }
    if !src.exists() {
        return Ok(None);
    }
    let mut source = std::fs::File::open(src)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut backup = options
        .open(&bak)
        .with_context(|| format!("creating transaction backup {bak:?}"))?;
    std::io::copy(&mut source, &mut backup)
        .with_context(|| format!("backing up {src:?} to {bak:?}"))?;
    backup.flush()?;
    backup.sync_all()?;
    #[cfg(unix)]
    {
        let meta = std::fs::metadata(src)?;
        std::fs::set_permissions(&bak, meta.permissions())?;
        std::os::unix::fs::chown(
            &bak,
            Some(std::os::unix::fs::MetadataExt::uid(&meta)),
            Some(std::os::unix::fs::MetadataExt::gid(&meta)),
        )
        .ok(); // best-effort: non-root test environments can't chown to an arbitrary uid/gid
    }
    Ok(Some(bak))
}

/// Restore `dst` from its `.rotate-bak` sibling (written by
/// `backup_for_rotate`) if one exists, then remove the backup file.
fn restore_from_rotate_backup(dst: &std::path::Path) -> Result<()> {
    let bak = rotate_backup_path(dst);
    if bak.exists() {
        #[cfg(not(unix))]
        if dst.exists() {
            std::fs::remove_file(dst)?;
        }
        std::fs::rename(&bak, dst)
            .with_context(|| format!("atomically restoring {dst:?} from backup {bak:?}"))?;
        if let Some(parent) = dst.parent() {
            fsync_dir(parent);
        }
    }
    Ok(())
}

fn remove_rotate_backup(p: &std::path::Path) {
    let _ = std::fs::remove_file(rotate_backup_path(p));
}

/// Coordinated REALITY key rotation (docs/FINAL_PRODUCTION_AUDIT.md
/// P0-5): backup -> generate candidate -> render+validate candidate
/// config with the REAL sing-box binary -> atomically install key
/// material -> apply config -> reload sing-box -> restart subscription
/// (it caches the public key/short_id at startup, so a plain config
/// reload does not pick up new REALITY public material on its own) ->
/// verify both -> commit. Any failure after key material starts being
/// touched triggers a full rollback of key files AND config, followed by
/// reloading/restarting both services back to the previous state and
/// verifying that recovery actually worked — this function only returns
/// `Ok` if the end state is fully consistent, and its `Err` messages
/// always say whether rollback succeeded.
fn cmd_reality_rotate(cfg: &DeploymentConfig) -> Result<()> {
    let priv_path = cfg.reality_private_key_file();
    let pub_path = cfg.reality_public_key_file();
    let sid_path = cfg.reality_dir().join("short_id.txt");
    let config_target = cfg.singbox_config_file();

    if !cfg.singbox_binary.exists() {
        bail!(
            "cannot safely rotate: sing-box binary not found at {:?} — rotation requires \
             validating the candidate config with the real binary before installing new key \
             material (never rotate blind).",
            cfg.singbox_binary
        );
    }

    println!("Rotating REALITY key material...");
    let (candidate_priv, candidate_pub, candidate_sid) = generate_reality_keypair(cfg)?;

    let candidate_reality = RealityServerParams {
        private_key_hex: SecretString::new(candidate_priv.clone()),
        public_key_hex: candidate_pub.clone(),
        short_ids: vec![candidate_sid.clone()],
        handshake_server: cfg.reality.handshake_server.clone(),
        handshake_port: cfg.reality.handshake_port,
    };
    let users = store::load_users(&cfg.users_file())?;
    let hysteria = load_hysteria_params(cfg);
    let ports = ServerPorts {
        vless_reality_port: cfg.reality.listen_port,
        hysteria2_port: cfg.hysteria2.listen_port,
    };
    let now = UnixSeconds::now().0 as i64;
    let candidate_doc =
        render_singbox_server_config(&users, &candidate_reality, &hysteria, ports, now);

    let backend = SingBoxBackend {
        binary_path: cfg.singbox_binary.clone(),
    };

    // Validate the candidate BEFORE creating transaction backups or
    // touching any live file. A staging/check failure is therefore a pure
    // no-op and can never restore an unrelated persistent config.json.bak.
    let tmp_validate = config_target.with_extension("rotate-candidate.json");
    if let Err(e) = write_config_for_validation(&tmp_validate, &candidate_doc) {
        let _ = std::fs::remove_file(&tmp_validate);
        return Err(e).context("failed to stage candidate config; live state was not changed");
    }
    let validate_result = backend.validate(&tmp_validate);
    let _ = std::fs::remove_file(&tmp_validate);
    validate_result.context(
        "candidate config failed sing-box check; live state and transaction backups were not changed",
    )?;

    let singbox_mgr = CompatibilityServiceManager::default();
    let sub_mgr = CompatibilityServiceManager::new("vpn-subscription");
    if !offline_mutation_allowed()
        && (!singbox_mgr.is_available()
            || !singbox_mgr.is_unit_installed()
            || !sub_mgr.is_available()
            || !sub_mgr.is_unit_installed())
    {
        bail!(
            "refusing REALITY rotation: both sing-box.service and vpn-subscription.service must \
             be installed and controllable so the key change can be committed atomically"
        );
    }

    // Back up the whole keyset before mutation. If preparing any backup
    // fails, remove only backups created by this attempt and leave live
    // state untouched.
    let mut prepared = Vec::new();
    let mut existed = Vec::new();
    for path in [&priv_path, &pub_path, &sid_path] {
        match backup_for_rotate(path) {
            Ok(backup) => {
                existed.push(backup.is_some());
                if let Some(backup) = backup {
                    prepared.push(backup);
                }
            }
            Err(error) => {
                for backup in prepared {
                    let _ = std::fs::remove_file(backup);
                }
                return Err(error).context(
                    "failed to prepare complete REALITY rotation backup; live state was not changed",
                );
            }
        }
    }

    let config_applied = std::cell::Cell::new(false);

    let rollback = |reason: &str| -> String {
        let mut restore_ok = true;
        for (p, did_exist) in [&priv_path, &pub_path, &sid_path]
            .into_iter()
            .zip(existed.iter().copied())
        {
            let result = if did_exist {
                restore_from_rotate_backup(p)
            } else {
                std::fs::remove_file(p)
                    .or_else(|error| {
                        if error.kind() == std::io::ErrorKind::NotFound {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    })
                    .map_err(anyhow::Error::from)
            };
            if result.is_err() {
                restore_ok = false;
            }
        }
        // apply_config_atomically already keeps target_path.bak from
        // its OWN last successful write — restore from that if our
        // candidate config was ever actually applied.
        if config_applied.get() {
            let cfg_backup = config_backup_path(&config_target);
            if !cfg_backup.exists() || std::fs::copy(&cfg_backup, &config_target).is_err() {
                restore_ok = false;
            }
        }
        let singbox_recovered = !singbox_mgr.is_available()
            || !singbox_mgr.is_unit_installed()
            || singbox_mgr.reload_and_verify().is_ok();
        let sub_recovered = !sub_mgr.is_available()
            || !sub_mgr.is_unit_installed()
            || sub_mgr.reload_and_verify().is_ok();
        if restore_ok && singbox_recovered && sub_recovered {
            format!(
                "REALITY rotation FAILED ({reason}). Previous key material and config were \
                 restored and both services were verified healthy on the PREVIOUS key — no \
                 client-visible change occurred."
            )
        } else {
            format!(
                "REALITY rotation FAILED ({reason}). ROLLBACK ALSO FAILED (files_restored={restore_ok}, \
                 sing-box_recovered={singbox_recovered}, subscription_recovered={sub_recovered}). \
                 The server may be in a broken/inconsistent state. Manual intervention required: \
                 check `systemctl status sing-box vpn-subscription`, `journalctl -u sing-box -u \
                 vpn-subscription`, and compare {:?}/{:?}/{:?} against their .rotate-bak siblings.",
                priv_path, pub_path, sid_path
            )
        }
    };

    // Install new key material — reuses the same rename-with-preserved-
    // ownership helper the atomic config/user-store writers use, so the
    // existing root:sing-box / root:vpn-subscription ownership carries
    // forward automatically (docs/FINAL_PRODUCTION_AUDIT.md P0-2).
    if let Err(e) = install_rotated_key_file(&priv_path, &candidate_priv) {
        bail!(rollback(&format!("failed to install new private key: {e}")));
    }
    if let Err(e) = install_rotated_key_file(&pub_path, &candidate_pub) {
        bail!(rollback(&format!("failed to install new public key: {e}")));
    }
    if let Err(e) = install_rotated_key_file(&sid_path, &candidate_sid) {
        bail!(rollback(&format!("failed to install new short_id: {e}")));
    }

    if let Err(e) = apply_config_atomically(&candidate_doc, &config_target, |p| backend.validate(p))
    {
        bail!(rollback(&format!("failed to apply candidate config: {e}")));
    }
    config_applied.set(true);

    if singbox_mgr.is_available() && singbox_mgr.is_unit_installed() {
        if let Err(e) = singbox_mgr.reload_and_verify() {
            bail!(rollback(&format!("sing-box reload failed: {e}")));
        }
    } else {
        println!("warning: systemctl/sing-box.service not available — config written but sing-box was NOT reloaded.");
    }

    // The subscription service reads the REALITY public key/short_id
    // ONCE at startup (services/subscription/src/main.rs) and has no
    // config-reload path — it MUST be restarted, not just reloaded, or
    // it keeps advertising the OLD public key to every client that asks
    // for a subscription after this point (docs/FINAL_PRODUCTION_AUDIT.md
    // P0-5's core scenario).
    if sub_mgr.is_available() && sub_mgr.is_unit_installed() {
        if let Err(e) = sub_mgr.reload_and_verify() {
            bail!(rollback(&format!(
                "subscription service restart failed: {e}"
            )));
        }
    } else {
        println!("warning: systemctl/vpn-subscription.service not available — new public key written but subscription service was NOT restarted.");
    }

    // Commit: only now is it safe to discard the rollback material.
    for p in [&priv_path, &pub_path, &sid_path] {
        remove_rotate_backup(p);
    }

    println!(
        "REALITY key rotated and applied. Every existing client's subscription/profile is now \
         invalid until re-imported — the public key changed."
    );
    Ok(())
}

fn write_config_for_validation(path: &std::path::Path, doc: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(doc)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Overwrite `target` (which already exists — this is only used for
/// rotating already-installed key files) with `contents`, preserving the
/// existing owner/group exactly, via the same tmp-file+rename+
/// preserve-ownership pattern as `compat_config::store`/`server`. A bare
/// `std::fs::write` would truncate-in-place (not atomic) and a naive
/// tmp+rename would silently drop back to the writing process's own
/// group, both of which this project treats as bugs
/// (docs/FINAL_PRODUCTION_AUDIT.md P0-2) — this is the same fix applied
/// to the same class of write.
fn install_rotated_key_file(target: &std::path::Path, contents: &str) -> Result<()> {
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".rotate-tmp");
    let tmp_path = std::path::PathBuf::from(tmp);
    write_secret_file(&tmp_path, contents)?;
    // Preserve BOTH the existing mode and owner/group across the swap —
    // `write_secret_file` always writes 0600, but e.g. public.key/
    // short_id.txt are 0640 root:vpn-subscription (see
    // deploy/almalinux/install.sh's ownership matrix), and a bare rename
    // would otherwise silently downgrade them to 0600 root:root.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(target) {
        std::fs::set_permissions(&tmp_path, meta.permissions())?;
    }
    compat_config::ownership::preserve_ownership_before_rename(&tmp_path, target)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::rename(&tmp_path, target)?;
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
    // Durability matters here specifically because `config.json` IS fsynced
    // (see compat-config's `apply_config_atomically`). Without this, a power
    // loss just after a rotation can persist a config.json holding the NEW
    // private key while the key files revert to the OLD one — the more
    // durable write landing and the less durable one not.
    f.sync_all()?;
    Ok(())
}

/// Best-effort fsync of a directory, so a rename into it is durable. A
/// rename is not implicitly fsynced on Linux; without this the directory
/// entry can be lost even though the file contents were synced.
fn fsync_dir(dir: &std::path::Path) {
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
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
    credentials::validate_reality_keypair(&private_key_hex, &public_key_hex)
        .map_err(anyhow::Error::msg)
        .context("REALITY private.key/public.key coherence check failed")?;
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
/// Authorization mutations are fail-closed unless an offline operator
/// explicitly sets `VPN1_ALLOW_OFFLINE_MUTATION=1`. Plain `render-config`
/// remains usable during installation before systemd is available.
fn offline_mutation_allowed() -> bool {
    std::env::var("VPN1_ALLOW_OFFLINE_MUTATION").as_deref() == Ok("1")
}

fn applied_config_stamp_path(target: &std::path::Path) -> PathBuf {
    target.with_extension("applied.sha256")
}

fn rendered_config_fingerprint(doc: &serde_json::Value) -> Result<String> {
    let canonical = serde_json::to_string(doc)?;
    Ok(credentials::hash_token(&canonical))
}

fn commit_applied_config_stamp(target: &std::path::Path, fingerprint: &str) -> Result<()> {
    let stamp = applied_config_stamp_path(target);
    let tmp = stamp.with_extension(format!("tmp.{}", std::process::id()));
    write_secret_file(&tmp, fingerprint)?;
    std::fs::rename(&tmp, &stamp)?;
    if let Some(parent) = stamp.parent() {
        fsync_dir(parent);
    }
    Ok(())
}

fn render_and_apply_singbox_config(
    cfg: &DeploymentConfig,
    users: &[CompatUser],
    require_live_apply: bool,
) -> Result<()> {
    let reality = match load_reality_params(cfg) {
        Ok(r) => r,
        Err(e) => {
            if require_live_apply && !offline_mutation_allowed() {
                return Err(e).context(
                    "refusing to commit an authorization mutation without a complete, coherent \
                     REALITY keyset; run `vpn-admin init` first",
                );
            }
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
    let doc = render_singbox_server_config(users, &reality, &hysteria, ports, now);
    let candidate_fingerprint = rendered_config_fingerprint(&doc)?;

    let target = cfg.singbox_config_file();
    if !cfg.singbox_binary.exists() {
        if require_live_apply && !offline_mutation_allowed() {
            bail!(
                "refusing to commit an authorization mutation: sing-box binary not found at \
                 {:?}, so the candidate config cannot be validated or loaded. For an intentional \
                 offline/dev-only mutation set VPN1_ALLOW_OFFLINE_MUTATION=1 explicitly.",
                cfg.singbox_binary
            );
        }
        println!(
            "warning: {:?} not found; wrote nothing. Install sing-box, then run `vpn-admin render-config`.",
            cfg.singbox_binary
        );
        return Ok(());
    }
    let backend = SingBoxBackend {
        binary_path: cfg.singbox_binary.clone(),
    };
    let mgr = CompatibilityServiceManager::default();
    let service_available = mgr.is_available() && mgr.is_unit_installed();
    if require_live_apply && !service_available && !offline_mutation_allowed() {
        bail!(
            "refusing to commit an authorization mutation: systemctl/sing-box.service is not \
             available, so vpn-admin cannot prove the running authorization state changed. \
             For an intentional offline/dev-only mutation set VPN1_ALLOW_OFFLINE_MUTATION=1."
        );
    }

    let target_already_matches = std::fs::read(&target)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|current| current == doc);
    let applied_stamp_matches = std::fs::read_to_string(applied_config_stamp_path(&target))
        .is_ok_and(|stamp| stamp.trim() == candidate_fingerprint);
    if target_already_matches && applied_stamp_matches && service_available && mgr.is_active() {
        println!("sing-box authorization config is already current; no reload needed.");
        return Ok(());
    }

    apply_config_atomically(&doc, &target, |p| backend.validate(p))
        .context("applying sing-box config")?;
    println!("sing-box config updated at {target:?} (validated by `sing-box check`).");

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
    commit_applied_config_stamp(&target, &candidate_fingerprint)
        .context("recording the config version verified live")?;
    println!("sing-box reloaded and verified active.");
    Ok(())
}

#[cfg(unix)]
fn apply_restored_file_policy(path: &std::path::Path, group: &str) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
        .with_context(|| format!("setting restored-file mode on {path:?}"))?;
    // Non-root unit tests cannot change ownership. Production restore is
    // root-only and must set the exact service-readable owner/group even
    // when the destination did not exist before restore.
    if unsafe { libc::geteuid() } == 0 {
        let name = CString::new(group)?;
        let record = unsafe { libc::getgrnam(name.as_ptr()) };
        if record.is_null() {
            bail!("required service group {group:?} does not exist");
        }
        let gid = unsafe { (*record).gr_gid };
        std::os::unix::fs::chown(path, Some(0), Some(gid))
            .with_context(|| format!("setting root:{group} ownership on {path:?}"))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn apply_restored_file_policy(_path: &std::path::Path, _group: &str) -> Result<()> {
    Ok(())
}

fn regenerate_singbox_config(cfg: &DeploymentConfig, require_live_apply: bool) -> Result<()> {
    let users = store::load_users(&cfg.users_file())?;
    render_and_apply_singbox_config(cfg, &users, require_live_apply)
}

fn cmd_render_config(cfg: &DeploymentConfig) -> Result<()> {
    regenerate_singbox_config(cfg, false)
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
    let previous_users = users.clone();
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
    apply_users_and_save(cfg, &previous_users, &users)?;

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
    let previous_users = users.clone();
    find_user_mut(&mut users, id)?.enabled = enabled;
    apply_users_and_save(cfg, &previous_users, &users)?;
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
    let previous_users = users.clone();
    mutate(find_user_mut(&mut users, id)?);
    apply_users_and_save(cfg, &previous_users, &users)?;
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

/// Push a proposed user-store change to the running server, then publish it
/// to the subscription service only after live authorization is verified.
///
/// Every user mutation previously did `save_users_atomic(...)?;
/// regenerate_singbox_config(...)?;` with nothing compensating the first
/// call when the second failed — or when the process died between them.
/// The consequences were not symmetric or cosmetic:
///
///   * `user remove` / `user disable`: the user vanishes from the
///     authoritative store while the RUNNING server still authorizes them.
///     Revocation silently does not take effect, and because nothing
///     reconciles automatically, nothing ever notices.
///   * `user create`: the record is committed and the raw subscription
///     token — printed only AFTER the render — is lost forever.
///
/// If publishing users.json fails, the previous authorization document is
/// rendered and reloaded. The expiry reconciliation timer also repairs a
/// crash between these phases from the authoritative users.json state.
fn apply_users_and_save(
    cfg: &DeploymentConfig,
    previous_users: &[CompatUser],
    users: &[CompatUser],
) -> Result<()> {
    // Load the proposed authorization into sing-box before publishing the
    // new store to vpn-subscription. This makes the transition fail-closed:
    // a revocation reaches the protocol first, while a newly enabled
    // credential is not distributed until the protocol accepts it.
    render_and_apply_singbox_config(cfg, users, true)?;

    if let Err(save_error) = store::save_users_atomic(&cfg.users_file(), users) {
        let rollback = render_and_apply_singbox_config(cfg, previous_users, true);
        bail!(
            "authorization config was loaded, but users.json could not be committed ({save_error}). {}",
            if rollback.is_ok() {
                "The previous authorization config was restored and reloaded successfully."
            } else {
                "ROLLBACK ALSO FAILED; running authorization may not match users.json. Run \
                 `vpn-admin render-config` and `vpn-admin doctor --protocol` immediately."
            }
        );
    }
    Ok(())
}

fn cmd_user_remove(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    let mut users = store::load_users(&cfg.users_file())?;
    let previous_users = users.clone();
    let before = users.len();
    users.retain(|u| u.id != id);
    if users.len() == before {
        bail!("no such user: {id}");
    }
    apply_users_and_save(cfg, &previous_users, &users)?;
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

/// `layer` is one of `"L1"` (process), `"L2"` (config/key/cert),
/// `"L3"` (listeners/network), `"L4"` (subscription-coherence), or
/// `"L5-6"` (real protocol handshake) — see the module-level note above
/// `cmd_doctor` for why this labeling exists: L1-L3 all passing does
/// NOT mean a real client can connect (that's what the incident this
/// tagging responds to actually looked like).
fn report_check(status: CheckStatus, layer: &str, message: impl AsRef<str>) {
    let label = match status {
        CheckStatus::Ok => "[OK]  ",
        CheckStatus::Warn => "[WARN]",
        CheckStatus::Fail => "[FAIL]",
    };
    println!("{label} [{layer:<4}] {}", message.as_ref());
}

#[cfg(unix)]
fn installed_file_policy(
    path: &std::path::Path,
    expected_group: &str,
) -> Option<Result<(), String>> {
    use std::ffi::CString;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return Some(Err(error.to_string())),
    };
    let group_name = match CString::new(expected_group) {
        Ok(name) => name,
        Err(error) => return Some(Err(error.to_string())),
    };
    let group = unsafe { libc::getgrnam(group_name.as_ptr()) };
    if group.is_null() {
        return Some(Err(format!(
            "required group {expected_group:?} does not exist"
        )));
    }
    let expected_gid = unsafe { (*group).gr_gid };
    let mode = metadata.permissions().mode() & 0o7777;
    if metadata.uid() != 0 || metadata.gid() != expected_gid || mode != 0o640 {
        return Some(Err(format!(
            "expected root:{expected_group} mode 0640, found uid={} gid={} mode={mode:04o}",
            metadata.uid(),
            metadata.gid()
        )));
    }
    Some(Ok(()))
}

#[cfg(not(unix))]
fn installed_file_policy(
    _path: &std::path::Path,
    _expected_group: &str,
) -> Option<Result<(), String>> {
    None
}

fn report_installed_file_policy(
    path: &std::path::Path,
    expected_group: &str,
    label: &str,
    failures: &mut u32,
) {
    match installed_file_policy(path, expected_group) {
        Some(Ok(())) => report_check(
            CheckStatus::Ok,
            "L2",
            format!("{label} ownership/mode is root:{expected_group} 0640"),
        ),
        Some(Err(error)) => {
            report_check(
                CheckStatus::Fail,
                "L2",
                format!("{label} policy invalid: {error}"),
            );
            *failures += 1;
        }
        None => report_check(
            CheckStatus::Warn,
            "L2",
            format!("{label} ownership/mode check unavailable on this platform"),
        ),
    }
}

/// Diagnostic checks, `[OK]`/`[WARN]`/`[FAIL]` per line (spec §17), each
/// tagged with the layer it actually covers. This tagging exists
/// because a real production incident passed every check that existed
/// here before (process active, config valid, port open, cert valid —
/// L1-L3) while a real Hiddify client's VLESS+REALITY handshake still
/// failed: sing-box logged "REALITY: processed invalid connection"
/// because the subscription service was advertising REALITY key
/// material that no longer matched what sing-box was enforcing. L1-L3
/// cannot see that class of bug by construction — they check that
/// *a* config is valid and *a* process is running, never that the
/// config a real client receives agrees with the config the server
/// enforces, and never that a handshake actually completes. L4 (always
/// run, file/struct comparisons only) and L5-6 (opt-in via
/// `--protocol`, a real throwaway client handshake) close that gap.
///
/// Returns an error (non-zero exit) iff any check is `[FAIL]`. A check
/// that needs a tool unavailable in the current environment is `[WARN]`,
/// never silently skipped and never counted as `[OK]`. The L5-6
/// self-test is always `[WARN]` on an inconclusive/skipped outcome,
/// never `[FAIL]` — see `check_l5_l6_protocol_selftest`'s doc comment
/// for why it cannot always distinguish "broken" from "untestable from
/// here".
fn cmd_doctor(cfg: &DeploymentConfig, protocol: bool, require_protocol: bool) -> Result<()> {
    let mut failures = 0u32;

    if cfg.singbox_binary.exists() {
        report_check(
            CheckStatus::Ok,
            "L2",
            format!("sing-box binary present at {:?}", cfg.singbox_binary),
        );
        let target = cfg.singbox_config_file();
        if target.exists() {
            report_installed_file_policy(&target, "sing-box", "sing-box config", &mut failures);
            let backend = SingBoxBackend {
                binary_path: cfg.singbox_binary.clone(),
            };
            match backend.validate(&target) {
                Ok(()) => report_check(CheckStatus::Ok, "L2", "sing-box config valid"),
                Err(e) => {
                    report_check(
                        CheckStatus::Fail,
                        "L2",
                        format!("sing-box config invalid: {e}"),
                    );
                    failures += 1;
                }
            }
        } else {
            report_check(
                CheckStatus::Warn,
                "L2",
                "sing-box config not yet rendered (run `vpn-admin render-config`)",
            );
        }
    } else {
        report_check(
            CheckStatus::Fail,
            "L2",
            format!("sing-box binary missing at {:?}", cfg.singbox_binary),
        );
        failures += 1;
    }

    if !cfg.reality_private_key_file().exists() {
        report_check(
            CheckStatus::Fail,
            "L2",
            format!(
                "REALITY private key missing at {:?}",
                cfg.reality_private_key_file()
            ),
        );
        failures += 1;
    } else {
        report_installed_file_policy(
            &cfg.reality_private_key_file(),
            "sing-box",
            "REALITY private key",
            &mut failures,
        );
    }
    if cfg.reality_public_key_file().exists() {
        report_installed_file_policy(
            &cfg.reality_public_key_file(),
            "vpn-subscription",
            "REALITY public key",
            &mut failures,
        );
        let short_id_path = cfg.reality_dir().join("short_id.txt");
        if short_id_path.exists() {
            report_installed_file_policy(
                &short_id_path,
                "vpn-subscription",
                "REALITY short_id",
                &mut failures,
            );
        }
        match (
            std::fs::read_to_string(cfg.reality_private_key_file()),
            std::fs::read_to_string(cfg.reality_public_key_file()),
        ) {
            (Ok(private), Ok(public)) => {
                match credentials::validate_reality_keypair(private.trim(), public.trim()) {
                    Ok(()) => report_check(
                        CheckStatus::Ok,
                        "L2",
                        "REALITY public.key cryptographically corresponds to private.key (X25519 derivation)",
                    ),
                    Err(e) => {
                        report_check(CheckStatus::Fail, "L2", format!("REALITY keypair incoherent: {e}"));
                        failures += 1;
                    }
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                report_check(
                    CheckStatus::Fail,
                    "L2",
                    format!("cannot read REALITY keypair: {e}"),
                );
                failures += 1;
            }
        }
    } else {
        report_check(
            CheckStatus::Fail,
            "L2",
            format!(
                "REALITY public key missing at {:?}",
                cfg.reality_public_key_file()
            ),
        );
        failures += 1;
    }

    if cfg.users_file().exists() {
        report_installed_file_policy(
            &cfg.users_file(),
            "vpn-subscription",
            "user store",
            &mut failures,
        );
    }
    match store::load_users(&cfg.users_file()) {
        Ok(users) => report_check(
            CheckStatus::Ok,
            "L2",
            format!("user store parses ({} user(s))", users.len()),
        ),
        Err(e) => {
            report_check(CheckStatus::Fail, "L2", format!("user store invalid: {e}"));
            failures += 1;
        }
    }

    let hysteria_cert = cfg.hysteria_dir().join("cert.pem");
    let hysteria_key = cfg.hysteria_dir().join("key.pem");
    if hysteria_cert.exists() {
        report_installed_file_policy(
            &hysteria_cert,
            "sing-box",
            "Hysteria2 certificate",
            &mut failures,
        );
    }
    if hysteria_key.exists() {
        report_installed_file_policy(
            &hysteria_key,
            "sing-box",
            "Hysteria2 private key",
            &mut failures,
        );
    }
    match cert_expiry_days(&hysteria_cert) {
        None => report_check(
            CheckStatus::Warn,
            "L2",
            "Hysteria2 TLS certificate not present (see docs/ALMALINUX_DEPLOYMENT.md)",
        ),
        Some(Ok(days)) if days < 0 => {
            report_check(
                CheckStatus::Fail,
                "L2",
                format!("Hysteria2 TLS certificate EXPIRED {} day(s) ago", -days),
            );
            failures += 1;
        }
        Some(Ok(days)) if days < 30 => report_check(
            CheckStatus::Warn,
            "L2",
            format!("Hysteria2 TLS certificate expires in {days} day(s)"),
        ),
        Some(Ok(days)) => report_check(
            CheckStatus::Ok,
            "L2",
            format!("Hysteria2 TLS certificate valid, expires in {days} day(s)"),
        ),
        Some(Err(e)) => report_check(
            CheckStatus::Warn,
            "L2",
            format!("could not check Hysteria2 TLS certificate expiry: {e}"),
        ),
    }

    for name in ["sing-box", "vpn-subscription"] {
        let mgr = CompatibilityServiceManager::new(name);
        if !mgr.is_available() {
            report_check(
                CheckStatus::Warn,
                "L1",
                format!("systemctl not available — cannot check {name}.service"),
            );
        } else if !mgr.is_unit_installed() {
            report_check(
                CheckStatus::Warn,
                "L1",
                format!("{name}.service not installed on this host"),
            );
        } else if mgr.is_active() {
            report_check(CheckStatus::Ok, "L1", format!("{name}.service active"));
        } else {
            report_check(
                CheckStatus::Fail,
                "L1",
                format!("{name}.service not active"),
            );
            failures += 1;
        }
    }

    match std::process::Command::new("firewall-cmd")
        .arg("--state")
        .output()
    {
        Ok(o) if o.status.success() => report_check(CheckStatus::Ok, "L3", "firewalld running"),
        Ok(_) => {
            report_check(CheckStatus::Fail, "L3", "firewalld not running");
            failures += 1;
        }
        Err(_) => report_check(
            CheckStatus::Warn,
            "L3",
            "firewall-cmd not available — firewall check skipped",
        ),
    }

    for (proto_label, port, udp) in [
        ("VLESS+REALITY", cfg.reality.listen_port, false),
        ("Hysteria2", cfg.hysteria2.listen_port, true),
    ] {
        match listener_reported_by_ss(port, udp) {
            Some(true) => report_check(
                CheckStatus::Ok,
                "L3",
                format!(
                    "{proto_label} {} listener present on port {port}",
                    if udp { "UDP" } else { "TCP" }
                ),
            ),
            Some(false) => {
                report_check(
                    CheckStatus::Fail,
                    "L3",
                    format!(
                        "{proto_label} {} listener missing on port {port}",
                        if udp { "UDP" } else { "TCP" }
                    ),
                );
                failures += 1;
            }
            None => report_check(
                CheckStatus::Warn,
                "L3",
                format!("cannot inspect {proto_label} listener: `ss` is unavailable or failed"),
            ),
        }
    }

    // Additional UDP egress check: if Hysteria2 is listening, ensure the
    // host can send and receive basic UDP packets to public resolvers.
    // Try multiple resolvers and a small retry window to reduce false
    // negatives on transient failures.
    match listener_reported_by_ss(cfg.hysteria2.listen_port, true) {
        Some(true) => {
            let probe_cfg = cfg.udp_probe_config();
            let ipv4_candidates_vec = probe_cfg.ipv4_resolvers;
            let ipv6_candidates_vec = probe_cfg.ipv6_resolvers;
            let timeout = std::time::Duration::from_millis(probe_cfg.timeout_ms);
            let retries = probe_cfg.retries;
            let delay = std::time::Duration::from_millis(probe_cfg.delay_ms);

            let ipv4_refs: Vec<&str> = ipv4_candidates_vec.iter().map(|s| s.as_str()).collect();
            match run_udp_probe_candidates(&ipv4_refs, timeout, retries, delay) {
                Some(true) => report_check(
                    CheckStatus::Ok,
                    "L3",
                    "UDP egress (IPv4) appears functional (DNS via UDP to public resolvers succeeded)",
                ),
                Some(false) => {
                    report_check(
                        CheckStatus::Fail,
                        "L3",
                        "UDP egress (IPv4) appears blocked — Hysteria2 (QUIC/UDP) may not work from this VPS (tried multiple resolvers)",
                    );
                    failures += 1;
                }
                None => report_check(
                    CheckStatus::Warn,
                    "L3",
                    "UDP egress check (IPv4) unavailable on this host (socket bind/permission failed)",
                ),
            }

            let ipv6_refs: Vec<&str> = ipv6_candidates_vec.iter().map(|s| s.as_str()).collect();
            match run_udp_probe_candidates(&ipv6_refs, timeout, retries, delay) {
                Some(true) => report_check(
                    CheckStatus::Ok,
                    "L3",
                    "UDP egress (IPv6) appears functional (DNS via UDP to public resolvers succeeded)",
                ),
                Some(false) => {
                    report_check(
                        CheckStatus::Fail,
                        "L3",
                        "UDP egress (IPv6) appears blocked — QUIC/UDP over IPv6 may not work from this VPS (tried multiple resolvers)",
                    );
                    failures += 1;
                }
                None => report_check(
                    CheckStatus::Warn,
                    "L3",
                    "UDP egress check (IPv6) unavailable on this host (socket bind/permission failed or IPv6 disabled)",
                ),
            }
        }
        _ => {}
    }

    check_l4_subscription_coherence(cfg, &mut failures);
    check_l4_live_subscription_process_state(cfg, &mut failures);

    if protocol {
        check_l5_l6_protocol_selftest(cfg, &mut failures, require_protocol);
    } else {
        report_check(
            CheckStatus::Warn,
            "L5-6",
            "protocol handshake self-test not run (pass `--protocol` to actually dial this \
             server's own REALITY listener with a throwaway sing-box client) — passing every \
             check above does NOT prove a real client can authenticate",
        );
    }

    println!();
    if failures > 0 {
        bail!("{failures} check(s) failed");
    }
    println!("All checks passed (see [WARN] lines above for anything unverifiable on this host).");
    Ok(())
}

/// L4, always run, no network/subprocess involved: render the sing-box
/// server config AND the client subscription the live `vpn-subscription`
/// service would hand a real user right now, from the SAME in-memory
/// load of the current REALITY key files + `users.json`, and assert the
/// client's `public_key`/`short_id` are exactly what the server config
/// accepts. This alone is a regression guard (it re-exercises the real
/// render functions on every `doctor` run, not just in unit tests) — it
/// cannot, by itself, catch a *running* subscription process serving a
/// stale in-memory key from before its last restart, because both
/// renders here read the same on-disk files in the same process.
///
/// The second half closes exactly that gap without touching the
/// network: compare the sing-box `config.json` ALREADY on disk (the
/// config the last `systemctl reload-or-restart sing-box` actually
/// picked up) against what would be rendered right now from the current
/// files. If they differ, `vpn-admin render-config` was never re-run
/// after the REALITY key files or `users.json` changed — sing-box may
/// be enforcing different key material than the subscription service is
/// currently advertising to brand-new clients. This is the exact
/// "server and subscription-service disagree about REALITY key
/// material" incident class, caught from file contents alone. Private
/// key material is compared only via a SHA-256 fingerprint, never the
/// raw value.
fn check_l4_subscription_coherence(cfg: &DeploymentConfig, failures: &mut u32) {
    let reality = match load_reality_params(cfg) {
        Ok(r) => r,
        Err(e) => {
            report_check(
                CheckStatus::Warn,
                "L4",
                format!("skipping subscription-coherence check: {e}"),
            );
            return;
        }
    };
    let users = match store::load_users(&cfg.users_file()) {
        Ok(u) => u,
        Err(e) => {
            report_check(
                CheckStatus::Warn,
                "L4",
                format!("skipping subscription-coherence check: user store unreadable ({e})"),
            );
            return;
        }
    };
    let hysteria = load_hysteria_params(cfg);
    let ports = ServerPorts {
        vless_reality_port: cfg.reality.listen_port,
        hysteria2_port: cfg.hysteria2.listen_port,
    };
    let now = UnixSeconds::now().0 as i64;
    let fresh_server_doc = render_singbox_server_config(&users, &reality, &hysteria, ports, now);

    // The EXACT same function `services/subscription`'s live process
    // calls to build its own `AppState.endpoints` — not a hand-rolled
    // equivalent construction on this side, which would only prove two
    // independent implementations agree by coincidence rather than that
    // they're actually computing the same thing. Paired with a
    // throwaway synthetic user that exists ONLY to exercise the render
    // function — never a real user's UUID/password, and never printed.
    let synthetic_user = CompatUser {
        id: "doctor-l4-synthetic".into(),
        name: "doctor-l4-synthetic".into(),
        enabled: true,
        vless_uuid: "00000000-0000-4000-8000-000000000000".into(),
        hysteria2_password: SecretString::new("unused"),
        subscription_token_hash_hex: String::new(),
        created_at: 0,
        expires_at: None,
    };
    let short_id = reality.short_ids.first().cloned().unwrap_or_default();
    let endpoints = compat_config::render::standard_endpoints(
        &cfg.public_host,
        cfg.reality.listen_port,
        cfg.hysteria2.listen_port,
        &reality.public_key_hex,
        &short_id,
        &reality.handshake_server,
    );
    let client_doc = match render_singbox_client_subscription(&synthetic_user, &endpoints) {
        Ok(d) => d,
        Err(e) => {
            report_check(
                CheckStatus::Fail,
                "L4",
                format!("failed to render client subscription for coherence check: {e}"),
            );
            *failures += 1;
            return;
        }
    };

    let server_short_ids: Vec<String> = fresh_server_doc["inbounds"][0]["tls"]["reality"]
        ["short_id"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let client_short_id = client_doc["outbounds"][0]["tls"]["reality"]["short_id"]
        .as_str()
        .unwrap_or("");
    let client_pubkey = client_doc["outbounds"][0]["tls"]["reality"]["public_key"]
        .as_str()
        .unwrap_or("");

    if server_short_ids.iter().any(|s| s == client_short_id)
        && client_pubkey == reality.public_key_hex
    {
        report_check(
            CheckStatus::Ok,
            "L4",
            "subscription render coherence: the client subscription's public_key/short_id match \
             what the current server config accepts",
        );
    } else {
        report_check(
            CheckStatus::Fail,
            "L4",
            format!(
                "subscription render coherence FAILED: the client subscription would advertise \
                 short_id={client_short_id:?}, but the server config accepts short_id(s)={server_short_ids:?} \
                 — a real client using this subscription would fail REALITY's handshake (\"processed \
                 invalid connection\"). This indicates a bug in the render code paths, not a config \
                 file problem — do not attempt to fix by rotating keys."
            ),
        );
        *failures += 1;
    }

    let target = cfg.singbox_config_file();
    if !target.exists() {
        report_check(
            CheckStatus::Warn,
            "L4",
            "sing-box config.json not yet rendered — on-disk drift check skipped",
        );
        return;
    }
    let on_disk_bytes = match std::fs::read(&target) {
        Ok(b) => b,
        Err(e) => {
            report_check(
                CheckStatus::Warn,
                "L4",
                format!("could not read on-disk sing-box config.json to check for drift: {e}"),
            );
            return;
        }
    };
    let on_disk: serde_json::Value = match serde_json::from_slice(&on_disk_bytes) {
        Ok(v) => v,
        Err(e) => {
            report_check(
                CheckStatus::Warn,
                "L4",
                format!("could not parse on-disk sing-box config.json to check for drift: {e}"),
            );
            return;
        }
    };
    if on_disk == fresh_server_doc {
        report_check(
            CheckStatus::Ok,
            "L4",
            "on-disk sing-box config.json exactly matches the complete authorization/key/cert \
             document current state would render (not stale)",
        );
    } else {
        report_check(
            CheckStatus::Fail,
            "L4",
            "on-disk sing-box config.json does NOT exactly match current users/expiry/REALITY/\
             Hysteria state — sing-box (as of its last reload) may be enforcing stale \
             authorization or key material. Run \
             `vpn-admin render-config` to resync, then confirm with `systemctl status sing-box`.",
        );
        *failures += 1;
    }
}

/// The check above (`check_l4_subscription_coherence`) can only prove
/// what a FRESH read of the current files would produce — it cannot see
/// what the ALREADY-RUNNING `vpn-subscription` PROCESS actually has
/// cached in memory, because `vpn-subscription` reads its REALITY public
/// key/short_id from disk exactly once, at its own startup, and has no
/// config-reload path (`services/subscription/src/main.rs`). A process
/// that started before the on-disk keys last changed — a restart that
/// silently failed, a `reload-or-restart` that degraded to `reload`
/// against a unit that doesn't actually support it, a code path this
/// audit didn't cover — would pass every check above and still be
/// serving stale key material to every real client that fetches a
/// subscription from it right now. This is the exact incident class
/// that motivated this whole diagnostic layer; a check that can't
/// observe the live process isn't actually verifying it.
///
/// This check closes that gap by asking the running process itself: it
/// fetches `GET /internal/state-fingerprint` from `vpn-subscription`'s
/// own loopback listener (`services/subscription/src/lib.rs`,
/// `state_fingerprint`) — a SHA-256 fingerprint of its actual in-memory
/// `AppState.endpoints`, never the raw key/short_id — and compares it
/// against a fingerprint of what a fresh read of the current files would
/// produce (via the SAME `standard_endpoints`/`endpoints_fingerprint`
/// functions `vpn-subscription` itself uses). Agreement is a hard
/// `[FAIL]`-eligible property, not advisory: a mismatch means a real
/// client fetching a subscription right now gets different REALITY key
/// material than `sing-box` is enforcing, which is precisely how the
/// original incident manifested. Unreachable (service not running, not
/// on loopback here, firewalled) is `[WARN]`, not `[FAIL]` — that is a
/// "cannot verify" outcome, not a proven mismatch, and this function
/// must not conflate the two.
fn check_l4_live_subscription_process_state(cfg: &DeploymentConfig, failures: &mut u32) {
    let reality = match load_reality_params(cfg) {
        Ok(r) => r,
        Err(e) => {
            report_check(
                CheckStatus::Warn,
                "L4",
                format!("skipping live subscription-process check: {e}"),
            );
            return;
        }
    };
    let short_id = reality.short_ids.first().cloned().unwrap_or_default();
    let expected_endpoints = compat_config::render::standard_endpoints(
        &cfg.public_host,
        cfg.reality.listen_port,
        cfg.hysteria2.listen_port,
        &reality.public_key_hex,
        &short_id,
        &reality.handshake_server,
    );
    let expected_fingerprint = compat_config::render::endpoints_fingerprint(&expected_endpoints);

    let response = http_get_local_json(
        cfg.subscription.listen_port,
        "/internal/state-fingerprint",
        std::time::Duration::from_millis(800),
    );
    let live_fingerprint = match response {
        Ok(json) => match json["endpoints_fingerprint_sha256"].as_str() {
            Some(fp) => fp.to_string(),
            None => {
                report_check(
                    CheckStatus::Warn,
                    "L4",
                    "vpn-subscription's /internal/state-fingerprint responded with an unexpected \
                     shape — cannot verify the running process's live state (this may indicate a \
                     version skew between vpn-admin and vpn-subscription-svc).",
                );
                return;
            }
        },
        Err(e) => {
            report_check(
                CheckStatus::Warn,
                "L4",
                format!(
                    "cannot reach the running vpn-subscription process on \
                     127.0.0.1:{} to verify its LIVE state (not the same as proving it's stale — \
                     only that this check could not observe it): {e}",
                    cfg.subscription.listen_port
                ),
            );
            return;
        }
    };

    if live_fingerprint == expected_fingerprint {
        report_check(
            CheckStatus::Ok,
            "L4",
            "the ALREADY-RUNNING vpn-subscription process's live in-memory state matches current \
             REALITY key files/deployment config (verified via its own /internal/state-fingerprint \
             endpoint, not just a fresh disk read that a stale process would also pass)",
        );
    } else {
        report_check(
            CheckStatus::Fail,
            "L4",
            "the RUNNING vpn-subscription process is serving STALE state that does not match the \
             current REALITY key files/deployment config — every real client fetching a \
             subscription right now receives different key material than sing-box is enforcing. \
             This is the exact production incident class. Fix: `systemctl restart vpn-subscription` \
             (a plain reload is not sufficient — this process has no config-reload path), then \
             re-run `vpn-admin doctor` to confirm the fingerprints now agree.",
        );
        *failures += 1;
    }
}

fn tcp_port_reachable(host: &str, port: u16, timeout: std::time::Duration) -> bool {
    use std::net::ToSocketAddrs;
    let addr = match format!("{host}:{port}").to_socket_addrs() {
        Ok(mut it) => match it.next() {
            Some(a) => a,
            None => return false,
        },
        Err(_) => return false,
    };
    std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
}

/// Minimal UDP DNS probe for basic outbound-UDP capability checks.
///
/// Returns Some(true) if a UDP DNS response was successfully received,
/// Some(false) if the probe completed with no response (indicating a
/// likely block), and None if the environment prevented running the
/// probe (socket bind failure, unsupported platform, etc.). Uses a
/// plain DNS A query for `example.com` sent to the provided IP
/// (e.g. "1.1.1.1" or "8.8.8.8").
fn build_dns_query(name: &str) -> Vec<u8> {
    let mut q = Vec::new();
    // ID
    q.push(0x12);
    q.push(0x34);
    // Flags: standard query, recursion desired
    q.push(0x01);
    q.push(0x00);
    // QDCOUNT=1
    q.push(0x00);
    q.push(0x01);
    // ANCOUNT, NSCOUNT, ARCOUNT = 0
    q.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    for label in name.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0x00); // end of QNAME
    // QTYPE A
    q.push(0x00);
    q.push(0x01);
    // QCLASS IN
    q.push(0x00);
    q.push(0x01);
    q
}

fn udp_dns_probe(resolver_ip: &str, timeout: std::time::Duration) -> Option<bool> {
    use std::net::{SocketAddr, UdpSocket};

    let query = build_dns_query("example.com");

    // Bind to an ephemeral UDP socket on all interfaces (IPv4).
    let bind_addr = match "0.0.0.0:0".parse::<SocketAddr>() {
        Ok(a) => a,
        Err(_) => return None,
    };
    let socket = match UdpSocket::bind(bind_addr) {
        Ok(s) => s,
        Err(_) => return None,
    };
    let _ = socket.set_read_timeout(Some(timeout));
    let _ = socket.set_write_timeout(Some(timeout));

    let target = format!("{resolver_ip}:53");
    let target_addr = match target.parse::<SocketAddr>() {
        Ok(a) => a,
        Err(_) => return None,
    };

    if socket.send_to(&query, target_addr).is_err() {
        return Some(false);
    }
    let mut buf = [0u8; 512];
    match socket.recv_from(&mut buf) {
        Ok((n, _)) => Some(n > 0),
        Err(_) => Some(false),
    }
}

/// IPv6 variant of the UDP DNS probe. Binds to the IPv6 unspecified
/// address and dials a bracketed IPv6 resolver address like
/// `[2606:4700:4700::1111]:53`.
fn udp_dns_probe_v6(resolver_ip: &str, timeout: std::time::Duration) -> Option<bool> {
    use std::net::{SocketAddr, UdpSocket};

    let query = build_dns_query("example.com");

    // Bind to an ephemeral UDP socket on all interfaces (IPv6).
    let bind_addr = match "[::]:0".parse::<SocketAddr>() {
        Ok(a) => a,
        Err(_) => return None,
    };
    let socket = match UdpSocket::bind(bind_addr) {
        Ok(s) => s,
        Err(_) => return None,
    };
    let _ = socket.set_read_timeout(Some(timeout));
    let _ = socket.set_write_timeout(Some(timeout));

    let target = format!("[{resolver_ip}]:53");
    let target_addr = match target.parse::<SocketAddr>() {
        Ok(a) => a,
        Err(_) => return None,
    };

    if socket.send_to(&query, target_addr).is_err() {
        return Some(false);
    }
    let mut buf = [0u8; 512];
    match socket.recv_from(&mut buf) {
        Ok((n, _)) => Some(n > 0),
        Err(_) => Some(false),
    }
}

/// Try multiple resolver candidates with retries and inter-attempt delay.
///
/// Returns Some(true) if any candidate returned a positive response,
/// Some(false) if probes ran but none responded, and None if every
/// attempt failed to run (e.g., socket bind errors across attempts).
fn run_udp_probe_candidates_with_probe<F>(
    candidates: &[&str],
    timeout: std::time::Duration,
    retries: usize,
    delay: std::time::Duration,
    mut probe: F,
) -> Option<bool>
where
    F: FnMut(&str, std::time::Duration) -> Option<bool>,
{
    let mut any_ran = false;
    for &cand in candidates {
        for attempt in 0..retries {
            let outcome = probe(cand, timeout);
            match outcome {
                Some(true) => return Some(true),
                Some(false) => {
                    any_ran = true;
                    // try again or next resolver
                }
                None => {
                    // probe could not be executed for this candidate/attempt
                }
            }
            if attempt + 1 < retries {
                std::thread::sleep(delay);
            }
        }
    }
    if !any_ran {
        None
    } else {
        Some(false)
    }
}

fn run_udp_probe_candidates(
    candidates: &[&str],
    timeout: std::time::Duration,
    retries: usize,
    delay: std::time::Duration,
) -> Option<bool> {
    run_udp_probe_candidates_with_probe(candidates, timeout, retries, delay, |cand, to| {
        if cand.contains(":") {
            udp_dns_probe_v6(cand, to)
        } else {
            udp_dns_probe(cand, to)
        }
    })
}

#[cfg(test)]
mod udp_probe_tests {
    use super::*;

    #[test]
    fn build_dns_query_contains_labels() {
        let q = build_dns_query("example.com");
        // Should contain label lengths 7 and 3 followed by the ascii bytes
        assert!(q.windows(2).any(|w| w == [7, b'e']));
        assert!(q.windows(2).any(|w| w == [3, b'c']));
    }

    #[test]
    fn run_udp_probe_candidates_with_probe_logic() {
        // Simulate first resolver failing twice, second resolver succeeding on first attempt
        let candidates = ["1.2.3.4", "5.6.7.8"];
        let mut calls = 0;
        let probe = |cand: &str, _timeout: std::time::Duration| -> Option<bool> {
            calls += 1;
            if cand == "1.2.3.4" {
                Some(false)
            } else if cand == "5.6.7.8" {
                Some(true)
            } else {
                None
            }
        };
        let res = run_udp_probe_candidates_with_probe(&candidates, std::time::Duration::from_millis(10), 2, std::time::Duration::from_millis(1), probe);
        assert_eq!(res, Some(true));
        assert!(calls >= 3);
    }
}

/// Minimal, dependency-free HTTP/1.0 GET over loopback: connect, send a
/// bare request with `Connection: close`, read the whole response (the
/// server closes after writing it, since we asked for that), split the
/// status line, headers, and body, and parse the body as JSON. Good
/// enough for talking to `vpn-subscription`'s own loopback-only
/// `/internal/state-fingerprint` — not a general-purpose HTTP client,
/// and deliberately not pulling in `reqwest` just for one same-host GET.
fn http_get_local_json(
    port: u16,
    path: &str,
    timeout: std::time::Duration,
) -> Result<serde_json::Value> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}")
            .parse()
            .context("building loopback address")?,
        timeout,
    )
    .with_context(|| format!("connecting to 127.0.0.1:{port}"))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .context("writing HTTP request")?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .context("reading HTTP response")?;
    let text = String::from_utf8_lossy(&response);
    let mut parts = text.splitn(2, "\r\n\r\n");
    let head = parts.next().unwrap_or("");
    let body = parts.next().unwrap_or("");
    let status_line = head.lines().next().unwrap_or("");
    if !status_line.contains(" 200 ") {
        bail!("unexpected HTTP status from {path}: {status_line:?}");
    }
    serde_json::from_str(body).with_context(|| format!("parsing JSON body from {path}: {body:?}"))
}

/// Best-effort L5/L6, only run with `doctor --protocol`: spin up the
/// REAL `sing-box` binary as a throwaway client process pointed at this
/// server's OWN VLESS+REALITY listener on `127.0.0.1`, using the live
/// REALITY public_key/short_id read from the CURRENT on-disk key files
/// — the same source `vpn-subscription` reads at its own startup, but
/// note this self-test builds its client config directly from those
/// files itself, so a clean PASS here proves the on-disk key material
/// is internally coherent with what `sing-box` enforces, but does NOT
/// by itself prove the ALREADY-RUNNING `vpn-subscription` process is
/// advertising the same thing — that is what
/// `check_l4_live_subscription_process_state` (run unconditionally,
/// every `doctor` invocation) exists to verify separately, by asking
/// that process directly rather than re-deriving from disk. Uses an
/// enabled, unexpired user's VLESS UUID and requires application bytes
/// from the local subscription health endpoint. A successful SOCKS
/// CONNECT reply alone is not evidence that VLESS authentication was
/// accepted by the server.
///
/// Gated on `sing-box` being present AND the port actually being
/// reachable on loopback; anything else is `[WARN] cannot self-test:
/// <reason>` — a self-test that can't run here says so, it does not
/// fake a pass.
///
/// A client-side REALITY verification rejection is a hard failure. A
/// timeout or missing prerequisite is a warning by default because it
/// can be environmental; `--require-protocol` promotes that uncertainty
/// to a failure for installation acceptance checks.
fn report_protocol_unavailable(
    require_protocol: bool,
    failures: &mut u32,
    message: impl AsRef<str>,
) {
    if require_protocol {
        report_check(CheckStatus::Fail, "L5-6", message.as_ref());
        *failures += 1;
    } else {
        report_check(CheckStatus::Warn, "L5-6", message.as_ref());
    }
}

fn listener_reported_by_ss(port: u16, udp: bool) -> Option<bool> {
    let args = if udp { ["-H", "-lun"] } else { ["-H", "-ltn"] };
    let output = std::process::Command::new("ss").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let suffix = format!(":{port}");
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().any(|line| {
        line.split_whitespace().any(|field| {
            field == suffix || field.ends_with(&suffix) || field.contains(&format!("]{suffix}"))
        })
    }))
}

fn check_l5_l6_protocol_selftest(
    cfg: &DeploymentConfig,
    failures: &mut u32,
    require_protocol: bool,
) {
    if !cfg.singbox_binary.exists() {
        report_protocol_unavailable(
            require_protocol,
            failures,
            format!(
                "cannot self-test: sing-box binary not found at {:?}",
                cfg.singbox_binary
            ),
        );
        return;
    }
    let reality = match load_reality_params(cfg) {
        Ok(r) => r,
        Err(e) => {
            report_protocol_unavailable(
                require_protocol,
                failures,
                format!("cannot self-test: {e}"),
            );
            return;
        }
    };
    let users = match store::load_users(&cfg.users_file()) {
        Ok(users) => users,
        Err(e) => {
            report_protocol_unavailable(
                require_protocol,
                failures,
                format!("cannot self-test: failed to load users: {e}"),
            );
            return;
        }
    };
    let now = UnixSeconds::now().0 as i64;
    let Some(test_user) = users.iter().find(|user| user.is_active(now)) else {
        report_protocol_unavailable(
            require_protocol,
            failures,
            "cannot self-test: there is no enabled, unexpired VLESS user",
        );
        return;
    };
    let port = cfg.reality.listen_port;

    match run_reality_client_selftest(cfg, &reality, test_user, port) {
        Ok(RealitySelfTestOutcome::Pass) => report_check(
            CheckStatus::Ok,
            "L5-6",
            format!(
                "protocol self-test: a throwaway sing-box client using the CURRENT REALITY \
                 public_key/short_id and an active VLESS user completed a full handshake through \
                 127.0.0.1:{port} and returned application bytes end-to-end"
            ),
        ),
        Ok(RealitySelfTestOutcome::HandshakeRejected) => {
            report_check(
                CheckStatus::Fail,
                "L5-6",
                format!(
                    "protocol self-test FAILED: a throwaway sing-box client using the CURRENT \
                     REALITY public_key/short_id could not complete a handshake through \
                     127.0.0.1:{port}. A real Hiddify client using this same key material would \
                     fail identically.\n\
                     \n\
                     TWO DIFFERENT CAUSES produce this, and sing-box logs the SAME message \
                     (\"REALITY: processed invalid connection\") for both — do not assume the \
                     first one:\n\
                     \n\
                     (a) The REALITY key material really is mismatched. The L4 checks above test \
                     exactly that; if they passed, this is NOT your cause.\n\
                     \n\
                     (b) The configured handshake_server (\"{decoy}\") returns a TLS 1.3 flight \
                     that sing-box's REALITY implementation refuses. It rejects ANY record larger \
                     than its hard-coded 8192-byte budget (metacubex/utls reality.go: \
                     `if handshakeLen > int(realitySize) {{ break f }}`), which an oversized \
                     certificate chain easily exceeds. Authentication SUCCEEDS and the connection \
                     is still dropped. This is edge- and CDN-dependent, so it can appear without \
                     any change on your side. Try a different handshake_server and re-run this \
                     check.",
                    decoy = reality.handshake_server
                ),
            );
            *failures += 1;
        }
        Ok(RealitySelfTestOutcome::Inconclusive) => report_protocol_unavailable(
            require_protocol,
            failures,
            "protocol self-test INCONCLUSIVE: the client did not return an HTTP success response \
             through the live VLESS+REALITY listener. This can be an authentication, routing, \
             decoy, listener, or transient failure; inspect both sing-box processes' logs.",
        ),
        Err(e) => report_protocol_unavailable(
            require_protocol,
            failures,
            format!("cannot self-test: {e}"),
        ),
    }
}

/// Runs the actual throwaway client + SOCKS probe for
/// `check_l5_l6_protocol_selftest`. `Ok(RealitySelfTestOutcome)` is a
/// verdict about the relay/server; `Err` means the self-test harness
/// itself failed to set up (never a verdict about the server).
fn run_reality_client_selftest(
    cfg: &DeploymentConfig,
    reality: &RealityServerParams,
    test_user: &CompatUser,
    reality_port: u16,
) -> Result<RealitySelfTestOutcome> {
    let short_id = reality.short_ids.first().cloned().unwrap_or_default();

    // Reserve a free loopback port for the throwaway client's local
    // SOCKS inbound, then release it immediately before sing-box binds
    // it — a small, unavoidable race in a best-effort self-test, not a
    // correctness requirement.
    let local_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("reserving a local port for the throwaway client")?;
        listener.local_addr()?.port()
    };

    let client_config = json!({
        "log": { "level": "error" },
        "inbounds": [
            { "type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": local_port }
        ],
        "outbounds": [
            {
                "type": "vless",
                "tag": "reality-selftest",
                "server": "127.0.0.1",
                "server_port": reality_port,
                "uuid": test_user.vless_uuid,
                "flow": "xtls-rprx-vision",
                "tls": {
                    "enabled": true,
                    "server_name": reality.handshake_server,
                    "utls": { "enabled": true, "fingerprint": "chrome" },
                    "reality": {
                        "enabled": true,
                        "public_key": reality.public_key_hex,
                        "short_id": short_id,
                    }
                }
            },
            { "type": "direct", "tag": "direct" }
        ],
        "route": { "final": "reality-selftest" }
    });

    let tmp = tempfile::NamedTempFile::new().context("creating throwaway client config file")?;
    std::fs::write(tmp.path(), serde_json::to_vec_pretty(&client_config)?)
        .context("writing throwaway client config")?;

    let mut child = std::process::Command::new(&cfg.singbox_binary)
        .arg("run")
        .arg("-c")
        .arg(tmp.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawning throwaway sing-box client")?;

    // Drain stderr on a background thread as it's produced (not after
    // the fact) — the pipe has a small OS buffer, and this process can
    // log continuously, so reading only after killing the child risks
    // blocking on a full pipe the child is also blocked writing to.
    // sing-box's own `errors.New("REALITY: processed invalid
    // connection")` (github.com/metacubex/utls, reality.go) and its
    // client-side counterpart `"reality verification failed"` are plain
    // static strings with no secret interpolated — safe to capture and
    // pattern-match on, unlike anything at `debug`/`trace` log level
    // (never enabled here; `client_config` above sets `"level": "error"`).
    let stderr_pipe = child.stderr.take().expect("stderr was piped");
    let stderr_capture = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        let mut pipe = stderr_pipe;
        let _ = pipe.read_to_string(&mut buf);
        buf
    });

    // Always kill the throwaway client before returning on every path
    // below — never leave an orphaned sing-box process behind from a
    // diagnostic command.
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    // Poll for the client's local SOCKS inbound to come up.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut client_bound = true;
    while !tcp_port_reachable(
        "127.0.0.1",
        local_port,
        std::time::Duration::from_millis(100),
    ) {
        if std::time::Instant::now() >= deadline {
            client_bound = false;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    let relay_ok = client_bound
        && socks5_http_get_succeeds(
            local_port,
            "127.0.0.1",
            cfg.subscription.listen_port,
            "/healthz",
            std::time::Duration::from_secs(4),
        );

    // Kill+reap now (releases the stderr pipe's write end so the reader
    // thread's `read_to_string` returns), THEN read what it captured —
    // ordering matters, joining first would deadlock against a child
    // that's still alive and still writing. Wrapping in the drop guard
    // even though we kill explicitly: if anything above this point had
    // returned early via `?`, the guard is what would have caught it —
    // kept for that safety net even though this exact path always kills
    // explicitly too (a second kill/wait on an already-reaped child in
    // `Drop` is a harmless no-op, ignored the same way as everywhere
    // else in this function).
    let mut guard = KillOnDrop(child);
    let _ = guard.0.kill();
    let _ = guard.0.wait();
    let captured_stderr = stderr_capture.join().unwrap_or_default();

    if relay_ok {
        return Ok(RealitySelfTestOutcome::Pass);
    }
    // A definitive signal that the handshake does not work — our own
    // throwaway client, built from the CURRENT REALITY public_key/short_id
    // exactly as a real subscription would hand a real client, could not
    // complete it.
    //
    // What it is NOT is evidence about the CAUSE. "processed invalid
    // connection" is sing-box's message for any connection that fails to
    // complete REALITY's hijack — including one whose key material is
    // perfect but whose handshake_server returned an over-budget TLS record
    // (see `crates/compat-config/tests/reality_decoy_budget.rs`). The
    // caller reports both possibilities; it must not claim a key mismatch.
    // Distinct from a bare timeout, which proves nothing either way (see
    // the caller's WARN path).
    if captured_stderr.contains("reality verification failed")
        || captured_stderr.contains("processed invalid connection")
    {
        return Ok(RealitySelfTestOutcome::HandshakeRejected);
    }
    Ok(RealitySelfTestOutcome::Inconclusive)
}

/// See `run_reality_client_selftest`'s doc comment for how these are
/// distinguished.
///
/// `HandshakeRejected` is a hard verdict that the handshake does not work,
/// but deliberately NOT a verdict about *why*: sing-box emits the same
/// "processed invalid connection" for a key/short_id mismatch and for a
/// handshake_server whose TLS records exceed REALITY's 8192-byte budget
/// (auth succeeds, connection still dropped). Conflating the two is what
/// sent three separate investigations after the wrong cause. `Inconclusive`
/// means exactly that
/// — a timeout with no corroborating client-side rejection proves
/// nothing about which layer, if any, is broken (could be no outbound
/// path to the REALITY decoy target, an unrelated transient failure,
/// etc.), so it must never be reported as if it were either verdict.
enum RealitySelfTestOutcome {
    Pass,
    HandshakeRejected,
    Inconclusive,
}

/// Minimal, dependency-free SOCKS5 HTTP probe. It does not treat SOCKS
/// `REP=0` as success: VLESS has no positive authentication ACK, so the
/// server can reject the UUID after the local proxy accepted CONNECT.
/// Only an HTTP 200 received through the tunnel proves authentication
/// and application-data relay by the live server.
fn socks5_http_get_succeeds(
    local_port: u16,
    target_host: &str,
    target_port: u16,
    target_path: &str,
    timeout: std::time::Duration,
) -> bool {
    use std::io::{Read, Write};
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", local_port)) else {
        return false;
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return false;
    }
    // Greeting: version 5, 1 method, no-auth (0x00).
    if stream.write_all(&[0x05, 0x01, 0x00]).is_err() {
        return false;
    }
    let mut resp = [0u8; 2];
    if stream.read_exact(&mut resp).is_err() || resp != [0x05, 0x00] {
        return false;
    }
    // CONNECT request, domain-name address type.
    let host_bytes = target_host.as_bytes();
    if host_bytes.is_empty() || host_bytes.len() > 255 {
        return false;
    }
    let mut req = vec![0x05u8, 0x01, 0x00, 0x03, host_bytes.len() as u8];
    req.extend_from_slice(host_bytes);
    req.push((target_port >> 8) as u8);
    req.push((target_port & 0xff) as u8);
    if stream.write_all(&req).is_err() {
        return false;
    }
    // Reply header: ver, rep, rsv, atyp. Consume the complete reply before
    // writing application data.
    let mut head = [0u8; 4];
    if stream.read_exact(&mut head).is_err() || head[0] != 0x05 || head[1] != 0x00 {
        return false;
    }
    let trailing_len = match head[3] {
        0x01 => 6,
        0x04 => 18,
        0x03 => {
            let mut len = [0u8; 1];
            if stream.read_exact(&mut len).is_err() {
                return false;
            }
            usize::from(len[0]) + 2
        }
        _ => return false,
    };
    let mut trailing = vec![0u8; trailing_len];
    if stream.read_exact(&mut trailing).is_err() {
        return false;
    }

    let request =
        format!("GET {target_path} HTTP/1.0\r\nHost: {target_host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = Vec::new();
    if stream.read_to_end(&mut response).is_err() {
        return false;
    }
    response.starts_with(b"HTTP/1.0 200") || response.starts_with(b"HTTP/1.1 200")
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

    // Own the destination from its first byte. `create_new` refuses both
    // pre-existing files and symlinks, avoiding predictable-name clobbering
    // and attacker-owned output. The Rust tar writer writes only through
    // this already-open descriptor.
    let mut created = false;
    let mut create_archive = || -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&dest).with_context(|| {
            format!("securely creating backup {dest:?} (destination must not already exist)")
        })?;
        created = true;
        let mut archive = tar::Builder::new(file);
        archive.follow_symlinks(false);
        archive
            .append_dir_all(".", staging.path())
            .context("writing backup archive contents")?;
        let file = archive.into_inner().context("finishing backup archive")?;
        file.sync_all().context("syncing backup archive")?;
        Ok(())
    };
    if let Err(error) = create_archive() {
        if created {
            let _ = std::fs::remove_file(&dest);
        }
        return Err(error);
    }

    println!("Backup written to {dest:?}.");
    println!(
        "This archive contains secrets (REALITY private key, Hysteria2 TLS key, user \
         credential hashes) — store it as securely as the live server."
    );
    Ok(())
}

fn tempdir_here() -> Result<tempfile::TempDir> {
    tempfile::tempdir().context("creating temporary staging directory")
}

/// Refuse to restore from an archive containing anything that is not a
/// plain file or directory.
///
/// Restore reads a handful of known relative paths out of the extracted
/// tree. A hostile or corrupted archive can plant other entry types there,
/// and each one is a distinct failure:
///   * **symlink** — reading it yields whatever the target happens to be
///     instead of the archive's own content.
///   * **FIFO** — `std::fs::read` on it blocks forever with no writer, and
///     restore holds the global `/run/lock/vpn1.lock` while it does, so
///     every subsequent `vpn user …`, rotation and restore deadlocks too.
///     (Reproduced: the process simply never returns.)
///   * **character/block device** — `read` on e.g. `/dev/zero` consumes
///     memory without bound until the OOM killer intervenes, which on a VPS
///     means it takes sing-box or nginx with it.
///
/// `tar` runs as root during restore, so all of these are really created.
/// Rejecting by entry type up front is cheaper and more complete than
/// hardening each read site.
fn normalized_archive_path(path: &std::path::Path) -> Result<PathBuf> {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("backup archive contains unsafe path {path:?}")
            }
        }
    }
    Ok(normalized)
}

/// Validate each archive header before extracting that entry into the
/// private staging directory. This rejects traversal, duplicate names,
/// links/devices/FIFOs, unexpected files, and archive bombs before any
/// live deployment path is touched.
fn extract_validated_backup(archive_path: &std::path::Path, dir: &std::path::Path) -> Result<()> {
    use std::collections::HashSet;

    const MAX_ENTRIES: usize = 32;
    const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
    const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
    const ALLOWED: &[&str] = &[
        "users",
        "users/users.json",
        "deployment.toml",
        "reality",
        "reality/private.key",
        "reality/public.key",
        "reality/short_id.txt",
        "hysteria",
        "hysteria/cert.pem",
        "hysteria/key.pem",
    ];

    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("opening backup archive {archive_path:?}"))?;
    let mut archive = tar::Archive::new(file);
    let mut seen = HashSet::new();
    let mut total_bytes = 0u64;
    let mut count = 0usize;
    for entry in archive
        .entries()
        .context("reading backup archive headers")?
    {
        let mut entry = entry.context("reading backup archive entry")?;
        count += 1;
        if count > MAX_ENTRIES {
            bail!("backup archive contains more than {MAX_ENTRIES} entries");
        }
        let raw_path = entry.path().context("reading backup entry path")?;
        let path = normalized_archive_path(&raw_path)?;
        if path.as_os_str().is_empty() {
            if !entry.header().entry_type().is_dir() {
                bail!("backup archive root entry is not a directory");
            }
            continue;
        }
        if !seen.insert(path.clone()) {
            bail!("backup archive contains duplicate entry {path:?}");
        }
        if !ALLOWED
            .iter()
            .any(|allowed| path == std::path::Path::new(allowed))
        {
            bail!("backup archive contains unexpected entry {path:?}");
        }
        let entry_type = entry.header().entry_type();
        if !(entry_type.is_file() || entry_type.is_dir()) {
            bail!(
                "backup archive entry {path:?} is not a regular file or directory; links and \
                 special files (symlink, hard link, FIFO, device) are forbidden"
            );
        }
        let size = entry.size();
        if size > MAX_ENTRY_BYTES {
            bail!("backup archive entry {path:?} is too large ({size} bytes)");
        }
        total_bytes = total_bytes
            .checked_add(size)
            .context("backup archive size overflow")?;
        if total_bytes > MAX_TOTAL_BYTES {
            bail!("backup archive expands beyond {MAX_TOTAL_BYTES} bytes");
        }
        if !entry
            .unpack_in(dir)
            .with_context(|| format!("extracting backup entry {path:?}"))?
        {
            bail!("backup entry {path:?} would escape the staging directory");
        }
    }
    Ok(())
}

fn cmd_restore(
    cfg: &DeploymentConfig,
    config_path: &std::path::Path,
    archive: &std::path::Path,
) -> Result<()> {
    let staging = tempdir_here()?;
    extract_validated_backup(archive, staging.path())
        .context("validating and extracting backup archive")?;

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
    // The REALITY triple must be restored as a SET. Restoring a new
    // private.key next to the live host's OLD public.key/short_id produces
    // a guaranteed split-brain: sing-box enforces the restored private key
    // while vpn-subscription keeps advertising the old public half, and
    // every client fails REALITY's handshake. Previously the last four
    // targets were restored only `if src.exists()`, so an archive with a
    // private key and nothing else reported success.
    for required in ["reality/public.key", "reality/short_id.txt"] {
        if !staging.path().join(required).exists() {
            bail!(
                "archive contains reality/private.key but not {required} — refusing to restore \
                 a partial REALITY keyset, which would leave the server enforcing one key while \
                 the subscription service advertises another"
            );
        }
    }
    let restored_private = std::fs::read_to_string(&reality_key_path)
        .context("reading restored REALITY private key")?;
    let restored_public = std::fs::read_to_string(staging.path().join("reality/public.key"))
        .context("reading restored REALITY public key")?;
    credentials::validate_reality_keypair(restored_private.trim(), restored_public.trim())
        .map_err(|error| anyhow::anyhow!(error))
        .context(
            "restored REALITY private/public keys do not form one X25519 keypair — refusing to \
             install a split keyset",
        )?;
    let hy_cert = staging.path().join("hysteria/cert.pem");
    let hy_key = staging.path().join("hysteria/key.pem");
    if hy_cert.exists() != hy_key.exists() {
        bail!(
            "archive contains only one half of the Hysteria2 TLS pair — refusing to restore a \
             mismatched certificate/key"
        );
    }

    let singbox_mgr = CompatibilityServiceManager::default();
    let sub_mgr = CompatibilityServiceManager::new("vpn-subscription");
    if !offline_mutation_allowed()
        && (!singbox_mgr.is_available()
            || !singbox_mgr.is_unit_installed()
            || !sub_mgr.is_available()
            || !sub_mgr.is_unit_installed())
    {
        bail!(
            "refusing restore: both sing-box.service and vpn-subscription.service must be \
             installed and controllable so restored authorization/key state can be committed \
             atomically. VPN1_ALLOW_OFFLINE_MUTATION=1 is only for explicit offline recovery."
        );
    }

    // Only after validation: copy into place.
    std::fs::create_dir_all(cfg.reality_dir())?;
    std::fs::create_dir_all(cfg.hysteria_dir())?;
    std::fs::create_dir_all(cfg.users_file().parent().unwrap())?;

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

    // Back EVERYTHING up before touching any of it, so a failure part-way
    // through can put the deployment back exactly as it was. `restore` was
    // the only mutating command with no rollback at all, while the rotation
    // path right next to it builds precisely this scaffolding.
    let users_path = cfg.users_file();
    let users_backup = backup_for_rotate(&users_path)?;
    let users_had_backup = users_backup.is_some();
    let mut prepared_backups: Vec<PathBuf> = users_backup.into_iter().collect();
    let mut backed_up: Vec<(PathBuf, bool)> = Vec::new();
    for (_, dest) in &restore_targets {
        match backup_for_rotate(dest) {
            Ok(backup) => {
                let existed = backup.is_some();
                prepared_backups.extend(backup);
                backed_up.push((dest.clone(), existed));
            }
            Err(error) => {
                for backup in prepared_backups {
                    let _ = std::fs::remove_file(backup);
                }
                return Err(error).context(
                    "failed to prepare the complete restore transaction; live state was not changed",
                );
            }
        }
    }
    let rollback = |backed_up: &[(PathBuf, bool)]| -> Result<()> {
        let mut errors = Vec::new();
        for (dest, existed) in backed_up {
            let result = if *existed {
                restore_from_rotate_backup(dest)
            } else {
                std::fs::remove_file(dest)
                    .or_else(|error| {
                        if error.kind() == std::io::ErrorKind::NotFound {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    })
                    .map_err(anyhow::Error::from)
            };
            if let Err(error) = result {
                errors.push(format!("restore {dest:?}: {error}"));
            }
        }
        let users_result = if users_had_backup {
            restore_from_rotate_backup(&users_path)
        } else {
            std::fs::remove_file(&users_path)
                .or_else(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })
                .map_err(anyhow::Error::from)
        };
        if let Err(error) = users_result {
            errors.push(format!("restore {users_path:?}: {error}"));
        }
        if let Err(error) = regenerate_singbox_config(cfg, true) {
            errors.push(format!("reload previous sing-box authorization: {error:#}"));
        }
        if sub_mgr.is_available() && sub_mgr.is_unit_installed() {
            if let Err(error) = sub_mgr.reload_and_verify() {
                errors.push(format!("restart previous subscription state: {error:#}"));
            }
        } else if !offline_mutation_allowed() {
            errors.push("vpn-subscription.service unavailable during rollback".into());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            bail!(errors.join("; "))
        }
    };

    let install_all = || -> Result<()> {
        store::save_users_atomic(&users_path, &restored_users)?;
        apply_restored_file_policy(&users_path, "vpn-subscription")?;
        for (rel, dest) in &restore_targets {
            let src = staging.path().join(rel);
            if !src.exists() {
                continue;
            }
            // Read-and-write rather than `fs::copy`. `fs::copy` propagates
            // the SOURCE file's permission bits to the destination, and the
            // source here is attacker-influenced archive metadata — a mode
            // of 04777 in a tar header became the live mode of
            // reality/private.key. It also creates the destination
            // root-owned when it doesn't already exist, which leaves
            // sing-box (running as Group=sing-box) unable to read its own
            // private key on a rebuilt host.
            let contents = std::fs::read(&src)
                .with_context(|| format!("reading {rel} from the backup archive"))?;
            let text = String::from_utf8(contents)
                .with_context(|| format!("{rel} in the backup archive is not valid UTF-8"))?;
            install_rotated_key_file(dest, &text)
                .with_context(|| format!("installing restored {rel}"))?;
            let group = if matches!(*rel, "reality/public.key" | "reality/short_id.txt") {
                "vpn-subscription"
            } else {
                "sing-box"
            };
            apply_restored_file_policy(dest, group)?;
        }
        fsync_dir(&cfg.reality_dir());
        fsync_dir(&cfg.hysteria_dir());
        Ok(())
    };

    if let Err(e) = install_all() {
        let recovery = rollback(&backed_up);
        bail!(
            "restore FAILED while installing staged state ({e:#}). {}",
            match recovery {
                Ok(()) => "Rollback restored and reloaded the complete previous state.".into(),
                Err(error) => format!(
                    "ROLLBACK ALSO FAILED ({error:#}); deployment may be inconsistent and needs \
                     manual recovery from .rotate-bak files."
                ),
            }
        );
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

    // Deliberately NOT announcing success yet: the previous ordering printed
    // "Restored N user(s)…" and only then applied the config, so a rejected
    // config produced a success line immediately followed by an error, with
    // the deployment left half-restored.
    if let Err(e) = regenerate_singbox_config(cfg, true) {
        let recovery = rollback(&backed_up);
        bail!(
            "restore FAILED while applying restored authorization ({e:#}). {}",
            match recovery {
                Ok(()) => "Rollback restored and reloaded the complete previous state.".into(),
                Err(error) => format!(
                    "ROLLBACK ALSO FAILED ({error:#}); deployment may be inconsistent and needs \
                     manual recovery from .rotate-bak files."
                ),
            }
        );
    }

    // The archive always contains REALITY private/public key + short_id
    // (checked above) and may differ from whatever key material was live
    // before this restore ran (e.g. restoring an older backup after a
    // rotation, or onto a fresh host). `regenerate_singbox_config` only
    // reloads sing-box — but vpn-subscription caches the REALITY public
    // key/short_id in memory at startup and has no config-reload path
    // (see `cmd_reality_rotate`'s doc comment for the same fact), so a
    // restore that skips this step can leave the subscription service
    // silently advertising a STALE public key while sing-box already
    // speaks the restored one — the exact split-brain P0-5 exists to
    // prevent, just reached via `restore` instead of `init --rotate`.
    if sub_mgr.is_available() && sub_mgr.is_unit_installed() {
        if let Err(e) = sub_mgr.reload_and_verify() {
            let recovery = rollback(&backed_up);
            bail!(
                "restore FAILED while restarting vpn-subscription ({e:#}). {}",
                match recovery {
                    Ok(()) => "Rollback restored and reloaded the complete previous state.".into(),
                    Err(error) => format!(
                        "ROLLBACK ALSO FAILED ({error:#}); deployment may be inconsistent and \
                         needs manual recovery from .rotate-bak files."
                    ),
                }
            );
        }
    } else {
        println!(
            "warning: systemctl/vpn-subscription.service not available — restored REALITY \
             key material was NOT picked up by the subscription service (it caches this at \
             startup, so a manual `systemctl restart vpn-subscription` is required on a real \
             deployment)."
        );
    }

    for (dest, existed) in &backed_up {
        if *existed {
            remove_rotate_backup(dest);
        }
    }
    if users_had_backup {
        remove_rotate_backup(&users_path);
    }

    println!(
        "Restored {} user(s) and REALITY/Hysteria2 material from {archive:?}.",
        restored_users.len()
    );
    println!("Restore applied and validated against the running server.");
    Ok(())
}
