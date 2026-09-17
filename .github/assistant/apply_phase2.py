from pathlib import Path


def read(path):
    return Path(path).read_text()


def write(path, text):
    Path(path).write_text(text)


def replace_once(path, old, new):
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count} for:\n{old[:500]}")
    write(path, text.replace(old, new, 1))


# ---------------------------------------------------------------------------
# deployment.toml: schema-v2 fresh files + explicit node identity/role.
# ---------------------------------------------------------------------------
path = "deploy/almalinux/templates/deployment.toml.template"
text = read(path)
text = text.replace("schema_version = 1\n\npublic_host", 'schema_version = 2\nnode_id = "{{NODE_ID}}"\nrole = "{{NODE_ROLE}}"\n\npublic_host', 1)
write(path, text)

# ---------------------------------------------------------------------------
# Deployment model: derive the concrete destinations a relay is allowed to
# reach. A relay may be intentionally unpaired (zero targets), but it MUST
# declare its local reality-1 as relay infrastructure. The server renderer
# then rejects everything until at least one peer exit is configured.
# ---------------------------------------------------------------------------
path = "crates/compat-config/src/deployment.rs"
text = read(path)
marker = "#[derive(Clone, Debug, Serialize, Deserialize)]\npub struct UdpProbeSection {"
insert = '''#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayTarget {
    pub host: String,
    pub port: u16,
}

'''
if marker not in text:
    raise SystemExit("deployment.rs: RelayTarget insertion marker not found")
text = text.replace(marker, insert + marker, 1)

old = '''        if self.schema_version >= 2 {
            if self.node_id.trim().is_empty() {
                return Err(CompatError::Parse(
                    "schema-v2 deployment.toml requires a non-empty node_id".into(),
                ));
            }
            if !self
                .node_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return Err(CompatError::Parse(format!(
                    "node_id {:?} contains unsupported characters (allowed: ASCII letters, digits, '-', '_', '.')",
                    self.node_id
                )));
            }
        }
'''
new = old + '''        if self.schema_version >= 2 && self.role == NodeRole::Relay {
            let has_local_relay_ingress = self.access_paths.iter().any(|path| {
                path.kind == "relay" && path.via_endpoint_id.as_deref() == Some("reality-1")
            });
            if !has_local_relay_ingress {
                return Err(CompatError::Parse(
                    "role=relay requires a [[access_paths]] relay whose via_endpoint_id is \\"reality-1\\". This declaration marks the local VLESS+REALITY listener as first-hop infrastructure so it is never exposed as a final exit.".into(),
                ));
            }
        }
'''
if text.count(old) != 1:
    raise SystemExit("deployment.rs: schema-v2 validation block changed")
text = text.replace(old, new, 1)

# Add relay_targets() near the existing path helpers.
marker = '''    pub fn users_file(&self) -> PathBuf {
        self.state_dir.join("users/users.json")
    }
'''
addition = '''    /// Concrete destinations this node may dial when it is a relay.
    ///
    /// Only peer endpoints that explicitly use a `kind = "relay"` access
    /// path whose first hop is this node's local `reality-1` listener count.
    /// The list is intentionally empty while a freshly installed relay is
    /// unpaired; the role-aware server renderer treats that as reject-all.
    pub fn relay_targets(&self) -> Vec<RelayTarget> {
        let relay_paths: std::collections::BTreeSet<&str> = self
            .access_paths
            .iter()
            .filter(|path| {
                path.kind == "relay" && path.via_endpoint_id.as_deref() == Some("reality-1")
            })
            .map(|path| path.id.as_str())
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut targets = Vec::new();
        for peer in &self.peer_endpoints {
            if relay_paths.contains(peer.path.as_str())
                && seen.insert((peer.host.clone(), peer.port))
            {
                targets.push(RelayTarget {
                    host: peer.host.clone(),
                    port: peer.port,
                });
            }
        }
        targets
    }

'''
if text.count(marker) != 1:
    raise SystemExit("deployment.rs: users_file marker changed")
text = text.replace(marker, addition + marker, 1)
write(path, text)

