//! `vpn-admin pair ...`: the two-host pairing workflow (docs/PAIRING.md).
//!
//! One protected document moves between the hosts per pairing step (a pipe or a 0600 file), carrying the
//! other node's public parameters and one dedicated credential. Secrets never appear on a command line, in a
//! log line or on a terminal; progress messages go to stderr so stdout can carry a bundle.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use compat_config::deployment::{DeploymentConfig, GoogleEgressHairpinSection, NodeRole};
use compat_config::model::{EndpointVlessFlow, PeerCredential};
use compat_config::pairing::{
    apply_peer_endpoint_toml, parse_exit_bundle, parse_hairpin_bundle,
    peer_endpoint_from_exit_bundle, BundleCredential, ExitPairingBundle, HairpinPairingBundle,
    PairedNode, PeerEndpointOutcome, EXIT_BUNDLE_SCHEMA, HAIRPIN_BUNDLE_SCHEMA,
};
use compat_config::store;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum PairCommands {
    /// EXIT: issue a dedicated credential for one relay user and emit the exit pairing bundle. The credential
    /// is a plain exit user named `peer-<label>`; with `--flow none` it is rendered Vision-less (see
    /// docs/RELAY_EXIT_VISION.md). The bundle goes to stdout only when stdout is NOT a terminal (pipe it to
    /// the relay), or to `--out FILE` (created 0600, never overwritten).
    IssueExit {
        #[arg(long)]
        label: String,
        /// vision (default) or none.
        #[arg(long, default_value = "vision")]
        flow: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// RELAY: declare the paired exit as a relayed route (once; a conflicting declaration is refused) and give
    /// one relay user the dedicated credential from the exit bundle. Applies and reloads with rollback.
    AcceptExit {
        /// The relay user (account id) who gets this route.
        #[arg(long)]
        user: String,
        /// Endpoint id to declare, e.g. `evolus-via-ru1`.
        #[arg(long)]
        endpoint_id: String,
        /// Client-visible name (default: `<exit node id> · via <this node id>`).
        #[arg(long)]
        tag: Option<String>,
        /// Relay access-path id (default: the only relay path).
        #[arg(long)]
        path: Option<String>,
        /// Read the bundle from stdin.
        #[arg(long, conflicts_with = "bundle_file")]
        bundle_stdin: bool,
        /// Read the bundle from a file (must not be readable by group/others).
        #[arg(long)]
        bundle_file: Option<PathBuf>,
        /// Replace this user's existing credential for the endpoint.
        #[arg(long)]
        replace_credential: bool,
    },
    /// RELAY: issue a Google/YouTube hairpin principal for one exit and emit the hairpin bundle (same output
    /// rules as `issue-exit`). The principal can reach ONLY the Google/YouTube domain set through this relay.
    IssueHairpin {
        #[arg(long)]
        label: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// EXIT: configure the Google/YouTube hairpin from a relay's bundle (validated, applied, rolled back on
    /// failure; same effect as `google-egress-hairpin set`).
    AcceptHairpin {
        #[arg(long, conflicts_with = "bundle_file")]
        bundle_stdin: bool,
        #[arg(long)]
        bundle_file: Option<PathBuf>,
    },
    /// Read-only: role, paired routes, credential counts, flows, and where Google/YouTube traffic exits.
    /// Prints no credential.
    Status,
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 31
        && label
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Where a credential-bearing bundle goes, fixed BEFORE the dedicated credential is created so that a refusal
/// (a terminal, an existing or unwritable file) never leaves an orphaned credential behind. A file is reserved
/// (created new, mode 0600) up front; if delivery fails later the file is removed again.
enum Sink {
    File {
        file: std::fs::File,
        path: PathBuf,
    },
    /// Stdout is not a terminal. Human progress lines are diverted to stderr for the whole run (the apply path
    /// prints "sing-box reloaded ..." on stdout), so only the bundle itself reaches the pipe.
    Stdout(crate::MachineStdout),
}

fn prepare_sink(out: Option<&Path>) -> Result<Sink> {
    match out {
        Some(path) => {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let file = opts.open(path).with_context(|| {
                format!(
                    "cannot create {} (a bundle file is created new, mode 0600, and never overwritten)",
                    path.display()
                )
            })?;
            Ok(Sink::File {
                file,
                path: path.to_path_buf(),
            })
        }
        None => {
            if std::io::stdout().is_terminal() {
                bail!(
                    "refusing to print a credential-bearing bundle to a terminal. Pipe it to the other host \
                     (`... | ssh other 'vpn-admin pair accept-... --bundle-stdin'`) or use --out FILE"
                );
            }
            Ok(Sink::Stdout(
                crate::MachineStdout::divert_human_output_to_stderr()?,
            ))
        }
    }
}

impl Sink {
    /// Deliver the bundle. A file that could not be written completely is removed.
    fn deliver(self, text: &str) -> Result<()> {
        match self {
            Sink::File { mut file, path } => {
                let written = file
                    .write_all(text.as_bytes())
                    .and_then(|_| file.write_all(b"\n"))
                    .and_then(|_| file.sync_all());
                drop(file);
                if let Err(e) = written {
                    let _ = std::fs::remove_file(&path);
                    return Err(e).with_context(|| format!("writing {}", path.display()));
                }
                eprintln!("bundle written to {} (mode 0600). It contains a credential: move it over a protected channel and delete it afterwards.", path.display());
                Ok(())
            }
            Sink::Stdout(machine) => machine.write_document(&serde_json::from_str(text)?),
        }
    }

    /// The credential could not be issued after all: give a reserved file back.
    fn abandon(self) {
        if let Sink::File { file, path } = self {
            drop(file);
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Remove a dedicated user that was created for a bundle which then could not be delivered, so a failed
/// pipe or write never leaves a credential nobody holds.
fn remove_dedicated_user(cfg: &DeploymentConfig, id: &str) -> Result<()> {
    let current = store::load_users(&cfg.users_file())?;
    let without: Vec<_> = current.iter().filter(|u| u.id != id).cloned().collect();
    crate::apply_users_and_save(cfg, &current, &without)?;
    Ok(())
}

/// Hand the finished bundle to its sink; on any failure take the just-created credential back out.
fn deliver_or_revoke(
    cfg: &DeploymentConfig,
    sink: Sink,
    user_id: &str,
    text: Result<String>,
) -> Result<()> {
    let outcome = text.and_then(|t| sink.deliver(&t));
    match outcome {
        Ok(()) => Ok(()),
        Err(error) => {
            let revoked = remove_dedicated_user(cfg, user_id);
            Err(error).context(match revoked {
                Ok(()) => "the bundle could not be delivered; the credential just issued was removed again (nothing is left behind)".to_string(),
                Err(e) => format!("the bundle could not be delivered AND the credential just issued could not be removed ({e}); run `vpn-admin user list` and remove user {user_id}"),
            })
        }
    }
}

fn read_bundle(stdin: bool, file: Option<&Path>) -> Result<String> {
    const LIMIT: u64 = 64 * 1024;
    match (stdin, file) {
        (true, None) => {
            let mut text = String::new();
            std::io::stdin().take(LIMIT).read_to_string(&mut text)?;
            Ok(text)
        }
        (false, Some(path)) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(path)
                    .with_context(|| format!("reading {}", path.display()))?
                    .permissions()
                    .mode();
                if mode & 0o077 != 0 {
                    bail!(
                        "{} is readable by group/others (mode {:o}); the bundle holds a credential. chmod 600 it first",
                        path.display(),
                        mode & 0o777
                    );
                }
            }
            let mut text = String::new();
            std::fs::File::open(path)?
                .take(LIMIT)
                .read_to_string(&mut text)?;
            Ok(text)
        }
        _ => bail!("give exactly one of --bundle-stdin or --bundle-file"),
    }
}

fn require_role(cfg: &DeploymentConfig, role: NodeRole, what: &str) -> Result<()> {
    if cfg.role != role {
        bail!(
            "{what} runs on a {} node; this node is a {}",
            role_name(role),
            role_name(cfg.role)
        );
    }
    Ok(())
}

fn role_name(role: NodeRole) -> &'static str {
    match role {
        NodeRole::Exit => "exit",
        NodeRole::Relay => "relay",
    }
}

fn paired_node(cfg: &DeploymentConfig) -> Result<PairedNode> {
    let reality = crate::load_reality_params(cfg)?;
    Ok(PairedNode {
        node_id: cfg.node_id.clone(),
        host: cfg.public_host.to_ascii_lowercase(),
        port: cfg.reality.listen_port,
        server_name: cfg.reality.handshake_server.clone(),
        reality_public_key: reality.public_key_hex.clone(),
        reality_short_id: reality
            .short_ids
            .first()
            .cloned()
            .context("this node has no REALITY short id; run `vpn-admin init`")?,
    })
}

fn create_dedicated_user(
    cfg: &DeploymentConfig,
    name: &str,
    configure: impl FnOnce(&mut compat_config::model::CompatUser),
) -> Result<compat_config::model::CompatUser> {
    crate::validate_user_display_name(name)?;
    let mut users = store::load_users(&cfg.users_file())?;
    if users.iter().any(|u| u.name == name) {
        bail!("a user named {name:?} already exists; pick another --label (a pairing credential is never reused)");
    }
    let previous = users.clone();
    let mut id = compat_config::credentials::generate_user_id();
    while users.iter().any(|u| u.id == id) {
        id = compat_config::credentials::generate_user_id();
    }
    let now = common::UnixSeconds::now().0 as i64;
    let (mut user, _token) = crate::build_new_operator_user(id, name, None, now);
    configure(&mut user);
    users.push(user.clone());
    crate::apply_users_and_save(cfg, &previous, &users)?;
    Ok(user)
}

pub fn issue_exit(
    cfg: &DeploymentConfig,
    label: &str,
    flow: &str,
    out: Option<&Path>,
) -> Result<()> {
    if cfg.role == NodeRole::Relay {
        if !cfg.exit_principals_enabled() {
            bail!(
                "this relay is not an exit for other relays. For a bidirectional deployment add \
                 `[exit_principals]` / `enabled = true` to deployment.toml and run `vpn-admin render-config` \
                 (docs/BIDIRECTIONAL.md)"
            );
        }
    } else {
        require_role(cfg, NodeRole::Exit, "pair issue-exit")?;
    }
    let as_principal = cfg.role == NodeRole::Relay;
    if !valid_label(label) {
        bail!(
            "--label must be 1-31 characters of a-z, 0-9 and '-', starting with a letter or digit"
        );
    }
    let vless_flow = match flow {
        "vision" => EndpointVlessFlow::Vision,
        "none" => EndpointVlessFlow::None,
        other => bail!("--flow must be `vision` or `none`, not {other:?}"),
    };
    let node = paired_node(cfg)?;
    let sink = prepare_sink(out)?;
    let name = format!("peer-{label}");
    let user = match create_dedicated_user(cfg, &name, |u| {
        u.vision_off_experiment = vless_flow == EndpointVlessFlow::None;
        u.exit_principal = as_principal;
    }) {
        Ok(user) => user,
        Err(error) => {
            sink.abandon();
            return Err(error);
        }
    };
    let bundle = ExitPairingBundle {
        schema: EXIT_BUNDLE_SCHEMA.into(),
        exit: node,
        vless_flow,
        credential: BundleCredential {
            uuid: user.vless_uuid.clone(),
        },
    };
    eprintln!(
        "issued dedicated exit credential: user {name} (id {}), flow {}",
        user.id,
        if vless_flow == EndpointVlessFlow::None {
            "none (Vision-less)"
        } else {
            "vision"
        }
    );
    deliver_or_revoke(
        cfg,
        sink,
        &user.id,
        serde_json::to_string(&bundle).map_err(Into::into),
    )
}

/// Options of `pair accept-exit`, grouped so the function stays readable.
pub struct AcceptExitArgs<'a> {
    pub user_id: &'a str,
    pub endpoint_id: &'a str,
    pub tag: Option<&'a str>,
    pub path_id: Option<&'a str>,
    pub stdin: bool,
    pub file: Option<&'a Path>,
    pub replace_credential: bool,
}

pub fn accept_exit(
    cfg: &DeploymentConfig,
    config_path: &Path,
    args: AcceptExitArgs<'_>,
) -> Result<()> {
    let AcceptExitArgs {
        user_id,
        endpoint_id,
        tag,
        path_id,
        stdin,
        file,
        replace_credential,
    } = args;
    require_role(cfg, NodeRole::Relay, "pair accept-exit")?;
    let bundle = parse_exit_bundle(&read_bundle(stdin, file)?)?;
    let relay_paths: Vec<&str> = cfg
        .access_paths
        .iter()
        .filter(|p| p.kind == "relay")
        .map(|p| p.id.as_str())
        .collect();
    let path_id = match path_id {
        Some(p) if relay_paths.contains(&p) => p.to_string(),
        Some(p) => {
            bail!("--path {p:?} is not a declared relay access path (declared: {relay_paths:?})")
        }
        None => match relay_paths.as_slice() {
            [only] => only.to_string(),
            [] => bail!(
                "this relay declares no relay access path; was it installed with --role relay?"
            ),
            many => bail!("several relay access paths exist ({many:?}); pass --path"),
        },
    };
    let default_tag = format!("{} · via {}", bundle.exit.node_id, cfg.node_id);
    let wanted =
        peer_endpoint_from_exit_bundle(endpoint_id, tag.unwrap_or(&default_tag), &path_id, &bundle);

    let users = store::load_users(&cfg.users_file())?;
    let user = users
        .iter()
        .find(|u| u.id == user_id)
        .with_context(|| format!("no such user: {user_id}"))?;
    if user.peer_credentials.contains_key(endpoint_id) && !replace_credential {
        bail!(
            "user {user_id} already has a credential for {endpoint_id}; pass --replace-credential to replace it \
             (after rotating it on the exit)"
        );
    }

    // Step 1: declare the route and apply. Between the steps the relay forwards only to the declared exit and
    // no user holds a credential for it, so an interruption leaves the relay fail-closed (unpaired users see
    // `no_selectable_route`), never unrestricted.
    let mode_backup = match apply_peer_endpoint_toml(config_path, &wanted)? {
        PeerEndpointOutcome::AlreadyDeclared => {
            eprintln!("route {endpoint_id} already declared with identical parameters");
            None
        }
        PeerEndpointOutcome::Declared(backup) => {
            eprintln!("route {endpoint_id} declared in deployment.toml");
            Some(backup)
        }
    };
    let candidate = DeploymentConfig::load(config_path)?;
    if let Some(backup) = &mode_backup {
        if let Err(e) = crate::regenerate_singbox_config(&candidate, true) {
            let bytes = std::fs::read(backup)?;
            compat_config::migrate::atomic_write(config_path, &bytes, 0o644).context(
                "apply failed and deployment.toml rollback failed; inspect the retained backup",
            )?;
            let _ = crate::regenerate_singbox_config(cfg, true);
            return Err(e).context(
                "applying the relay config for the new route FAILED; deployment.toml restored",
            );
        }
    }

    // Step 2: the dedicated credential for this user.
    let mut users = store::load_users(&candidate.users_file())?;
    let user = crate::find_user_mut(&mut users, user_id)?;
    user.peer_credentials.insert(
        endpoint_id.to_string(),
        PeerCredential::VlessReality {
            uuid: bundle.credential.uuid.clone(),
        },
    );
    store::save_users_atomic(&candidate.users_file(), &users)?;
    crate::refresh_stale_subscription_service(&candidate);
    eprintln!(
        "user {user_id} now has a dedicated credential for {endpoint_id} (flow {}). No credential was printed.",
        if bundle.vless_flow == EndpointVlessFlow::None { "none" } else { "vision" }
    );
    if bundle.vless_flow == EndpointVlessFlow::None {
        eprintln!("This route is Vision-less: hand out the `?format=singbox&compat=hiddify-pinned` link (docs/RELAY_EXIT_VISION.md).");
    }
    Ok(())
}

pub fn issue_hairpin(cfg: &DeploymentConfig, label: &str, out: Option<&Path>) -> Result<()> {
    require_role(cfg, NodeRole::Relay, "pair issue-hairpin")?;
    if !valid_label(label) {
        bail!(
            "--label must be 1-31 characters of a-z, 0-9 and '-', starting with a letter or digit"
        );
    }
    let node = paired_node(cfg)?;
    let sink = prepare_sink(out)?;
    let name = format!("hairpin-{label}");
    let user = match create_dedicated_user(cfg, &name, |u| u.google_egress_hairpin = true) {
        Ok(user) => user,
        Err(error) => {
            sink.abandon();
            return Err(error);
        }
    };
    let bundle = HairpinPairingBundle {
        schema: HAIRPIN_BUNDLE_SCHEMA.into(),
        relay: node,
        credential: BundleCredential {
            uuid: user.vless_uuid.clone(),
        },
    };
    eprintln!("issued hairpin principal: user {name} (id {}); it can reach only the Google/YouTube domain set", user.id);
    deliver_or_revoke(
        cfg,
        sink,
        &user.id,
        serde_json::to_string(&bundle).map_err(Into::into),
    )
}

pub fn accept_hairpin(
    cfg: &DeploymentConfig,
    config_path: &Path,
    stdin: bool,
    file: Option<&Path>,
) -> Result<()> {
    require_role(cfg, NodeRole::Exit, "pair accept-hairpin")?;
    let bundle = parse_hairpin_bundle(&read_bundle(stdin, file)?)?;
    crate::cmd_google_egress_hairpin_set(
        cfg,
        config_path,
        GoogleEgressHairpinSection {
            relay_host: bundle.relay.host,
            relay_port: bundle.relay.port,
            relay_server_name: bundle.relay.server_name,
            relay_reality_public_key: bundle.relay.reality_public_key,
            relay_reality_short_id: bundle.relay.reality_short_id,
        },
        &bundle.credential.uuid,
    )
}

/// Read-only summary; names and counts only.
pub fn status(cfg: &DeploymentConfig) -> Result<()> {
    let users = store::load_users(&cfg.users_file()).unwrap_or_default();
    println!("node: {} ({})", cfg.node_id, role_name(cfg.role));
    match cfg.role {
        NodeRole::Relay => {
            println!(
                "paired exits (relayed routes): {}",
                cfg.peer_endpoints
                    .iter()
                    .filter(|p| p.path != "direct")
                    .count()
            );
            for p in &cfg.peer_endpoints {
                let holders = users
                    .iter()
                    .filter(|u| u.peer_credentials.contains_key(&p.id))
                    .count();
                println!(
                    "  - {} -> {}:{} path={} flow={} users_with_credential={}",
                    p.id,
                    p.host,
                    p.port,
                    p.path,
                    if p.vless_flow == EndpointVlessFlow::None {
                        "none"
                    } else {
                        "vision"
                    },
                    holders
                );
            }
            let hairpins = users.iter().filter(|u| u.google_egress_hairpin).count();
            println!("hairpin principals: {hairpins}");
            if cfg.exit_principals_enabled() {
                let principals = users.iter().filter(|u| u.exit_principal).count();
                println!(
                    "exit principals (this relay is also an EXIT for peer relays): {principals}"
                );
            }
            println!(
                "Google/YouTube egress: leaves THIS relay for its {hairpins} hairpin principal(s) (TLS spoof {}); all other traffic follows each user's declared exit",
                match &cfg.google_egress_tls_spoof { Some(s) => format!("on, sni {}", s.sni), None => "off".into() }
            );
        }
        NodeRole::Exit => {
            let dedicated = users.iter().filter(|u| u.name.starts_with("peer-")).count();
            let visionless = users
                .iter()
                .filter(|u| u.name.starts_with("peer-") && u.vision_off_experiment)
                .count();
            println!(
                "dedicated relay credentials (peer-*): {dedicated} ({visionless} Vision-less)"
            );
            match &cfg.google_egress_hairpin {
                Some(h) => println!(
                    "Google/YouTube egress: hairpinned through {}:{} (this exit's own address is not used for that set); all other traffic leaves this exit",
                    h.relay_host, h.relay_port
                ),
                None => println!("Google/YouTube egress: leaves this exit directly (no hairpin configured)"),
            }
        }
    }
    Ok(())
}
