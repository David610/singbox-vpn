#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

def read(path):
    return (ROOT / path).read_text()

def write(path, text):
    (ROOT / path).write_text(text)

def replace_once(path, old, new):
    text = read(path)
    n = text.count(old)
    if n != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {n} for:\n{old[:300]}")
    write(path, text.replace(old, new, 1))

# ---------------------------------------------------------------------------
# provisioning-contract: access paths can point at a concrete first-hop
# endpoint without carrying any secret material.
# ---------------------------------------------------------------------------
path = "crates/provisioning-contract/src/lib.rs"
replace_once(path,
'''pub struct AccessPath {
    pub id: String,
    pub kind: AccessPathKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_domain: Option<String>,''',
'''pub struct AccessPath {
    pub id: String,
    pub kind: AccessPathKind,
    /// Endpoint id of the authenticated first hop used by this path.
    /// This is a non-secret reference only: credentials stay in the
    /// embedded Core configuration and never enter access-path metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_endpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_domain: Option<String>,''')
replace_once(path,
'''            id: id.into(),
            kind,
            failure_domain: None,''',
'''            id: id.into(),
            kind,
            via_endpoint_id: None,
            failure_domain: None,''')
replace_once(path,
'''    pub fn with_metadata(
        mut self,
        failure_domain: Option<String>,''',
'''    pub fn with_via_endpoint_id(mut self, endpoint_id: impl Into<String>) -> Self {
        self.via_endpoint_id = Some(endpoint_id.into());
        self
    }

    pub fn with_metadata(
        mut self,
        failure_domain: Option<String>,''')
replace_once(path,
'''        non_empty("access_paths[].id", &self.id)?;
        non_empty("access_paths[].kind", self.kind.as_str())?;
        if self.capabilities.is_empty() {''',
'''        non_empty("access_paths[].id", &self.id)?;
        non_empty("access_paths[].kind", self.kind.as_str())?;
        if let Some(via) = &self.via_endpoint_id {
            non_empty("access_paths[].via_endpoint_id", via)?;
        }
        if self.capabilities.is_empty() {''')

# ---------------------------------------------------------------------------
# compat model: route aliases can deliberately reuse one peer credential.
# ---------------------------------------------------------------------------
path = "crates/compat-config/src/model.rs"
replace_once(path,
'''    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}''',
'''    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Optional peer-endpoint id whose per-user credential authenticates
    /// this route. This lets a direct exit and a via-relay alias share the
    /// one credential issued by that exit instead of duplicating secrets.
    /// Local endpoints never use this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}''')

# ---------------------------------------------------------------------------
# deployment schema v2: node identity/role, relay first-hop references and
# peer credential aliases. Old configs deserialize with safe exit defaults;
# migration stamps identity explicitly so older binaries fail closed on v2.
# ---------------------------------------------------------------------------
path = "crates/compat-config/src/deployment.rs"
replace_once(path, 'pub const DEPLOYMENT_SCHEMA_VERSION: u32 = 1;', 'pub const DEPLOYMENT_SCHEMA_VERSION: u32 = 2;')
replace_once(path,
'''#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {''',
'''#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    #[default]
    Exit,
    Relay,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeRole::Exit => "exit",
            NodeRole::Relay => "relay",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {''')
replace_once(path,
'''    #[serde(default)]
    pub schema_version: u32,

    /// Public hostname/IP clients connect''',
'''    #[serde(default)]
    pub schema_version: u32,

    /// Stable operator-facing node identity. Schema-v2 files always carry
    /// this explicitly; legacy files default empty until migrated.
    #[serde(default)]
    pub node_id: String,
    /// Operational role. Legacy deployments were ordinary exits, so the
    /// serde default deliberately preserves that behavior.
    #[serde(default)]
    pub role: NodeRole,

    /// Public hostname/IP clients connect''')
replace_once(path,
'''pub struct AccessPathSection {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub failure_domain: Option<String>,''',
'''pub struct AccessPathSection {
    pub id: String,
    pub kind: String,
    /// Non-secret endpoint-id reference for a real first hop. A relay path
    /// referenced by an endpoint must set this; credentials remain in the
    /// per-user endpoint model, never in this metadata block.
    #[serde(default)]
    pub via_endpoint_id: Option<String>,
    #[serde(default)]
    pub failure_domain: Option<String>,''')