# ---------------------------------------------------------------------------
# Server-side relay enforcement. Exit rendering is byte-for-byte unchanged;
# relay rendering adds allow-only rules to declared peer exits and a final
# reject. An unpaired relay therefore rejects all forwarded application
# traffic instead of accidentally becoming a Russian exit node.
# ---------------------------------------------------------------------------
path = "crates/compat-config/src/server.rs"
text = read(path)
text = text.replace(
    "use crate::model::{CompatUser, Hysteria2ServerParams, RealityServerParams};\n",
    "use crate::deployment::{DeploymentConfig, NodeRole};\nuse crate::model::{CompatUser, Hysteria2ServerParams, RealityServerParams};\n",
    1,
)
marker = '''/// Confirms the rendered config never contains anything it shouldn't
'''
addition = r'''/// Production server renderer that applies deployment-role policy on top of
/// the transport renderer above. Exit nodes preserve the historical config
/// exactly. Relay nodes can only forward application traffic to explicitly
/// declared peer exits used by a local `reality-1` relay path; everything
/// else arriving through either authenticated VPN inbound is rejected.
pub fn render_singbox_server_config_for_deployment(
    users: &[CompatUser],
    reality: &RealityServerParams,
    hysteria: &Hysteria2ServerParams,
    ports: ServerPorts,
    now_unix: i64,
    deployment: &DeploymentConfig,
) -> serde_json::Value {
    let mut config = render_singbox_server_config(users, reality, hysteria, ports, now_unix);
    if deployment.role != NodeRole::Relay {
        return config;
    }

    let inbound_tags = ["vless-reality-in", "hysteria2-in"];
    let mut rules = Vec::new();
    for target in deployment.relay_targets() {
        let mut rule = json!({
            "inbound": inbound_tags,
            "network": "tcp",
            "port": target.port,
            "action": "route",
            "outbound": "direct",
        });
        match target.host.parse::<std::net::IpAddr>() {
            Ok(ip) => {
                let prefix = if ip.is_ipv4() { 32 } else { 128 };
                rule["ip_cidr"] = json!([format!("{ip}/{prefix}")]);
            }
            Err(_) => {
                rule["domain"] = json!([target.host]);
            }
        }
        rules.push(rule);
    }
    // This rule is intentionally present even when `rules` was empty before
    // it. A freshly installed but not-yet-paired relay is therefore a closed
    // ingress, never an unrestricted exit by accident.
    rules.push(json!({
        "inbound": inbound_tags,
        "action": "reject",
        "method": "default",
    }));
    config["route"] = json!({ "rules": rules });
    config
}

'''
if marker not in text:
    raise SystemExit("server.rs: role-aware renderer insertion marker not found")
text = text.replace(marker, addition + marker, 1)
write(path, text)

# ---------------------------------------------------------------------------
# vpn-admin must use the role-aware renderer at EVERY production render site
# (normal apply, key rotation, Hysteria2 obfs rotation, doctor drift check).
# ---------------------------------------------------------------------------
path = "apps/admin/src/main.rs"
text = read(path)
old = '''use compat_config::server::{
    apply_config_atomically, config_backup_path, render_singbox_server_config,
    CompatibilityBackend, ServerPorts, SingBoxBackend,
};'''
new = '''use compat_config::server::{
    apply_config_atomically, config_backup_path, render_singbox_server_config_for_deployment,
    CompatibilityBackend, ServerPorts, SingBoxBackend,
};'''
if text.count(old) != 1:
    raise SystemExit("admin: server import block changed")
text = text.replace(old, new, 1)
repls = {
'''    let candidate_doc =
        render_singbox_server_config(&users, &candidate_reality, &hysteria, ports, now);''':
'''    let candidate_doc = render_singbox_server_config_for_deployment(
        &users, &candidate_reality, &hysteria, ports, now, cfg,
    );''',
'''    let candidate_doc =
        render_singbox_server_config(&users, &reality, &candidate_hysteria, ports, now);''':
'''    let candidate_doc = render_singbox_server_config_for_deployment(
        &users, &reality, &candidate_hysteria, ports, now, cfg,
    );''',
'''    let doc = render_singbox_server_config(users, &reality, &hysteria, ports, now);''':
'''    let doc = render_singbox_server_config_for_deployment(
        users, &reality, &hysteria, ports, now, cfg,
    );''',
'''    let fresh_server_doc = render_singbox_server_config(&users, &reality, &hysteria, ports, now);''':
'''    let fresh_server_doc = render_singbox_server_config_for_deployment(
        &users, &reality, &hysteria, ports, now, cfg,
    );''',
}
for old, new in repls.items():
    if text.count(old) != 1:
        raise SystemExit(f"admin: expected exactly one render call shape, found {text.count(old)}: {old[:120]}")
    text = text.replace(old, new, 1)

