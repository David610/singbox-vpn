//! Platform v2 operator surface: transports, endpoints, nodes and
//! AmneziaWG user credentials.
//!
//! Every mutating command here goes through the same load → mutate →
//! apply → persist transaction as the existing user commands
//! (`apply_users_and_save`), so a revocation reaches every data plane
//! before `users.json` publishes the new state. No command prints a
//! private key, preshared key or header protection key, except
//! `user awg show`, which exists to hand a user their own profile over
//! SSH and honours `SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS`.

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use compat_config::amneziawg::{
    render_client_awg_quick, render_server_setconf, AmneziaWgProvider, AwgCredential, AwgProfile,
};
use compat_config::amneziawg_state::{self, StateOutcome};
use compat_config::contract_v2::{provisioning_document_v2, route_catalog, RouteContext};
use compat_config::deployment::{AmneziaWgSection, DeploymentConfig};
use compat_config::model::CompatUser;
use compat_config::store;
use platform_core::transport::{TransportDescriptor, TransportKind, TransportProvider};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Subcommand)]
pub enum TransportCommands {
    /// Show which transports this node serves, their listeners, engines and
    /// live state. Never prints secrets.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Enable a transport on this node. Only `amneziawg` can be enabled;
    /// REALITY and Hysteria2 are part of every installation.
    Enable {
        transport: String,
        #[arg(long, default_value_t = 51820)]
        listen_port: u16,
        /// `awg-3` (default) or `awg-2-compatible`.
        #[arg(long, default_value = "awg-3")]
        profile: String,
        #[arg(long)]
        subnet_v4: Option<String>,
        #[arg(long)]
        subnet_v6: Option<String>,
        #[arg(long)]
        mtu: Option<u16>,
        /// Do not issue credentials to existing users now.
        #[arg(long)]
        no_issue: bool,
    },
    /// Disable a transport on this node and return deployment.toml to the
    /// schema older binaries understand.
    Disable {
        transport: String,
        /// Also delete every user's credential for it (returns users.json to
        /// schema 1, required before rolling back to a pre-AmneziaWG build).
        #[arg(long)]
        purge_credentials: bool,
    },
    /// Replace the node keys and obfuscation parameters. CLIENT-BREAKING:
    /// every user must re-import their profile afterwards.
    Rotate {
        transport: String,
        #[arg(long)]
        yes: bool,
    },
    /// Print the pinned install plan the installer consumes (JSON).
    Plan { transport: String },
}