replace_once(path,
'''        for (name, value) in [
            ("failure_domain", self.failure_domain.as_deref()),''',
'''        if self
            .via_endpoint_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(CompatError::Parse(format!(
                "[[access_paths]] {}: via_endpoint_id is empty",
                self.id
            )));
        }
        for (name, value) in [
            ("failure_domain", self.failure_domain.as_deref()),''')
replace_once(path,
'''        Ok(provisioning_contract::AccessPath::new(
            self.id.clone(),
            provisioning_contract::AccessPathKind::from_wire(&self.kind),
            self.capabilities.clone(),
        )
        .with_metadata(
            self.failure_domain.clone(),
            self.region.clone(),
            self.provider.clone(),
        ))''',
'''        let mut path = provisioning_contract::AccessPath::new(
            self.id.clone(),
            provisioning_contract::AccessPathKind::from_wire(&self.kind),
            self.capabilities.clone(),
        )
        .with_metadata(
            self.failure_domain.clone(),
            self.region.clone(),
            self.provider.clone(),
        );
        if let Some(via) = &self.via_endpoint_id {
            path = path.with_via_endpoint_id(via.clone());
        }
        Ok(path)''')
replace_once(path,
'''    #[serde(default = "default_peer_path")]
    pub path: String,

    #[serde(flatten)]''',
'''    #[serde(default = "default_peer_path")]
    pub path: String,
    /// Optional endpoint id whose per-user credential should be reused by
    /// this route alias (for example DE-direct and DE-via-RU).
    #[serde(default)]
    pub credential_ref: Option<String>,

    #[serde(flatten)]''')
replace_once(path,
'''        if self.path.trim().is_empty() {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: path is empty",
                self.id
            )));
        }
        if self.host.trim().is_empty()''',
'''        if self.path.trim().is_empty() {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: path is empty",
                self.id
            )));
        }
        if self
            .credential_ref
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: credential_ref is empty",
                self.id
            )));
        }
        if self.host.trim().is_empty()''')
replace_once(path,
'''            asn: self.asn.clone(),
            path: Some(self.path.clone()),
        })''',
'''            asn: self.asn.clone(),
            path: Some(self.path.clone()),
            credential_ref: self.credential_ref.clone(),
        })''')
replace_once(path,
'''        if self.schema_version > DEPLOYMENT_SCHEMA_VERSION {
            return Err(CompatError::UnsupportedSchema {''',
'''        if self.schema_version > DEPLOYMENT_SCHEMA_VERSION {
            return Err(CompatError::UnsupportedSchema {''')
# Insert v2 node validation immediately after future-schema guard.
replace_once(path,
'''            });
        }
        let mut seen_access_path_ids: Vec<&str> = Vec::with_capacity(self.access_paths.len());''',
'''            });
        }
        if self.schema_version >= 2 {
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
        let mut seen_access_path_ids: Vec<&str> = Vec::with_capacity(self.access_paths.len());''')