status_marker = '''fn cmd_status(cfg: &DeploymentConfig) -> Result<()> {
    let users = store::load_users(&cfg.users_file())?;
'''
status_new = '''fn cmd_status(cfg: &DeploymentConfig) -> Result<()> {
    let node_label = if cfg.node_id.is_empty() {
        cfg.public_host.as_str()
    } else {
        cfg.node_id.as_str()
    };
    println!("Node: {node_label} (role: {})", cfg.role.as_str());
    if cfg.role == compat_config::deployment::NodeRole::Relay {
        let targets = cfg.relay_targets();
        if targets.is_empty() {
            println!("Relay pairing: UNPAIRED (forwarding is reject-all until an exit is declared)");
        } else {
            println!("Relay pairing: {} declared exit target(s)", targets.len());
        }
    }
    println!();
    let users = store::load_users(&cfg.users_file())?;
'''
if text.count(status_marker) != 1:
    raise SystemExit("admin: cmd_status marker changed")
text = text.replace(status_marker, status_new, 1)
write(path, text)

# ---------------------------------------------------------------------------
# Subscription fail-closed behavior: access-path first-hop endpoints are
# infrastructure, not user-selectable exits. Legacy share-link formats cannot
# represent detour, and an unpaired relay returns 503 rather than advertising
# its local ingress as a direct exit.
# ---------------------------------------------------------------------------
path = "services/subscription/src/lib.rs"
text = read(path)
marker = '''async fn get_subscription(
'''
helper = '''fn selectable_endpoints_for_access_paths(
    endpoints: &[compat_config::model::CompatEndpoint],
    access_paths: &[contract::AccessPath],
) -> Vec<compat_config::model::CompatEndpoint> {
    let infrastructure_ids: std::collections::BTreeSet<&str> = access_paths
        .iter()
        .filter(|path| matches!(path.kind, contract::AccessPathKind::Relay))
        .filter_map(|path| path.via_endpoint_id.as_deref())
        .collect();
    endpoints
        .iter()
        .filter(|endpoint| !infrastructure_ids.contains(endpoint.id.as_str()))
        .cloned()
        .collect()
}

'''
if marker not in text:
    raise SystemExit("subscription: handler marker not found")
text = text.replace(marker, helper + marker, 1)

old = '''    match format {
        "singbox" => {
'''
new = '''    let selectable_endpoints =
        selectable_endpoints_for_access_paths(&state.endpoints, &state.access_paths);
    if !state.access_paths.is_empty() && selectable_endpoints.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "relay is installed but no selectable exit is paired yet",
        )
            .into_response();
    }

    match format {
        "singbox" => {
'''
if text.count(old) != 1:
    raise SystemExit("subscription: format match marker changed")
text = text.replace(old, new, 1)
text = text.replace(
    'render::render_vision_off_uri_list(&user, &state.endpoints)',
    'render::render_vision_off_uri_list(&user, &selectable_endpoints)',
    1,
)
text = text.replace(
    'render::render_uri_list(&user, &state.endpoints)',
    'render::render_uri_list(&user, &selectable_endpoints)',
    1,
)

old = '''    tracing::debug!(
        user_id = %user.id,
        schema_version = contract::SCHEMA_VERSION,
        diagnostic = ?query.diagnostic,
        "provisioning contract served"
    );

    let doc = match compat_config::contract::provisioning_document_with_mode_and_access_paths(
'''
new = '''    tracing::debug!(
        user_id = %user.id,
        schema_version = contract::SCHEMA_VERSION,
        diagnostic = ?query.diagnostic,
        "provisioning contract served"
    );

    let selectable_endpoints =
        selectable_endpoints_for_access_paths(&state.endpoints, &state.access_paths);
    if !state.access_paths.is_empty() && selectable_endpoints.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "application/json")],
            serde_json::json!({
                "error": "relay_unpaired",
                "message": "relay is installed but no selectable exit is paired yet"
            })
            .to_string(),
        )
            .into_response();
    }

    let doc = match compat_config::contract::provisioning_document_with_mode_and_access_paths(
'''
if text.count(old) != 1:
    raise SystemExit("subscription: provisioning insertion marker changed")