#[derive(Subcommand)]
pub enum EndpointCommands {
    /// List this node's listeners and declared peer endpoints. With
    /// `--user`, list the routes that user's v2 document contains. Never
    /// prints credentials.
    List {
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum NodeCommands {
    /// List nodes known to this deployment: this node plus one node per
    /// declared peer failure domain.
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum AwgUserCommands {
    /// Issue an AmneziaWG credential (tunnel keys, preshared key, address).
    Issue {
        user_id: Option<String>,
        /// Issue to every user that has none.
        #[arg(long, conflicts_with = "user_id")]
        all: bool,
    },
    /// Replace a user's AmneziaWG keys; the address is kept.
    Rotate { user_id: String },
    /// Remove a user's AmneziaWG credential; applied to the data plane
    /// before it is persisted.
    Revoke { user_id: String },
    /// Print the user's `awg-quick` profile (a credential) for out-of-band
    /// delivery.
    Show { user_id: String },
}

pub fn is_read_only_transport(cmd: &TransportCommands) -> bool {
    matches!(cmd, TransportCommands::Status { .. } | TransportCommands::Plan { .. })
}

pub fn is_read_only_awg(cmd: &AwgUserCommands) -> bool {
    matches!(cmd, AwgUserCommands::Show { .. })
}

fn require_amneziawg(transport: &str) -> Result<()> {
    match TransportKind::from_wire(transport) {
        TransportKind::AmneziaWg => Ok(()),
        TransportKind::VlessReality | TransportKind::Hysteria2 => bail!(
            "{transport} is part of every installation and is managed by install.sh/update.sh; \
             only amneziawg can be enabled, disabled or rotated here"
        ),
        TransportKind::Other(_) => bail!("unknown transport {transport:?} (supported: amneziawg)"),
    }
}

fn parse_profile(profile: &str) -> Result<AwgProfile> {
    match profile {
        "awg-3" => Ok(AwgProfile::Awg3),
        "awg-2-compatible" => Ok(AwgProfile::Awg2Compatible),
        other => bail!("unknown AmneziaWG profile {other:?} (expected awg-3 or awg-2-compatible)"),
    }
}

/// `SINGBOX_VPN_AWG` overrides the binary (tests install a guard there so
/// they can never reconfigure a live interface on the host).
pub fn awg_binary() -> PathBuf {
    std::env::var_os("SINGBOX_VPN_AWG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/local/bin/awg"))
}

fn awg_output(args: &[&str]) -> Option<String> {
    let out = Command::new(awg_binary()).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Render the AmneziaWG server configuration for `users` and apply it to
/// the running interface.
///
/// Returns `Ok(true)` when the running data plane provably matches (or the
/// transport is disabled, or the interface is not running, in which case
/// no AmneziaWG peer can connect at all), `Ok(false)` when the file was
/// written but could not be applied and `require_live_apply` is false.
pub fn render_and_apply_amneziawg(
    cfg: &DeploymentConfig,
    users: &[CompatUser],
    require_live_apply: bool,
) -> Result<bool> {
    let Some(node) = amneziawg_state::load_node_config(cfg, true)? else {
        return Ok(true);
    };
    let now = common::UnixSeconds::now().0 as i64;
    let active: Vec<(&str, &AwgCredential)> = users
        .iter()
        .filter(|u| u.is_active(now))
        .filter_map(|u| u.amneziawg.as_ref().map(|c| (u.id.as_str(), c)))
        .collect();
    let text = render_server_setconf(&node, &active)
        .context("rendering the AmneziaWG server configuration")?;
    let target = cfg.amneziawg_setconf_file();
    let current = std::fs::read_to_string(&target).ok();
    if current.as_deref() != Some(text.as_str()) {
        compat_config::migrate::atomic_write(&target, text.as_bytes(), 0o600)
            .context("writing the AmneziaWG server configuration")?;
    }
    write_runtime_env(cfg, &node)?;

    let awg = awg_binary();
    if !awg.exists() {
        if require_live_apply && !super::offline_mutation_allowed() {
            bail!(
                "refusing to commit an authorization mutation: `awg` not found at {awg:?}, so the \
                 AmneziaWG data plane cannot be updated. Install it with deploy/lib/amneziawg.sh, \
                 or set SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 for an intentional offline change."
            );
        }
        println!("warning: {awg:?} not found; AmneziaWG config written to {target:?} but not applied.");
        return Ok(false);
    }
    if awg_output(&["show", &node.interface]).is_none() {
        println!(
            "AmneziaWG interface {} is not running; configuration written to {target:?} and \
             applied when vpn-amneziawg starts (no AmneziaWG peer can connect meanwhile).",
            node.interface
        );
        return Ok(true);
    }
    let out = Command::new(&awg)
        .args(["syncconf", &node.interface])
        .arg(&target)
        .output()
        .context("running awg syncconf")?;
    if !out.status.success() {
        bail!(
            "awg syncconf {} failed: {}",
            node.interface,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // Prove the live peer set now equals the rendered one.
    let live: std::collections::BTreeSet<String> = awg_output(&["show", &node.interface, "peers"])
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let expected: std::collections::BTreeSet<String> =
        active.iter().map(|(_, c)| c.public_key.clone()).collect();
    if live != expected {
        bail!(
            "AmneziaWG interface {} reports {} peers after syncconf, expected {}; the running \
             authorization does not match users.json",
            node.interface,
            live.len(),
            expected.len()
        );
    }
    println!(
        "AmneziaWG configuration applied to {} ({} active peer(s)).",
        node.interface,
        expected.len()
    );
    Ok(true)
}

/// Non-secret interface facts `deploy/lib/amneziawg.sh post-start` needs
/// (addresses, MTU, subnet for NAT). Plain `KEY=value`, no quoting needed:
/// every value is validated address/number syntax.
fn write_runtime_env(
    cfg: &DeploymentConfig,
    node: &compat_config::amneziawg::AwgNodeConfig,
) -> Result<()> {
    let v6 = node
        .subnet_v6
        .map(|net| (format!("{}/{}", node.server_address_v6().expect("subnet"), net.prefix), net.to_string()))
        .unwrap_or_default();
    let text = format!(
        "AWG_INTERFACE={}\nAWG_LISTEN_PORT={}\nAWG_ADDRESS_V4={}/{}\nAWG_SUBNET_V4={}\nAWG_ADDRESS_V6={}\nAWG_SUBNET_V6={}\nAWG_MTU={}\nAWG_SETCONF={}\n",
        node.interface,
        node.listen_port,
        node.server_address_v4(),
        node.subnet_v4.prefix,
        node.subnet_v4,
        v6.0,
        v6.1,
        node.mtu,
        cfg.amneziawg_setconf_file().display(),
    );
    let path = cfg.amneziawg_dir().join("runtime.env");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        compat_config::migrate::atomic_write(&path, text.as_bytes(), 0o644)
            .context("writing AmneziaWG runtime.env")?;
    }
    Ok(())
}

fn existing_credentials(users: &[CompatUser]) -> Vec<(&str, &AwgCredential)> {
    users
        .iter()
        .filter_map(|u| u.amneziawg.as_ref().map(|c| (u.id.as_str(), c)))
        .collect()
}

fn load_public_node(cfg: &DeploymentConfig) -> Result<compat_config::amneziawg::AwgNodeConfig> {
    amneziawg_state::load_node_config(cfg, false)?
        .context("AmneziaWG is not enabled on this node (`vpn-admin transport enable amneziawg`)")
}

pub fn cmd_transport_enable(
    cfg: &DeploymentConfig,
    config_path: &Path,
    transport: &str,
    listen_port: u16,
    profile: &str,
    subnet_v4: Option<String>,
    subnet_v6: Option<String>,
    mtu: Option<u16>,
    no_issue: bool,
) -> Result<()> {
    require_amneziawg(transport)?;
    let profile = parse_profile(profile)?;
    if cfg.amneziawg.is_some() {
        bail!("amneziawg is already enabled on this node");
    }
    let mut section: AmneziaWgSection = toml::from_str(&format!("listen_port = {listen_port}"))
        .context("building the [amneziawg] section")?;
    if let Some(v4) = subnet_v4 {
        section.subnet_v4 = v4;
    }
    section.subnet_v6 = subnet_v6;
    if let Some(mtu) = mtu {
        section.mtu = mtu;
    }

    // Validate the would-be file before generating anything.
    let original = std::fs::read_to_string(config_path)?;
    let candidate: DeploymentConfig =
        toml::from_str(&amneziawg_state::enable_in_toml(&original, &section)?)?;
    let mut candidate = candidate;
    candidate.state_dir = cfg.state_dir.clone();
    candidate.validate()?;

    match amneziawg_state::ensure_node_state(cfg, profile)? {
        StateOutcome::Created => println!("Generated AmneziaWG server keys and parameters in {:?}.", cfg.amneziawg_dir()),
        _ => println!("Reusing existing AmneziaWG server keys and parameters in {:?}.", cfg.amneziawg_dir()),
    }
    let enabled = amneziawg_state::patch_deployment_file(config_path, |text| {
        amneziawg_state::enable_in_toml(text, &section)
    })?;
    println!(
        "deployment.toml: [amneziawg] enabled on UDP {} (schema_version {}); a backup of the \
         previous file was kept.",
        section.listen_port, enabled.schema_version
    );

    if !no_issue {
        issue_all(&enabled)?;
    } else {
        render_and_apply_amneziawg(&enabled, &store::load_users(&enabled.users_file())?, false)?;
    }
    println!(
        "Next: open UDP {} in the provider firewall, then run \
         `/opt/singbox-vpn/deploy/lib/amneziawg.sh install && systemctl enable --now vpn-amneziawg`. \
         Restart vpn-subscription so /v2/provision and ?format=amneziawg serve the new transport.",
        section.listen_port
    );
    Ok(())
}

fn issue_all(cfg: &DeploymentConfig) -> Result<()> {
    let node = load_public_node(cfg)?;
    let mut users = store::load_users(&cfg.users_file())?;
    let previous = users.clone();
    let mut issued = 0;
    for i in 0..users.len() {
        if users[i].amneziawg.is_some() {
            continue;
        }
        let credential = {
            let existing = existing_credentials(&users);
            AmneziaWgProvider
                .issue_credentials(&node, &users[i].id, &existing)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        };
        users[i].amneziawg = Some(credential);
        issued += 1;
    }
    super::apply_users_and_save(cfg, &previous, &users)?;
    println!("Issued AmneziaWG credentials to {issued} user(s).");
    Ok(())
}

pub fn cmd_transport_disable(
    cfg: &DeploymentConfig,
    config_path: &Path,
    transport: &str,
    purge: bool,
) -> Result<()> {
    require_amneziawg(transport)?;
    if cfg.amneziawg.is_none() {
        bail!("amneziawg is not enabled on this node");
    }
    if purge {
        let mut users = store::load_users(&cfg.users_file())?;
        let previous = users.clone();
        let count = users.iter_mut().filter_map(|u| u.amneziawg.take()).count();
        // Apply the empty peer set first so revocation is live before the
        // credentials disappear from users.json.
        render_and_apply_amneziawg(cfg, &users, true)?;
        super::apply_users_and_save(cfg, &previous, &users)?;
        println!("Purged {count} AmneziaWG credential(s); users.json no longer requires schema 2.");
    }
    let disabled =
        amneziawg_state::patch_deployment_file(config_path, amneziawg_state::disable_in_toml)?;
    println!(
        "deployment.toml: [amneziawg] removed (schema_version {}). Stop the data plane with \
         `systemctl disable --now vpn-amneziawg`; key material in {:?} is kept for a later \
         re-enable.",
        disabled.schema_version,
        cfg.amneziawg_dir()
    );
    if !purge {
        println!(
            "note: users still carry AmneziaWG credentials (users.json schema 2). Run with \
             --purge-credentials before rolling back to a build without AmneziaWG."
        );
    }
    Ok(())
}

pub fn cmd_transport_rotate(cfg: &DeploymentConfig, transport: &str, yes: bool) -> Result<()> {
    require_amneziawg(transport)?;
    if cfg.amneziawg.is_none() {
        bail!("amneziawg is not enabled on this node");
    }
    if !yes {
        bail!(
            "rotating AmneziaWG replaces the server keys and obfuscation parameters; EVERY user's \
             AmneziaWG profile stops working until they re-import. Re-run with --yes."
        );
    }
    let current = load_public_node(cfg)?;
    amneziawg_state::rotate_node_state(cfg, current.params.profile)?;
    let users = store::load_users(&cfg.users_file())?;
    render_and_apply_amneziawg(cfg, &users, true)?;
    println!(
        "AmneziaWG server keys and parameters rotated. Restart vpn-subscription, then have every \
         user refresh their profile."
    );
    Ok(())
}

pub fn cmd_transport_plan(cfg: &DeploymentConfig, transport: &str) -> Result<()> {
    require_amneziawg(transport)?;
    let node = load_public_node(cfg)?;
    let plan = AmneziaWgProvider.install_plan(&node);
    plan.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

pub fn cmd_transport_status(cfg: &DeploymentConfig, json: bool) -> Result<()> {
    let users = store::load_users(&cfg.users_file()).unwrap_or_default();
    let singbox_active = super::CompatibilityServiceManager::default().is_active();
    let mut rows = vec![
        serde_json::json!({
            "transport": "vless-reality",
            "enabled": true,
            "engine": "sing-box",
            "listen": format!("tcp/{}", cfg.reality.listen_port),
            "service_active": singbox_active,
            "users_with_credentials": users.len(),
        }),
        serde_json::json!({
            "transport": "hysteria2",
            "enabled": cfg.role == compat_config::deployment::NodeRole::Exit,
            "engine": "sing-box",
            "listen": format!("udp/{}", cfg.hysteria2.listen_port),
            "service_active": singbox_active,
            "users_with_credentials": users.len(),
        }),
    ];
    match &cfg.amneziawg {
        None => rows.push(serde_json::json!({"transport": "amneziawg", "enabled": false})),
        Some(section) => {
            let with_credentials = users.iter().filter(|u| u.amneziawg.is_some()).count();
            let now = common::UnixSeconds::now().0 as i64;
            let handshakes = awg_output(&["show", &section.interface, "latest-handshakes"]);
            let (running, peers, recent) = match &handshakes {
                None => (false, 0, 0),
                Some(text) => {
                    let times: Vec<i64> = text
                        .lines()
                        .filter_map(|l| l.split_whitespace().nth(1)?.parse().ok())
                        .collect();
                    let recent = times.iter().filter(|t| **t > 0 && now - **t <= 180).count();
                    (true, times.len(), recent)
                }
            };
            let caps = AmneziaWgProvider.capabilities();
            rows.push(serde_json::json!({
                "transport": "amneziawg",
                "enabled": true,
                "engine": "amneziawg",
                "listen": format!("udp/{}", section.listen_port),
                "interface": section.interface,
                "interface_running": running,
                "users_with_credentials": with_credentials,
                "live_peers": peers,
                "peers_with_handshake_last_180s": recent,
                "detour_capable": caps.detour_capable,
                "pinned_amneziawg_go": compat_config::amneziawg::AMNEZIAWG_GO_VERSION,
            }));
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    for r in &rows {
        let enabled = r["enabled"].as_bool().unwrap_or(false);
        if !enabled {
            println!("{:<14} disabled", r["transport"].as_str().unwrap_or(""));
            continue;
        }
        let live = match r["transport"].as_str() {
            Some("amneziawg") => format!(
                "interface {} {}; {} peer(s), {} with a handshake in the last 180 s",
                r["interface"].as_str().unwrap_or(""),
                if r["interface_running"].as_bool() == Some(true) { "up" } else { "NOT RUNNING" },
                r["live_peers"],
                r["peers_with_handshake_last_180s"]
            ),
            _ => format!(
                "sing-box {}",
                if r["service_active"].as_bool() == Some(true) { "active" } else { "NOT ACTIVE" }
            ),
        };
        println!(
            "{:<14} enabled  {:<9} engine {:<9} {} user credential(s); {live}",
            r["transport"].as_str().unwrap_or(""),
            r["listen"].as_str().unwrap_or(""),
            r["engine"].as_str().unwrap_or(""),
            r["users_with_credentials"]
        );
    }
    Ok(())
}

fn route_context(cfg: &DeploymentConfig) -> Result<RouteContext> {
    let reality = super::load_reality_params(cfg)?;
    let obfs = std::fs::read_to_string(cfg.hysteria_obfs_password_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let endpoints = super::served_endpoints(cfg, &reality, obfs.as_deref())?;
    let awg = amneziawg_state::load_node_config(cfg, false)?;
    Ok(RouteContext::from_deployment(cfg, endpoints, awg))
}

pub fn cmd_endpoint_list(cfg: &DeploymentConfig, user: Option<&str>, json: bool) -> Result<()> {
    let ctx = route_context(cfg)?;
    if let Some(user_id) = user {
        let users = store::load_users(&cfg.users_file())?;
        let u = users
            .iter()
            .find(|u| u.id == user_id)
            .with_context(|| format!("user {user_id:?} not found"))?;
        let doc = match provisioning_document_v2(&ctx, u) {
            Ok(doc) => doc,
            Err(compat_config::CompatError::NoSelectableRoute) => {
                println!("user {user_id} has no selectable route");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let catalog = route_catalog(&doc);
        if json {
            println!("{}", serde_json::to_string_pretty(&catalog)?);
            return Ok(());
        }
        for r in &catalog.routes {
            let transports: Vec<String> = r
                .hops
                .iter()
                .filter_map(|h| catalog.endpoint(h))
                .map(|e| format!("{}@{}", e.transport, e.node_id))
                .collect();
            println!(
                "{:<28} {:<8} prio {:>3}  {}  [{}]",
                r.route_id,
                format!("{:?}", r.kind).to_lowercase(),
                r.priority,
                transports.join(" -> "),
                r.label
            );
        }
        return Ok(());
    }

    let mut rows: Vec<serde_json::Value> = ctx
        .endpoints
        .iter()
        .map(|e| {
            serde_json::json!({
                "endpoint_id": e.id,
                "transport": e.transport.as_str(),
                "origin": format!("{:?}", e.origin).to_lowercase(),
                "host": e.host,
                "port": e.port,
                "failure_domain": e.failure_domain,
                "path": e.path.clone().unwrap_or_else(|| "direct".into()),
            })
        })
        .collect();
    if let Some(awg) = &ctx.amneziawg {
        rows.push(serde_json::json!({
            "endpoint_id": compat_config::deployment::LOCAL_AMNEZIAWG_ENDPOINT_ID,
            "transport": "amneziawg",
            "origin": "local",
            "host": awg.public_host,
            "port": awg.listen_port,
            "failure_domain": null,
            "path": "direct",
        }));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for r in rows {
            println!(
                "{:<20} {:<14} {:<6} {}:{}  path={}  failure_domain={}",
                r["endpoint_id"].as_str().unwrap_or(""),
                r["transport"].as_str().unwrap_or(""),
                r["origin"].as_str().unwrap_or(""),
                r["host"].as_str().unwrap_or(""),
                r["port"],
                r["path"].as_str().unwrap_or(""),
                r["failure_domain"].as_str().unwrap_or("(this node)")
            );
        }
    }
    Ok(())
}

pub fn cmd_node_list(cfg: &DeploymentConfig, json: bool) -> Result<()> {
    let mut nodes: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    let local_id = if cfg.node_id.is_empty() {
        compat_config::deployment::default_node_id_for_host(&cfg.public_host)
    } else {
        cfg.node_id.clone()
    };
    let mut local_transports = vec!["vless-reality"];
    if cfg.role == compat_config::deployment::NodeRole::Exit {
        local_transports.push("hysteria2");
    }
    if cfg.amneziawg.is_some() {
        local_transports.push("amneziawg");
    }
    nodes.insert(
        local_id.clone(),
        serde_json::json!({
            "node_id": local_id,
            "managed_here": true,
            "role": cfg.role.as_str(),
            "failure_domain": format!("host:{}", cfg.public_host),
            "transports": local_transports,
        }),
    );
    for peer in &cfg.peer_endpoints {
        let id = format!("peer-{}", peer.failure_domain);
        let entry = nodes.entry(id.clone()).or_insert_with(|| {
            serde_json::json!({
                "node_id": id,
                "managed_here": false,
                "role": "exit",
                "failure_domain": peer.failure_domain,
                "provider": peer.provider,
                "region": peer.region,
                "transports": [],
                "endpoints": [],
            })
        });
        entry["endpoints"].as_array_mut().map(|a| a.push(peer.id.clone().into()));
        let t = match peer.transport {
            compat_config::CompatTransport::VlessReality => "vless-reality",
            compat_config::CompatTransport::Hysteria2 => "hysteria2",
        };
        if let Some(arr) = entry["transports"].as_array_mut() {
            if !arr.iter().any(|v| v == t) {
                arr.push(t.into());
            }
        }
    }
    let list: Vec<_> = nodes.into_values().collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else {
        for n in list {
            println!(
                "{:<24} {:<5} {:<9} failure_domain={}  transports={}",
                n["node_id"].as_str().unwrap_or(""),
                n["role"].as_str().unwrap_or(""),
                if n["managed_here"].as_bool() == Some(true) { "this node" } else { "peer" },
                n["failure_domain"].as_str().unwrap_or(""),
                n["transports"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(","))
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

pub fn cmd_user_awg(cfg: &DeploymentConfig, cmd: AwgUserCommands) -> Result<()> {
    match cmd {
        AwgUserCommands::Issue { user_id: None, all: true } => issue_all(cfg),
        AwgUserCommands::Issue { user_id: None, all: false } => bail!("give a user id or --all"),
        AwgUserCommands::Issue { user_id: Some(id), .. } => mutate_one(cfg, &id, |node, users, i| {
            if users[i].amneziawg.is_some() {
                bail!("user {id} already has an AmneziaWG credential; use `user awg rotate`");
            }
            let existing = existing_credentials(users);
            let c = AmneziaWgProvider
                .issue_credentials(node, &users[i].id, &existing)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            users[i].amneziawg = Some(c);
            println!("Issued an AmneziaWG credential to {id}.");
            Ok(())
        }),
        AwgUserCommands::Rotate { user_id } => mutate_one(cfg, &user_id, |node, users, i| {
            let current = users[i]
                .amneziawg
                .clone()
                .with_context(|| format!("user {} has no AmneziaWG credential", users[i].id))?;
            let existing = existing_credentials(users);
            let c = AmneziaWgProvider
                .rotate_credentials(node, &users[i].id, &current, &existing)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            users[i].amneziawg = Some(c);
            println!("Rotated the AmneziaWG credential of {}; the old keys stop working now.", users[i].id);
            Ok(())
        }),
        AwgUserCommands::Revoke { user_id } => mutate_one(cfg, &user_id, |_, users, i| {
            if users[i].amneziawg.take().is_none() {
                bail!("user {} has no AmneziaWG credential", users[i].id);
            }
            println!("Revoked the AmneziaWG credential of {}.", users[i].id);
            Ok(())
        }),
        AwgUserCommands::Show { user_id } => {
            let node = load_public_node(cfg)?;
            let users = store::load_users(&cfg.users_file())?;
            let u = users
                .iter()
                .find(|u| u.id == user_id)
                .with_context(|| format!("user {user_id:?} not found"))?;
            let c = u
                .amneziawg
                .as_ref()
                .with_context(|| format!("user {user_id} has no AmneziaWG credential"))?;
            if super::suppress_onboarding_secrets() {
                println!("AmneziaWG profile generated: yes (suppressed)");
                return Ok(());
            }
            print!("{}", render_client_awg_quick(&node, c).map_err(|e| anyhow::anyhow!("{e}"))?);
            Ok(())
        }
    }
}

fn mutate_one(
    cfg: &DeploymentConfig,
    id: &str,
    f: impl FnOnce(&compat_config::amneziawg::AwgNodeConfig, &mut Vec<CompatUser>, usize) -> Result<()>,
) -> Result<()> {
    let node = load_public_node(cfg)?;
    let mut users = store::load_users(&cfg.users_file())?;
    let previous = users.clone();
    let i = users
        .iter()
        .position(|u| u.id == id)
        .with_context(|| format!("user {id:?} not found"))?;
    f(&node, &mut users, i)?;
    super::apply_users_and_save(cfg, &previous, &users)?;
    Ok(())
}