# Extend the end of peer validation before hysteria bandwidth check.
replace_once(path,
'''        if self.hysteria2.up_mbps.is_some() != self.hysteria2.down_mbps.is_some() {''',
'''        // Route aliases may reuse a credential issued for another peer
        // endpoint, but only when that target exists and speaks the same
        // transport. Otherwise the generated profile would be confidently
        // undialable.
        for peer in &self.peer_endpoints {
            if let Some(credential_ref) = &peer.credential_ref {
                let target = self
                    .peer_endpoints
                    .iter()
                    .find(|candidate| candidate.id == *credential_ref)
                    .ok_or_else(|| {
                        CompatError::Parse(format!(
                            "[[peer_endpoints]] {}: credential_ref {:?} does not name a declared peer endpoint",
                            peer.id, credential_ref
                        ))
                    })?;
                if target.transport != peer.transport {
                    return Err(CompatError::Parse(format!(
                        "[[peer_endpoints]] {}: credential_ref {:?} is {}, but this alias is {}; credentials cannot cross transports",
                        peer.id,
                        credential_ref,
                        target.transport.as_str(),
                        peer.transport.as_str()
                    )));
                }
            }

            if peer.path != "direct" {
                if let Some(path) = self.access_paths.iter().find(|path| path.id == peer.path) {
                    if path.kind == "relay" {
                        if peer.transport != crate::model::CompatTransport::VlessReality {
                            return Err(CompatError::Parse(format!(
                                "[[peer_endpoints]] {}: relay path {:?} is TCP/VLESS-only in the current MVP; hysteria2-over-relay is not implemented",
                                peer.id, peer.path
                            )));
                        }
                        let via = path.via_endpoint_id.as_deref().ok_or_else(|| {
                            CompatError::Parse(format!(
                                "[[peer_endpoints]] {}: relay path {:?} has no via_endpoint_id; refusing to silently render it as a direct route",
                                peer.id, peer.path
                            ))
                        })?;
                        if via == peer.id {
                            return Err(CompatError::Parse(format!(
                                "[[peer_endpoints]] {}: relay path {:?} points back to itself",
                                peer.id, peer.path
                            )));
                        }
                        if via != "reality-1" {
                            let first_hop = self
                                .peer_endpoints
                                .iter()
                                .find(|candidate| candidate.id == via)
                                .ok_or_else(|| {
                                    CompatError::Parse(format!(
                                        "[[access_paths]] {}: via_endpoint_id {:?} is neither local reality-1 nor a declared peer endpoint",
                                        path.id, via
                                    ))
                                })?;
                            if first_hop.transport != crate::model::CompatTransport::VlessReality {
                                return Err(CompatError::Parse(format!(
                                    "[[access_paths]] {}: via_endpoint_id {:?} is not VLESS+REALITY; relay chaining is TCP/VLESS-only in this MVP",
                                    path.id, via
                                )));
                            }
                        }
                        if path.capabilities.iter().any(|cap| cap == "udp") {
                            return Err(CompatError::Parse(format!(
                                "[[access_paths]] {}: capability 'udp' is not implemented for the relay MVP; advertise only capabilities actually proven",
                                path.id
                            )));
                        }
                    }
                }
            }
        }

        if self.hysteria2.up_mbps.is_some() != self.hysteria2.down_mbps.is_some() {''')

# Replace the old marker-only migration with schema-v2 identity migration.
old = '''pub fn migrate_deployment_toml_text(original: &str) -> Option<String> {
    let already_versioned = original
        .lines()
        .any(|l| l.trim_start().starts_with("schema_version"));
    if already_versioned {
        return None;
    }
    Some(format!(
        "schema_version = {DEPLOYMENT_SCHEMA_VERSION}\\n{original}"
    ))
}'''
new = '''fn derived_legacy_node_id(original: &str) -> String {
    let host = original
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("public_host")
                .and_then(|rest| rest.split_once('=').map(|(_, value)| value))
                .map(|value| value.trim().trim_matches('"'))
        })
        .unwrap_or("legacy-node");
    let first = host.split('.').next().unwrap_or(host);
    let mut id: String = first
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while id.starts_with('-') {
        id.remove(0);
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        "legacy-node".to_string()
    } else {
        id
    }
}

pub fn migrate_deployment_toml_text(original: &str) -> Option<String> {
    let explicit_version = original.lines().find_map(|line| {
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix("schema_version")?;
        let (_, value) = rest.split_once('=')?;
        value.trim().parse::<u32>().ok()
    });
    let has_node_id = original
        .lines()
        .any(|line| line.trim_start().starts_with("node_id"));
    let has_role = original
        .lines()
        .any(|line| line.trim_start().starts_with("role"));
    if explicit_version == Some(DEPLOYMENT_SCHEMA_VERSION) && has_node_id && has_role {
        return None;
    }

    let mut body = String::new();
    let mut wrote_version = false;
    for line in original.lines() {
        if line.trim_start().starts_with("schema_version") {
            if !wrote_version {
                body.push_str(&format!("schema_version = {DEPLOYMENT_SCHEMA_VERSION}\\n"));
                wrote_version = true;
            }
        } else {
            body.push_str(line);
            body.push('\\n');
        }
    }
    if !wrote_version {
        body = format!("schema_version = {DEPLOYMENT_SCHEMA_VERSION}\\n{body}");
    }

    let mut identity = String::new();
    if !has_node_id {
        identity.push_str(&format!("node_id = {:?}\\n", derived_legacy_node_id(original)));
    }
    if !has_role {
        // Every deployment before role support was an ordinary unrestricted
        // exit. Migration must never invent a relay.
        identity.push_str("role = \\"exit\\"\\n");
    }
    if !identity.is_empty() {
        body = body.replacen(
            &format!("schema_version = {DEPLOYMENT_SCHEMA_VERSION}\\n"),
            &format!("schema_version = {DEPLOYMENT_SCHEMA_VERSION}\\n{identity}"),
            1,
        );
    }
    Some(body)
}'''
replace_once(path, old, new)
# Migration comparison ignores the new explicit-default identity fields.
replace_once(path,
'''    if let Some(obj) = original_normalized.as_object_mut() {
        obj.remove("schema_version");
    }
    if let Some(obj) = migrated_normalized.as_object_mut() {
        obj.remove("schema_version");
    }''',
'''    if let Some(obj) = original_normalized.as_object_mut() {
        obj.remove("schema_version");
        obj.remove("node_id");
        obj.remove("role");
    }
    if let Some(obj) = migrated_normalized.as_object_mut() {
        obj.remove("schema_version");
        obj.remove("node_id");
        obj.remove("role");
    }''')