text = text.replace(old, new, 1)
write(path, text)

# ---------------------------------------------------------------------------
# Installer role/node flags and relay-safe first install. Fresh relay installs
# write a placeholder access path that marks reality-1 as infrastructure; no
# exit peer exists yet, so server policy is reject-all and onboarding waits for
# pairing. Exit-node acceptance behavior remains unchanged.
# ---------------------------------------------------------------------------
path = "deploy/almalinux/install.sh"
text = read(path)
old = '''DRY_RUN=0
print_install_help() {'''
new = '''DRY_RUN=0
NODE_ROLE="${SINGBOX_VPN_ROLE:-exit}"
NODE_ID="${SINGBOX_VPN_NODE_ID:-}"
ROLE_EXPLICIT=0
NODE_ID_EXPLICIT=0
print_install_help() {'''
if text.count(old) != 1:
    raise SystemExit("install: DRY_RUN marker changed")
text = text.replace(old, new, 1)

old = '''  --ssh-port PORT                  same as SINGBOX_VPN_SSH_PORT; explicitly declares
                                    the port sshd listens on when singbox-vpn cannot
'''
new = '''  --role exit|relay               node role (default: exit). Relay installs are
                                    fail-closed until an exit peer is paired.
  --node-id ID                     stable operator-facing node id; defaults to the
                                    first label of --domain / PUBLIC_HOST.
  --ssh-port PORT                  same as SINGBOX_VPN_SSH_PORT; explicitly declares
                                    the port sshd listens on when singbox-vpn cannot
'''
if text.count(old) != 1:
    raise SystemExit("install: help insertion marker changed")
text = text.replace(old, new, 1)

old = '''      --subscription-port) SUBSCRIPTION_PORT="$2"; shift 2 ;;
      --subscription-port=*) SUBSCRIPTION_PORT="${1#*=}"; shift ;;
      --ssh-port) SINGBOX_VPN_SSH_PORT="$2"; shift 2 ;;
'''
new = '''      --subscription-port) SUBSCRIPTION_PORT="$2"; shift 2 ;;
      --subscription-port=*) SUBSCRIPTION_PORT="${1#*=}"; shift ;;
      --role) NODE_ROLE="$2"; ROLE_EXPLICIT=1; shift 2 ;;
      --role=*) NODE_ROLE="${1#*=}"; ROLE_EXPLICIT=1; shift ;;
      --node-id) NODE_ID="$2"; NODE_ID_EXPLICIT=1; shift 2 ;;
      --node-id=*) NODE_ID="${1#*=}"; NODE_ID_EXPLICIT=1; shift ;;
      --ssh-port) SINGBOX_VPN_SSH_PORT="$2"; shift 2 ;;
'''
if text.count(old) != 1:
    raise SystemExit("install: parse args insertion marker changed")
text = text.replace(old, new, 1)

old = '''load_existing_host_config() {
  [ -f "$DEPLOYMENT_TOML" ] || return 1
  local existing_public existing_sub
  existing_public="$(grep -E '^public_host' "$DEPLOYMENT_TOML" | sed -E 's/^public_host *= *"([^"]*)".*/\\1/')"
  existing_sub="$(grep -E '^subscription_host' "$DEPLOYMENT_TOML" | sed -E 's/^subscription_host *= *"([^"]*)".*/\\1/')"
  [ -n "$existing_public" ] || return 1
  PUBLIC_HOST="$existing_public"
  SUBSCRIPTION_HOST="${existing_sub:-$existing_public}"
  return 0
}
'''
new = '''load_existing_host_config() {
  [ -f "$DEPLOYMENT_TOML" ] || return 1
  local existing_public existing_sub existing_role existing_node
  existing_public="$(grep -E '^public_host' "$DEPLOYMENT_TOML" | sed -E 's/^public_host *= *"([^"]*)".*/\\1/')"
  existing_sub="$(grep -E '^subscription_host' "$DEPLOYMENT_TOML" | sed -E 's/^subscription_host *= *"([^"]*)".*/\\1/')"
  existing_role="$(grep -E '^role' "$DEPLOYMENT_TOML" | sed -E 's/^role *= *"([^"]*)".*/\\1/' || true)"
  existing_node="$(grep -E '^node_id' "$DEPLOYMENT_TOML" | sed -E 's/^node_id *= *"([^"]*)".*/\\1/' || true)"
  [ -n "$existing_public" ] || return 1
  existing_role="${existing_role:-exit}"
  if [ "$ROLE_EXPLICIT" -eq 1 ] && [ "$NODE_ROLE" != "$existing_role" ]; then
    die "existing deployment role is '$existing_role' but this repair requested --role '$NODE_ROLE'. Refusing an implicit in-place role conversion; reinstall or edit/pair deployment.toml intentionally."
  fi
  if [ "$NODE_ID_EXPLICIT" -eq 1 ] && [ -n "$existing_node" ] && [ "$NODE_ID" != "$existing_node" ]; then
    die "existing deployment node_id is '$existing_node' but this repair requested --node-id '$NODE_ID'. Refusing to silently rename a live node."
  fi
  PUBLIC_HOST="$existing_public"
  SUBSCRIPTION_HOST="${existing_sub:-$existing_public}"
  NODE_ROLE="$existing_role"
  NODE_ID="${existing_node:-$NODE_ID}"
  return 0
}
'''
if text.count(old) != 1:
    raise SystemExit("install: load_existing_host_config block changed")
text = text.replace(old, new, 1)

# Resolve node identity after PUBLIC_HOST is known, inside resolve_host_config.
needle = '''  export PUBLIC_HOST SUBSCRIPTION_HOST
}

host_config_stage() {'''
replacement = '''  case "$NODE_ROLE" in
    exit|relay) ;;
    *) die "invalid --role '$NODE_ROLE' (expected exit or relay)" ;;
  esac
  if [ -z "$NODE_ID" ]; then
    NODE_ID="${PUBLIC_HOST%%.*}"
    NODE_ID="$(printf '%s' "$NODE_ID" | sed -E 's/[^A-Za-z0-9._-]+/-/g; s/^-+//; s/-+$//')"
    [ -n "$NODE_ID" ] || NODE_ID="node"
  fi
  case "$NODE_ID" in
    *[!A-Za-z0-9._-]*|'') die "invalid --node-id '$NODE_ID' (allowed: ASCII letters, digits, '.', '_', '-')" ;;
  esac
  export PUBLIC_HOST SUBSCRIPTION_HOST NODE_ROLE NODE_ID
}

host_config_stage() {'''
if text.count(needle) != 1:
    raise SystemExit("install: resolve_host_config tail changed")
text = text.replace(needle, replacement, 1)

old = '''  sed -e "s/{{PUBLIC_HOST}}/$PUBLIC_HOST/" \\
      -e "s/{{SUBSCRIPTION_HOST}}/$SUBSCRIPTION_HOST/" \\
      -e "s/{{SUBSCRIPTION_PORT}}/$SUBSCRIPTION_PORT/" \\
      -e "s/{{REALITY_HANDSHAKE_SERVER}}/$REALITY_HANDSHAKE_SERVER/" \\
      "$REPO_ROOT/deploy/almalinux/templates/deployment.toml.template" >"$DEPLOYMENT_TOML"
  chmod 0644 "$DEPLOYMENT_TOML"
'''
new = '''  sed -e "s/{{PUBLIC_HOST}}/$PUBLIC_HOST/" \\
      -e "s/{{SUBSCRIPTION_HOST}}/$SUBSCRIPTION_HOST/" \\
      -e "s/{{SUBSCRIPTION_PORT}}/$SUBSCRIPTION_PORT/" \\
      -e "s/{{REALITY_HANDSHAKE_SERVER}}/$REALITY_HANDSHAKE_SERVER/" \\
      -e "s/{{NODE_ID}}/$NODE_ID/" \\
      -e "s/{{NODE_ROLE}}/$NODE_ROLE/" \\
      "$REPO_ROOT/deploy/almalinux/templates/deployment.toml.template" >"$DEPLOYMENT_TOML"
  if [ "$NODE_ROLE" = "relay" ]; then
    cat >>"$DEPLOYMENT_TOML" <<'EOF'

# Relay ingress declaration. `reality-1` is infrastructure, not a final exit.
# Pair an exit by adding a [[peer_endpoints]] entry whose path is
# "relay-egress"; until then the relay renderer rejects all forwarded traffic.
[[access_paths]]
id = "relay-egress"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]
EOF
  fi
  chmod 0644 "$DEPLOYMENT_TOML"
'''
if text.count(old) != 1:
    raise SystemExit("install: template render block changed")