replace_once(path,
'''        assert!(patched.starts_with("schema_version = 1\\n"));
        assert!(patched.ends_with(&original));''',
'''        assert!(patched.starts_with(&format!("schema_version = {DEPLOYMENT_SCHEMA_VERSION}\\n")));
        assert!(patched.contains("node_id = \\\"vpn\\\""));
        assert!(patched.contains("role = \\\"exit\\\""));''')
replace_once(path,
'''        assert_eq!(migrated_cfg.schema_version, DEPLOYMENT_SCHEMA_VERSION);

        let original_cfg''',
'''        assert_eq!(migrated_cfg.schema_version, DEPLOYMENT_SCHEMA_VERSION);
        assert_eq!(migrated_cfg.node_id, "vpn");
        assert_eq!(migrated_cfg.role, NodeRole::Exit);

        let original_cfg''')

# ---------------------------------------------------------------------------
# Credential alias resolution in the contract bridge.
# ---------------------------------------------------------------------------
path = "crates/compat-config/src/contract.rs"
replace_once(path,
'''        EndpointOrigin::Local => None,
        EndpointOrigin::Peer => match user.peer_credential(&endpoint.id) {
            None => return Ok(None),''',
'''        EndpointOrigin::Local => None,
        EndpointOrigin::Peer => {
            let credential_id = endpoint
                .credential_ref
                .as_deref()
                .unwrap_or(endpoint.id.as_str());
            match user.peer_credential(credential_id) {
            None => return Ok(None),''')
replace_once(path,
'''                Some(c)
            }
        },
    };''',
'''                Some(c)
            }
        }
        },
    };''')
# Provisioning: build all credential-bearing endpoints, hide first-hop infra
# from the public selector/catalog, but render it into Core and apply detour.
old = '''    let mut capabilities: Vec<contract::Capability> = Vec::new();
    for ep in &contract_endpoints {
        let cap = contract::Capability::for_transport(&ep.transport());
        if !capabilities.contains(&cap) {
            capabilities.push(cap);
        }
    }
'''
new = '''    let infrastructure_ids: std::collections::BTreeSet<String> = access_paths
        .iter()
        .filter(|path| matches!(path.kind, contract::AccessPathKind::Relay))
        .filter_map(|path| path.via_endpoint_id.clone())
        .collect();
    let selectable_endpoints: Vec<contract::Endpoint> = contract_endpoints
        .iter()
        .filter(|endpoint| !infrastructure_ids.contains(&endpoint.id))
        .cloned()
        .collect();

    let mut capabilities: Vec<contract::Capability> = Vec::new();
    for ep in &selectable_endpoints {
        let cap = contract::Capability::for_transport(&ep.transport());
        if !capabilities.contains(&cap) {
            capabilities.push(cap);
        }
    }
'''
replace_once(path, old, new)
replace_once(path,
'''    let singbox_config = crate::render::render_singbox_config_from_contract(
        &contract_endpoints,
        crate::render::SelectionProfile::default(),''',
'''    let selectable_ids: Vec<String> = selectable_endpoints
        .iter()
        .map(|endpoint| endpoint.id.clone())
        .collect();
    let singbox_config = crate::render::render_singbox_config_from_contract_with_access_paths(
        &contract_endpoints,
        &selectable_ids,
        access_paths,
        crate::render::SelectionProfile::default(),''')