text = text.replace(old, new, 1)

# Relay acceptance is intentionally different: the unpaired state is safe but
# cannot pass an end-to-end exit/subscription test by definition.
old = '''acceptance_stage() {
  stage 17 "first user + acceptance test"
  ensure_first_user
  if ! "$BIN_DIR/vpn-health-check"; then
'''
new = '''acceptance_stage() {
  stage 17 "first user + acceptance test"
  if [ "$NODE_ROLE" = "relay" ]; then
    log "relay role: validating the installed ingress in fail-closed/unpaired mode; user onboarding and end-to-end exit acceptance begin only after an exit peer is paired."
    if ! "$BIN_DIR/vpn-health-check"; then
      die "post-install relay health check failed — see output above. Installation did not complete cleanly."
    fi
    local relay_doctor_output relay_doctor_rc=0
    relay_doctor_output="$("$BIN_DIR/vpn" --config "$DEPLOYMENT_TOML" doctor 2>&1)" || relay_doctor_rc=$?
    if [ "$relay_doctor_rc" -ne 0 ]; then
      echo "$relay_doctor_output" >&2
      die "post-install relay L1-L4 checks failed. The relay remains fail-closed; fix the [FAIL] line(s) above before pairing an exit."
    fi
    log "relay ingress checks passed; forwarding remains reject-all until an exit peer is paired."
    return
  fi
  ensure_first_user
  if ! "$BIN_DIR/vpn-health-check"; then
'''
if text.count(old) != 1:
    raise SystemExit("install: acceptance_stage start changed")
text = text.replace(old, new, 1)

# Manifest includes identity.
old = '''  "public_host": "$PUBLIC_HOST",
  "subscription_host": "${SUBSCRIPTION_HOST:-$PUBLIC_HOST}",
'''
new = '''  "node_id": "$NODE_ID",
  "role": "$NODE_ROLE",
  "public_host": "$PUBLIC_HOST",
  "subscription_host": "${SUBSCRIPTION_HOST:-$PUBLIC_HOST}",
'''
if text.count(old) != 1:
    raise SystemExit("install: manifest identity marker changed")
text = text.replace(old, new, 1)

# Summary: relay does not require a subscription fetch and must not claim a
# standalone end-to-end handshake it intentionally cannot perform unpaired.
old = '''  if [ "$PRIOR_ACCEPTANCE_STATE" != "accepted" ]; then
    [ "$SUBSCRIPTION_FETCH_OK" -eq 1 ] || die "[FAIL] internal: reached summary without the subscription-through-nginx fetch confirmed on a non-repair run — this should be unreachable (verify_subscription_through_nginx should have aborted first)."
  fi
'''
new = '''  if [ "$NODE_ROLE" != "relay" ] && [ "$PRIOR_ACCEPTANCE_STATE" != "accepted" ]; then
    [ "$SUBSCRIPTION_FETCH_OK" -eq 1 ] || die "[FAIL] internal: reached summary without the subscription-through-nginx fetch confirmed on a non-repair run — this should be unreachable (verify_subscription_through_nginx should have aborted first)."
  fi
'''
if text.count(old) != 1:
    raise SystemExit("install: summary fetch assertion changed")
text = text.replace(old, new, 1)