replace_once(path,
'''        capabilities,
        contract_endpoints,
    )''',
'''        capabilities,
        selectable_endpoints,
    )''')

# ---------------------------------------------------------------------------
# Core renderer: actual native sing-box detour. Infrastructure outbounds are
# present in Core but deliberately absent from the selector/catalog.
# ---------------------------------------------------------------------------
path = "crates/compat-config/src/render.rs"
start = read(path).index("pub fn render_singbox_config_from_contract(")
end = read(path).index("\n/// The single `route.rules` entry", start)
old_block = read(path)[start:end]
new_block = r'''pub fn render_singbox_config_from_contract(
    contract_endpoints: &[contract::Endpoint],
    profile: SelectionProfile,
    compat_mode: CompatibilityMode,
) -> Result<serde_json::Value, CompatError> {
    let selectable_ids: Vec<String> = contract_endpoints
        .iter()
        .map(|endpoint| endpoint.id.clone())
        .collect();
    render_singbox_config_from_contract_with_access_paths(
        contract_endpoints,
        &selectable_ids,
        &[],
        profile,
        compat_mode,
    )
}

/// Access-path-aware Core renderer. `all_endpoints` includes both the
/// client-selectable exits and any authenticated first-hop endpoints needed
/// only as dialers. `selectable_endpoint_ids` is the exact catalog/selector
/// surface. Relay paths use sing-box's native outbound `detour`; a malformed
/// or metadata-only path is an error rather than silently becoming direct.
pub fn render_singbox_config_from_contract_with_access_paths(
    all_endpoints: &[contract::Endpoint],
    selectable_endpoint_ids: &[String],
    access_paths: &[contract::AccessPath],
    profile: SelectionProfile,
    compat_mode: CompatibilityMode,
) -> Result<serde_json::Value, CompatError> {
    use std::collections::{BTreeMap, BTreeSet};

    let selectable: BTreeSet<&str> = selectable_endpoint_ids.iter().map(String::as_str).collect();
    let endpoint_by_id: BTreeMap<&str, &contract::Endpoint> = all_endpoints
        .iter()
        .map(|endpoint| (endpoint.id.as_str(), endpoint))
        .collect();
    let path_by_id: BTreeMap<&str, &contract::AccessPath> = access_paths
        .iter()
        .map(|path| (path.id.as_str(), path))
        .collect();

    for id in &selectable {
        if !endpoint_by_id.contains_key(id) {
            return Err(CompatError::Parse(format!(
                "selectable endpoint id {id:?} has no credential-bearing endpoint"
            )));
        }
    }

    let mut infrastructure_ids: BTreeSet<&str> = BTreeSet::new();
    for path in access_paths {
        if matches!(path.kind, contract::AccessPathKind::Relay) {
            if let Some(via) = path.via_endpoint_id.as_deref() {
                infrastructure_ids.insert(via);
            }
        }
    }

    let mut outbounds = Vec::new();
    let mut tags = Vec::new();
    let mut reality_tag: Option<String> = None;
    let mut hysteria2_tag: Option<String> = None;

    for ep in all_endpoints {
        let is_selectable = selectable.contains(ep.id.as_str());
        let is_infrastructure = infrastructure_ids.contains(ep.id.as_str());
        if !is_selectable && !is_infrastructure {
            continue;
        }

        let tag = ep.tag.clone();
        if is_selectable {
            if tags.contains(&tag) {
                return Err(CompatError::Parse(format!(
                    "duplicate selectable outbound tag {tag:?}; endpoint labels used as Core tags must be unique"
                )));
            }
            tags.push(tag.clone());
            match &ep.params {
                contract::TransportParams::VlessReality { .. } if reality_tag.is_none() => {
                    reality_tag = Some(tag.clone());
                }
                contract::TransportParams::Hysteria2 { .. } if hysteria2_tag.is_none() => {
                    hysteria2_tag = Some(tag.clone());
                }
                _ => {}
            }
        }

        let mut outbound = match &ep.params {
            contract::TransportParams::VlessReality {
                uuid,
                flow,
                reality,
            } => {
                let mut ob = json!({
                    "type": "vless",
                    "tag": tag,
                    "server": ep.host,
                    "server_port": ep.port,
                    "uuid": uuid,
                    "tls": {
                        "enabled": true,
                        "server_name": ep.server_name,
                        "utls": { "enabled": true, "fingerprint": reality.fingerprint },
                        "reality": {
                            "enabled": true,
                            "public_key": reality.public_key,
                            "short_id": reality.short_id,
                        }
                    }
                });
                if let Some(flow) = flow {
                    ob["flow"] = json!(flow);
                }
                if compat_mode == CompatibilityMode::TcpOnly {
                    ob["network"] = json!("tcp");
                }
                ob
            }
            contract::TransportParams::Hysteria2 { password, obfs } => {
                let mut ob = json!({
                    "type": "hysteria2",
                    "tag": tag,
                    "server": ep.host,
                    "server_port": ep.port,
                    "password": password,
                    "tls": {
                        "enabled": true,
                        "server_name": ep.server_name,
                        "insecure": false,
                    }
                });
                if let Some(obfs) = obfs {
                    ob["obfs"] = json!({ "type": obfs.obfs_type, "password": obfs.password });
                }
                ob
            }
        };

        if is_selectable && !access_paths.is_empty() {
            if let Some(contract::PathType::Other(path_id)) = &ep.path {
                let path = path_by_id.get(path_id.as_str()).ok_or_else(|| {
                    CompatError::Parse(format!(
                        "endpoint {:?} references unknown access path {:?}",
                        ep.id, path_id
                    ))
                })?;
                if !matches!(path.kind, contract::AccessPathKind::Relay) {
                    return Err(CompatError::Parse(format!(
                        "endpoint {:?} references access path {:?} of kind {:?}; only relay paths have executable chaining semantics",
                        ep.id,
                        path_id,
                        path.kind.as_str()
                    )));
                }
                let via_id = path.via_endpoint_id.as_deref().ok_or_else(|| {
                    CompatError::Parse(format!(
                        "endpoint {:?} references relay path {:?} without via_endpoint_id; refusing to silently dial direct",
                        ep.id, path_id
                    ))
                })?;
                let via = endpoint_by_id.get(via_id).ok_or_else(|| {
                    CompatError::Parse(format!(
                        "relay path {:?} references unavailable first-hop endpoint {:?} for this user",
                        path_id, via_id
                    ))
                })?;
                if via.id == ep.id {
                    return Err(CompatError::Parse(format!(
                        "endpoint {:?} cannot detour through itself",
                        ep.id
                    )));
                }
                if !matches!(ep.params, contract::TransportParams::VlessReality { .. })
                    || !matches!(via.params, contract::TransportParams::VlessReality { .. })
                {
                    return Err(CompatError::Parse(format!(
                        "relay path {:?} is TCP/VLESS+REALITY-only in the current MVP; both first hop and exit must be vless-reality",
                        path_id
                    )));
                }
                outbound["network"] = json!("tcp");
                outbound["detour"] = json!(via.tag);
            }
        }
        outbounds.push(outbound);
    }

    if tags.is_empty() {
        return Err(CompatError::Parse(
            "no selectable endpoints remain after reserving relay first-hop infrastructure; pair/configure an exit before provisioning users"
                .to_string(),
        ));
    }

    outbounds.push(json!({
        "type": "urltest",
        "tag": "auto",
        "outbounds": tags.clone(),
        "url": "https://www.gstatic.com/generate_204",
        "interval": "1m",
    }));

    let mut selector_options = tags.clone();
    selector_options.push("auto".to_string());
    let default_tag = match profile {
        SelectionProfile::Reliability => reality_tag
            .or_else(|| tags.first().cloned())
            .unwrap_or_else(|| "auto".to_string()),
        SelectionProfile::Performance => hysteria2_tag
            .or(reality_tag)
            .or_else(|| tags.first().cloned())
            .unwrap_or_else(|| "auto".to_string()),
        SelectionProfile::Auto => "auto".to_string(),
    };
    outbounds.push(json!({
        "type": "selector",
        "tag": "select",
        "outbounds": selector_options,
        "default": default_tag,
    }));
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));

    let mut route = json!({ "final": "select" });
    if compat_mode == CompatibilityMode::QuicReject {
        route["rules"] = json!([quic_reject_rule()]);
    }

    Ok(json!({
        "outbounds": outbounds,
        "route": route
    }))
}
'''
text = read(path)
write(path, text[:start] + new_block + text[end:])