# Put identity in the human banner and replace the protocol/fetch lines with
# role-aware text without disturbing the existing exit claims.
old = '''Server
  Address: ${PUBLIC_HOST}
  Status:  running

SERVER-SIDE VERIFIED
  ✓ VLESS+REALITY service active, 443/tcp listener bound
  ✓ Hysteria2 443/udp listener bound
  ✓ REALITY protocol self-test (real sing-box handshake)
  ✓ subscription backend healthy
  ✓ subscription HTTPS/TLS (nginx, real cert/hostname verified)
  $([ "$SUBSCRIPTION_FETCH_OK" -eq 1 ] && echo "✓ subscription profile fetched through nginx and matches current server state" || echo "- subscription profile re-fetch skipped this run (repair of an already-accepted install; not re-minting a token)")
  ✓ firewall configured$([ "$FIREWALL_OK" -eq 1 ] || echo " (unconfirmed — see stage 13 output)")
BANNER
'''
new = '''Server
  Node:    ${NODE_ID}
  Role:    ${NODE_ROLE}
  Address: ${PUBLIC_HOST}
  Status:  running

SERVER-SIDE VERIFIED
  ✓ VLESS+REALITY service active, 443/tcp listener bound
  ✓ Hysteria2 443/udp listener bound
  $([ "$NODE_ROLE" = "relay" ] && echo "✓ relay forwarding policy is fail-closed (only declared exit targets are allowed; currently reject-all until paired)" || echo "✓ REALITY protocol self-test (real sing-box handshake)")
  ✓ subscription backend healthy
  ✓ subscription HTTPS/TLS (nginx, real cert/hostname verified)
  $([ "$NODE_ROLE" = "relay" ] && echo "- client onboarding deferred until an exit peer is paired" || ([ "$SUBSCRIPTION_FETCH_OK" -eq 1 ] && echo "✓ subscription profile fetched through nginx and matches current server state" || echo "- subscription profile re-fetch skipped this run (repair of an already-accepted install; not re-minting a token)"))
  ✓ firewall configured$([ "$FIREWALL_OK" -eq 1 ] || echo " (unconfirmed — see stage 13 output)")
BANNER
'''
if text.count(old) != 1:
    raise SystemExit("install: summary banner block changed")
text = text.replace(old, new, 1)
write(path, text)

# Root bootstrap help: flags are passed through already, document them.
path = "install.sh"
text = read(path)
old = '''  --subscription-port PORT         public HTTPS port for the subscription
                                    endpoint (default 8443)
  --non-interactive                never prompt; fail fast instead
'''
new = '''  --subscription-port PORT         public HTTPS port for the subscription
                                    endpoint (default 8443)
  --role exit|relay               node role (default: exit); relay is
                                    fail-closed until a peer exit is paired
  --node-id ID                     stable node identity (default: domain label)
  --non-interactive                never prompt; fail fast instead
'''
if text.count(old) != 1:
    raise SystemExit("root install: help marker changed")
text = text.replace(old, new, 1)
write(path, text)