# ---------------------------------------------------------------------------
# Subscription server: feed access-path-aware rendering to legacy sing-box
# subscriptions too, and don't advertise Hysteria2 as a local exit on a
# node explicitly configured as relay.
# ---------------------------------------------------------------------------
path = "services/subscription/src/main.rs"
replace_once(path,
'''use compat_config::deployment::DeploymentConfig;''',
'''use compat_config::deployment::{DeploymentConfig, NodeRole};''')
replace_once(path,
'''    let mut endpoints = standard_endpoints(
        &cfg.public_host,''',
'''    let mut endpoints = standard_endpoints(
        &cfg.public_host,''')
replace_once(path,
'''        hysteria_obfs_password.as_deref(),
    );

    // Operator-declared endpoints''',
'''        hysteria_obfs_password.as_deref(),
    );
    if cfg.role == NodeRole::Relay {
        // A relay node's local VLESS listener is first-hop infrastructure,
        // not an Internet-exit choice. Hysteria2-over-relay is deliberately
        // not claimed by the TCP-first MVP.
        endpoints.retain(|endpoint| endpoint.id == "reality-1");
    }

    // Operator-declared endpoints''')

path = "services/subscription/src/lib.rs"
replace_once(path,
'''        "singbox" => match render::render_singbox_client_subscription_with_options(
            &user,
            &state.endpoints,
            profile,
            compat_mode,
        ) {''',
'''        "singbox" => {
            let mode = match compat_mode {
                render::CompatibilityMode::TcpOnly => compat_config::contract::DiagnosticMode::TcpOnly,
                render::CompatibilityMode::VisionOff => compat_config::contract::DiagnosticMode::VisionOff,
                _ => compat_config::contract::DiagnosticMode::None,
            };
            match compat_config::contract::provisioning_document_with_mode_and_access_paths(
                &user,
                &state.endpoints,
                mode,
                &state.access_paths,
            ) {
                Ok(doc) => match doc.singbox_config {
                    Some(config) => match serde_json::to_string_pretty(&config) {
                        Ok(body) => (StatusCode::OK, [("content-type", "application/json")], body).into_response(),
                        Err(e) => {
                            tracing::error!(error = %e, "failed to serialize singbox subscription");
                            (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
                        }
                    },
                    None => (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response(),
                },
                Err(e) => {
                    tracing::error!(error = %e, "failed to render singbox subscription");
                    (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
                }
            }
        }''')