# ---------------------------------------------------------------------------
# Role-aware server-policy integration test, including a real sing-box parser
# when the pinned binary is available/required by CI.
# ---------------------------------------------------------------------------
Path("crates/compat-config/tests/relay_role_policy.rs").write_text(r'''mod common;

use compat_config::deployment::{DeploymentConfig, NodeRole};
use compat_config::model::{CompatUser, Hysteria2ServerParams, RealityServerParams};
use compat_config::secret::SecretString;
use compat_config::server::{
    render_singbox_server_config, render_singbox_server_config_for_deployment, ServerPorts,
};
use std::path::Path;

fn deployment(role: &str, with_peer: bool) -> (tempfile::TempDir, DeploymentConfig) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    let peer = if with_peer {
        r#"
[[peer_endpoints]]
id = "de1"
tag = "Germany via RU"
host = "de1.example.test"
port = 443
transport = "vless_reality"
server_name = "www.example.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "de1"
path = "relay-egress"
"#
    } else {
        ""
    };
    let access_path = if role == "relay" {
        r#"
[[access_paths]]
id = "relay-egress"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]
"#
    } else {
        ""
    };
    std::fs::write(
        &path,
        format!(
            r#"schema_version = 2
node_id = "test-node"
role = "{role}"
public_host = "ru1.example.test"
subscription_host = "ru1.example.test"

[reality]
listen_port = 443
handshake_server = "www.example.com"
handshake_port = 443

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
public_port = 8443
{access_path}{peer}"#
        ),
    )
    .unwrap();
    let cfg = DeploymentConfig::load(Path::new(&path)).unwrap();
    (dir, cfg)
}

fn users() -> Vec<CompatUser> {
    vec![CompatUser {
        id: "u1".into(),
        name: "relay-test".into(),
        enabled: true,
        vless_uuid: "841cac4a-efe4-48ac-92b8-d11f4c98c45e".into(),
        hysteria2_password: SecretString::new("test-password"),
        subscription_token_hash_hex: "unused".into(),
        vision_off_experiment: false,
        created_at: 0,
        expires_at: None,
        peer_credentials: Default::default(),
    }]
}

fn reality() -> RealityServerParams {
    RealityServerParams {
        private_key_hex: SecretString::new("private-key-not-parsed-in-structural-test"),
        public_key_hex: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8".into(),
        short_ids: vec!["0a1b2c3d".into()],
        handshake_server: "www.example.com".into(),
        handshake_port: 443,
    }
}

fn hysteria() -> Hysteria2ServerParams {
    Hysteria2ServerParams {
        tls_cert_path: "/tmp/nonexistent-cert".into(),
        tls_key_path: "/tmp/nonexistent-key".into(),
        obfs_password: None,
        masquerade_dir_path: None,
        up_mbps: None,
        down_mbps: None,
    }
}

fn ports() -> ServerPorts {
    ServerPorts {
        vless_reality_port: 443,
        hysteria2_port: 443,
    }
}

#[test]
fn relay_targets_are_derived_only_from_declared_relay_exit_peers() {
    let (_dir, cfg) = deployment("relay", true);
    assert_eq!(cfg.role, NodeRole::Relay);
    let targets = cfg.relay_targets();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].host, "de1.example.test");
    assert_eq!(targets[0].port, 443);
}

#[test]
fn paired_relay_allows_only_declared_exit_then_rejects_everything_else() {
    let (_dir, cfg) = deployment("relay", true);
    let doc = render_singbox_server_config_for_deployment(
        &users(), &reality(), &hysteria(), ports(), 0, &cfg,
    );
    let rules = doc["route"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0]["action"], "route");
    assert_eq!(rules[0]["outbound"], "direct");
    assert_eq!(rules[0]["network"], "tcp");
    assert_eq!(rules[0]["port"], 443);
    assert_eq!(rules[0]["domain"][0], "de1.example.test");
    assert_eq!(rules[1]["action"], "reject");
    assert_eq!(rules[1]["method"], "default");
}

#[test]
fn unpaired_relay_is_reject_all_not_an_accidental_exit() {
    let (_dir, cfg) = deployment("relay", false);
    assert!(cfg.relay_targets().is_empty());
    let doc = render_singbox_server_config_for_deployment(
        &users(), &reality(), &hysteria(), ports(), 0, &cfg,
    );
    let rules = doc["route"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["action"], "reject");
}

#[test]
fn exit_renderer_is_unchanged_by_role_feature() {
    let (_dir, cfg) = deployment("exit", false);
    let old = render_singbox_server_config(&users(), &reality(), &hysteria(), ports(), 0);
    let role_aware = render_singbox_server_config_for_deployment(
        &users(), &reality(), &hysteria(), ports(), 0, &cfg,
    );
    assert_eq!(old, role_aware);
    assert!(role_aware.get("route").is_none());
}

#[test]
fn relay_route_section_is_accepted_by_real_sing_box() {
    let Some(sb) = common::SingBox::find() else {
        if std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok() {
            panic!("real sing-box is required by CI but was not found");
        }
        eprintln!("skipping real sing-box route parse: binary not found");
        return;
    };
    let (_dir, cfg) = deployment("relay", true);
    let production = render_singbox_server_config_for_deployment(
        &users(), &reality(), &hysteria(), ports(), 0, &cfg,
    );
    let p1 = common::free_port();
    let p2 = common::free_port();
    let minimal = serde_json::json!({
        "inbounds": [
            {"type":"mixed","tag":"vless-reality-in","listen":"127.0.0.1","listen_port":p1},
            {"type":"mixed","tag":"hysteria2-in","listen":"127.0.0.1","listen_port":p2}
        ],
        "outbounds": [{"type":"direct","tag":"direct"}],
        "route": production["route"].clone()
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay-route.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&minimal).unwrap()).unwrap();
    let out = sb.check(&path);
    assert!(
        out.status.success(),
        "real sing-box rejected production relay route rules:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
''')

print("phase2 relay role hardening edits applied")