# remove the now-left old match arms trailer from the exact replaced block.
replace_once(path,
'''            Ok(doc) => match serde_json::to_string_pretty(&doc) {
                Ok(body) => {
                    (StatusCode::OK, [("content-type", "application/json")], body).into_response()
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to serialize singbox subscription");
                    (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
                }
            },
            Err(e) => {
                tracing::error!(error = %e, "failed to render singbox subscription");
                (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
            }
        },
        "uri" | "hiddify" => {''',
'''        ,
        "uri" | "hiddify" => {''')

# URI/share-link syntax has no detour. Never misrepresent a relayed route as
# direct; the first-party/Core config remains the supported relay surface.
path = "crates/compat-config/src/render.rs"
replace_once(path,
'''    for ep in endpoints {
        let endpoint = match crate::contract::contract_endpoint_opt(''',
'''    for ep in endpoints {
        if ep.path.as_deref().is_some_and(|path| path != "direct") {
            // Share-link syntax cannot express sing-box detour. Omitting the
            // route is safer than silently handing a client a direct exit.
            continue;
        }
        let endpoint = match crate::contract::contract_endpoint_opt(''')

# ---------------------------------------------------------------------------
# A deterministic relay rendering integration test (no live network claim).
# ---------------------------------------------------------------------------
test_path = ROOT / "crates/compat-config/tests/relay_detour.rs"
test_path.write_text(r'''use compat_config::contract::provisioning_document_with_mode_and_access_paths;
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, PeerCredential};
use compat_config::render::standard_endpoints;
use compat_config::secret::SecretString;
use provisioning_contract::{AccessPath, AccessPathKind};

const DEPLOYMENT: &str = r#"
schema_version = 2
node_id = "ru1"
role = "relay"
public_host = "ru1.example.com"
subscription_host = "ru1.example.com"

[reality]
listen_port = 443
handshake_server = "www.cloudflare.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100

[[access_paths]]
id = "relay-ru1"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]

[[peer_endpoints]]
id = "de1-direct"
tag = "Germany · Direct"
host = "de1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "direct"

[[peer_endpoints]]
id = "de1-via-ru1"
tag = "Germany · via Russia"
host = "de1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "relay-ru1"
credential_ref = "de1-direct"
"#;

fn user() -> CompatUser {
    let mut peer_credentials = std::collections::BTreeMap::new();
    peer_credentials.insert(
        "de1-direct".to_string(),
        PeerCredential::VlessReality {
            uuid: "22222222-2222-4222-8222-222222222222".to_string(),
        },
    );
    CompatUser {
        id: "u1".into(),
        name: "test".into(),
        enabled: true,
        vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
        hysteria2_password: SecretString::new("hy2"),
        subscription_token_hash_hex: "hash".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        peer_credentials,
    }
}

#[test]
fn relay_path_is_real_detour_and_first_hop_is_not_selectable() {
    let cfg: DeploymentConfig = toml::from_str(DEPLOYMENT).unwrap();
    cfg.validate().unwrap();
    let mut endpoints = standard_endpoints(
        &cfg.public_host,
        443,
        443,
        "RURUFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake",
        "11223344",
        "www.cloudflare.com",
        None,
    );
    endpoints.retain(|endpoint| endpoint.id == "reality-1");
    endpoints.extend(
        cfg.peer_endpoints
            .iter()
            .map(|peer| peer.to_compat_endpoint().unwrap()),
    );
    let paths: Vec<AccessPath> = cfg
        .access_paths
        .iter()
        .map(|path| path.to_contract_access_path().unwrap())
        .collect();

    let doc = provisioning_document_with_mode_and_access_paths(
        &user(),
        &endpoints,
        compat_config::contract::DiagnosticMode::None,
        &paths,
    )
    .unwrap();

    let ids: Vec<_> = doc.endpoints.iter().map(|endpoint| endpoint.id.as_str()).collect();
    assert_eq!(ids, vec!["de1-direct", "de1-via-ru1"]);
    let config = doc.singbox_config.as_ref().unwrap();
    let outbounds = config["outbounds"].as_array().unwrap();
    let relay = outbounds
        .iter()
        .find(|outbound| outbound["server"] == "ru1.example.com")
        .expect("hidden RU first-hop outbound");
    let via = outbounds
        .iter()
        .find(|outbound| outbound["tag"] == "Germany · via Russia")
        .expect("via-RU exit outbound");
    assert_eq!(via["detour"], relay["tag"]);
    assert_eq!(via["network"], "tcp");
    assert_eq!(via["uuid"], "22222222-2222-4222-8222-222222222222");

    let selector = outbounds
        .iter()
        .find(|outbound| outbound["tag"] == "select")
        .unwrap();
    let options: Vec<_> = selector["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(options.contains(&"Germany · Direct"));
    assert!(options.contains(&"Germany · via Russia"));
    assert!(!options.contains(&relay["tag"].as_str().unwrap()));
}

#[test]
fn relay_path_without_via_endpoint_fails_closed() {
    let endpoints = standard_endpoints(
        "ru1.example.com",
        443,
        443,
        "RURUFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake",
        "11223344",
        "www.cloudflare.com",
        None,
    );
    let path = AccessPath::new("relay-broken", AccessPathKind::Relay, vec!["tcp".into()]);
    let mut exit = endpoints[0].clone();
    exit.id = "exit".into();
    exit.label = "Exit via broken relay".into();
    exit.path = Some("relay-broken".into());
    let err = provisioning_document_with_mode_and_access_paths(
        &user(),
        &[endpoints[0].clone(), exit],
        compat_config::contract::DiagnosticMode::None,
        &[path],
    )
    .unwrap_err();
    assert!(err.to_string().contains("without via_endpoint_id"));
}
''')

print("phase1 relay core edits applied")
