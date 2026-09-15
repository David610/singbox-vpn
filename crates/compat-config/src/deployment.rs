//! `/etc/vpn/deployment.toml` schema — shared by `vpn-admin` and
//! `services/subscription` so the domain/port/path configuration used to
//! render subscriptions and sing-box config lives in exactly one place,
//! never hardcoded into source (spec §36).

use crate::CompatError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current on-disk schema version for `deployment.toml`. Every
/// deployment.toml written before this field existed has no
/// `schema_version` key at all, which `#[serde(default)]` reads as `0`
/// ("legacy" — the only shape that ever existed, still fully loadable:
/// nothing else about the shape has changed yet). A value greater than
/// this constant means a NEWER vpn-admin wrote this file — an older
/// binary cannot safely assume it still understands every field's
/// meaning, so `DeploymentConfig::load` refuses it outright (see
/// `validate`) rather than silently reinterpreting it.
pub const DEPLOYMENT_SCHEMA_VERSION: u32 = 2;

/// Declared by a `deployment.toml` that enables `[amneziawg]`, and only by
/// one. A binary that predates AmneziaWG refuses the file (it is newer
/// than it supports) instead of rendering a node that silently stops
/// serving every AWG peer. Files without `[amneziawg]` stay at
/// [`DEPLOYMENT_SCHEMA_VERSION`] and byte-identical.
pub const DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG: u32 = 3;

/// Highest `deployment.toml` schema this binary can load.
pub const DEPLOYMENT_MAX_SCHEMA_VERSION: u32 = DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG;

/// Printed by `vpn-admin config validate` from every build that can render
/// and apply AmneziaWG. `update.sh` refuses to switch an AWG-enabled node
/// to a `vpn-admin` that does not print it.
pub const AMNEZIAWG_CAPABILITY: &str = "capability: amneziawg-3";

/// Endpoint id of this deployment's own AmneziaWG listener.
pub const LOCAL_AMNEZIAWG_ENDPOINT_ID: &str = "amneziawg-1";

/// Printed by `vpn-admin config validate` from every build that renders
/// relays fail-closed. `update.sh` refuses to switch a relay node to a
/// `vpn-admin` that does not print it: a build that parses `role` but
/// predates relay enforcement would otherwise render the relay as an
/// unrestricted exit after an update or rollback. Kept in sync with
/// `deploy/lib/node-identity.sh` by `deploy/lib/tests/test-node-identity.sh`.
pub const RELAY_ENFORCEMENT_CAPABILITY: &str = "capability: relay-fail-closed-forwarding";

/// Endpoint id of this deployment's own VLESS+REALITY listener. On a relay
/// node this is the authenticated first hop, never a final exit.
pub const LOCAL_REALITY_ENDPOINT_ID: &str = "reality-1";
/// Endpoint id of this deployment's own Hysteria2 listener.
pub const LOCAL_HYSTERIA2_ENDPOINT_ID: &str = "hysteria2-1";

/// `[[access_paths]] kind` value whose routes are executable through a
/// Core-native `detour` first hop.
pub const RELAY_ACCESS_PATH_KIND: &str = "relay";

/// Longest accepted `node_id`: one DNS label, so an operator can reuse it
/// as a hostname label or metrics dimension without re-validating it.
pub const NODE_ID_MAX_LEN: usize = 63;

/// Core outbound tags the client renderer emits itself. A peer endpoint's
/// `tag` becomes a Core outbound tag, so it must never collide with one.
pub const RESERVED_ENDPOINT_TAGS: &[&str] = &["Reality", "Hysteria2", "auto", "select", "direct"];

/// What a node is allowed to do with authenticated client traffic.
///
/// `Exit` is the historical single-server behaviour and the default for
/// every file that predates roles. `Relay` accepts authenticated
/// first-hop traffic and may forward it ONLY to the exits this file
/// declares; it is never an Internet exit (see
/// `server::render_server_config_for_deployment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
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

    /// Strict parser for operator input (installer flags, CLI). Unknown
    /// values are refused, never mapped to the permissive default.
    pub fn parse(value: &str) -> Result<Self, CompatError> {
        match value {
            "exit" => Ok(NodeRole::Exit),
            "relay" => Ok(NodeRole::Relay),
            other => Err(CompatError::Parse(format!(
                "invalid node role {other:?} (expected \"exit\" or \"relay\")"
            ))),
        }
    }
}

/// A concrete destination a relay node is allowed to dial on behalf of an
/// authenticated first-hop user. Derived only from declared peer exits;
/// never from user input or traffic.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelayTarget {
    pub host: String,
    pub port: u16,
}

/// Validate a `node_id`: 1..=63 ASCII letters, digits, `-`, `_`, `.`,
/// starting with a letter or digit.
pub fn validate_node_id(node_id: &str) -> Result<(), CompatError> {
    if node_id.is_empty() {
        return Err(CompatError::Parse("node_id is empty".into()));
    }
    if node_id.len() > NODE_ID_MAX_LEN {
        return Err(CompatError::Parse(format!(
            "node_id {node_id:?} is longer than {NODE_ID_MAX_LEN} characters"
        )));
    }
    if !node_id
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return Err(CompatError::Parse(format!(
            "node_id {node_id:?} must start with an ASCII letter or digit"
        )));
    }
    if !node_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(CompatError::Parse(format!(
            "node_id {node_id:?} contains unsupported characters (allowed: ASCII letters, digits, '-', '_', '.')"
        )));
    }
    Ok(())
}

/// The documented default node identity for a host: the first DNS label of
/// `public_host`, with unsupported characters replaced by `-`, trimmed and
/// truncated to `NODE_ID_MAX_LEN`. `legacy-node` when nothing usable
/// remains. The installer implements the same rule for fresh installs and
/// `deploy/lib/tests/test-node-identity.sh` holds the two in agreement.
pub fn default_node_id_for_host(public_host: &str) -> String {
    let first = public_host.split('.').next().unwrap_or(public_host);
    let mapped: String = first
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut id: String = mapped
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(NODE_ID_MAX_LEN)
        .collect();
    while id.ends_with(['-', '_']) {
        id.pop();
    }
    if id.is_empty() {
        "legacy-node".to_string()
    } else {
        id
    }
}

/// A relay target host must be something a Core `domain`/`ip_cidr` route
/// rule can match exactly: an IP literal that names one concrete host, or
/// a lowercase DNS name.
fn validate_relay_target_host(host: &str) -> Result<(), String> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let unusable = ip.is_unspecified()
            || ip.is_multicast()
            || matches!(ip, std::net::IpAddr::V4(v4) if v4.is_broadcast());
        return if unusable {
            Err(format!(
                "{host:?} is not a single concrete host (unspecified/multicast/broadcast)"
            ))
        } else {
            Ok(())
        };
    }
    if host.len() > 253 || host.ends_with('.') {
        return Err(format!("{host:?} is not a valid DNS name"));
    }
    for label in host.split('.') {
        let valid = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            return Err(format!(
                "{host:?} is not a lowercase DNS name or IP literal; relay forwarding rules match \
                 the destination exactly, so it must be written exactly as clients dial it"
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// On-disk schema version. `0` (the default when the key is absent)
    /// means "legacy, pre-versioning" — `vpn-admin config migrate` stamps
    /// it to `DEPLOYMENT_SCHEMA_VERSION` explicitly without touching any
    /// other value. See `DEPLOYMENT_SCHEMA_VERSION`'s doc comment.
    #[serde(default)]
    pub schema_version: u32,

    /// Stable operator-facing node identity. Schema-v2 files always carry
    /// this explicitly; legacy files default empty until migrated.
    #[serde(default)]
    pub node_id: String,
    /// Operational role. Legacy deployments were ordinary exits, so the
    /// serde default deliberately preserves that behavior.
    #[serde(default)]
    pub role: NodeRole,

    /// Public hostname/IP clients connect the VLESS+REALITY and
    /// Hysteria2 listeners to.
    pub public_host: String,
    /// Hostname the subscription HTTPS endpoint is served on (may equal
    /// `public_host`; kept separate because the reverse proxy terminating
    /// TLS for the subscription API may live on a different name/port).
    pub subscription_host: String,

    pub reality: RealitySection,
    pub hysteria2: Hysteria2Section,
    pub subscription: SubscriptionSection,

    /// UDP probe / diagnostic tuning for `vpn-admin doctor` (probe
    /// resolvers, timeouts, retries). Optional; sensible defaults are
    /// supplied if omitted.
    #[serde(default)]
    pub udp_probe: Option<UdpProbeSection>,

    /// Root of the `/etc/vpn/compat` state tree. Defaults applied by
    /// `default_state_dir` if omitted from the TOML file.
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,

    #[serde(default = "default_singbox_binary")]
    pub singbox_binary: PathBuf,

    /// Optional non-secret first-hop metadata advertised to first-party
    /// clients. This is declarative only: it neither deploys nor contacts a
    /// relay and it deliberately has no credential-bearing fields.
    #[serde(default)]
    pub access_paths: Vec<AccessPathSection>,

    /// Endpoints on OTHER, independently-operated servers that this
    /// deployment should also advertise (ADR-0009).
    ///
    /// This is not orchestration: this server never contacts, controls,
    /// deploys, health-checks, or mints credentials for a peer. It only
    /// repeats what the operator declared, to users the operator has
    /// given a peer credential.
    ///
    /// Optional and absent from every existing file, so
    /// `DEPLOYMENT_SCHEMA_VERSION` does not move.
    #[serde(default)]
    pub peer_endpoints: Vec<PeerEndpointSection>,

    /// Optional AmneziaWG listener on this node. Its presence requires
    /// `schema_version = 3` (see [`DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG`]).
    /// Keys and obfuscation parameters are generated state under
    /// `state_dir/amneziawg`, never part of this hand-edited file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amneziawg: Option<AmneziaWgSection>,
}

/// `[amneziawg]` — listener and tunnel addressing for the AmneziaWG
/// transport. Unknown keys are refused, like `[[peer_endpoints]]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmneziaWgSection {
    pub listen_port: u16,
    #[serde(default = "default_awg_interface")]
    pub interface: String,
    #[serde(default = "default_awg_subnet_v4")]
    pub subnet_v4: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subnet_v6: Option<String>,
    #[serde(default = "default_awg_mtu")]
    pub mtu: u16,
    #[serde(default = "default_awg_keepalive")]
    pub persistent_keepalive: u16,
    /// Only for fallback `awg-quick` profiles; see
    /// `amneziawg::AwgNodeConfig::fallback_client_dns`.
    #[serde(default = "default_awg_fallback_dns")]
    pub fallback_client_dns: Vec<String>,
}

fn default_awg_interface() -> String {
    "awg0".into()
}
fn default_awg_subnet_v4() -> String {
    "10.66.0.0/16".into()
}
fn default_awg_mtu() -> u16 {
    1380
}
fn default_awg_keepalive() -> u16 {
    25
}
fn default_awg_fallback_dns() -> Vec<String> {
    vec!["1.1.1.1".into(), "2606:4700:4700::1111".into()]
}

impl AmneziaWgSection {
    fn validate(&self, cfg: &DeploymentConfig) -> Result<(), CompatError> {
        let bad = |m: String| Err(CompatError::Parse(format!("[amneziawg] {m}")));
        if self.listen_port == 0 {
            return bad("listen_port must be non-zero".into());
        }
        if self.listen_port == cfg.hysteria2.listen_port {
            return bad(format!(
                "listen_port {} collides with the Hysteria2 UDP listener",
                self.listen_port
            ));
        }
        if self.interface.is_empty()
            || self.interface.len() > 15
            || !self
                .interface
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return bad("interface must be 1-15 characters of [A-Za-z0-9_.-]".into());
        }
        self.subnet_v4
            .parse::<crate::amneziawg::Ipv4Net>()
            .map_err(|e| CompatError::Parse(format!("[amneziawg] subnet_v4: {e}")))?;
        if let Some(v6) = &self.subnet_v6 {
            v6.parse::<crate::amneziawg::Ipv6Net>()
                .map_err(|e| CompatError::Parse(format!("[amneziawg] subnet_v6: {e}")))?;
        }
        if !(1280..=1500).contains(&self.mtu) {
            return bad("mtu must be 1280..=1500".into());
        }
        for dns in &self.fallback_client_dns {
            if dns.parse::<std::net::IpAddr>().is_err() {
                return bad(format!("fallback_client_dns entry {dns:?} is not an IP literal"));
            }
        }
        if cfg.role == NodeRole::Relay {
            return bad(
                "a relay node never exits to the Internet, and AmneziaWG is not detour-capable; \
                 enable it on exit nodes only"
                    .into(),
            );
        }
        Ok(())
    }
}

/// One `[[access_paths]]` entry. Metadata only — no credentials or raw
/// proxy configuration are accepted here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessPathSection {
    pub id: String,
    pub kind: String,
    /// Non-secret endpoint-id reference for a real first hop. A relay path
    /// referenced by an endpoint must set this; credentials remain in the
    /// per-user endpoint model, never in this metadata block.
    #[serde(default)]
    pub via_endpoint_id: Option<String>,
    #[serde(default)]
    pub failure_domain: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

impl AccessPathSection {
    fn validate(&self) -> Result<(), CompatError> {
        if let Some(key) = self.extra.keys().find(|key| {
            let key = key.to_ascii_lowercase();
            [
                "private",
                "secret",
                "password",
                "token",
                "credential",
                "key",
            ]
            .iter()
            .any(|needle| key.contains(needle))
        }) {
            return Err(CompatError::Parse(format!(
                "[[access_paths]] {}: key {key:?} looks credential-bearing. Access-path metadata must never contain relay secrets or proxy configuration.",
                self.id
            )));
        }
        if let Some(key) = self.extra.keys().next() {
            return Err(CompatError::Parse(format!(
                "[[access_paths]] {}: unknown key {key:?}; unknown keys are refused rather than silently ignored",
                self.id
            )));
        }
        if self.id.trim().is_empty() {
            return Err(CompatError::Parse("[[access_paths]] id is empty".into()));
        }
        if self.kind.trim().is_empty() {
            return Err(CompatError::Parse(format!(
                "[[access_paths]] {}: kind is empty",
                self.id
            )));
        }
        if self
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
            ("failure_domain", self.failure_domain.as_deref()),
            ("region", self.region.as_deref()),
            ("provider", self.provider.as_deref()),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(CompatError::Parse(format!(
                    "[[access_paths]] {}: {name} is empty",
                    self.id
                )));
            }
        }
        if self.capabilities.is_empty() {
            return Err(CompatError::Parse(format!(
                "[[access_paths]] {}: capabilities is empty",
                self.id
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for capability in &self.capabilities {
            if capability.trim().is_empty() {
                return Err(CompatError::Parse(format!(
                    "[[access_paths]] {}: capability is empty",
                    self.id
                )));
            }
            if !seen.insert(capability.as_str()) {
                return Err(CompatError::Parse(format!(
                    "[[access_paths]] {}: duplicate capability {:?}",
                    self.id, capability
                )));
            }
        }
        Ok(())
    }

    pub fn to_contract_access_path(
        &self,
    ) -> Result<provisioning_contract::AccessPath, CompatError> {
        self.validate()?;
        let mut path = provisioning_contract::AccessPath::new(
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
        Ok(path)
    }
}

/// One `[[peer_endpoints]]` entry.
///
/// `extra` catches every key that is not a field here. Unknown keys are
/// rejected rather than ignored: a mistyped `reality_public_ky` that
/// silently defaulted to absent would produce an endpoint no client could
/// use, and a pasted `reality_private_key` that was silently dropped
/// would leave the operator believing this server needed it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerEndpointSection {
    /// Must not collide with a locally generated id or another peer's.
    pub id: String,
    /// Client-visible display name (also the Core outbound tag).
    pub tag: String,
    pub host: String,
    pub port: u16,
    pub transport: crate::model::CompatTransport,
    #[serde(default)]
    pub server_name: Option<String>,

    /// The peer's REALITY **public** key and short id. Public material:
    /// same treatment as this server's own, which is already published to
    /// every client. The peer's PRIVATE key is not a field here and is
    /// actively refused — see `PeerEndpointSection::validate`.
    #[serde(default)]
    pub reality_public_key: Option<String>,
    #[serde(default)]
    pub reality_short_id: Option<String>,
    #[serde(default = "default_peer_fingerprint")]
    pub reality_fingerprint: String,

    /// Salamander obfuscation password configured on the peer, if any.
    #[serde(default)]
    pub obfs_password: Option<String>,

    /// Shared-fate identifier. Required for a peer: the whole point of
    /// declaring one is that it fails independently of this server, and
    /// leaving the client to derive that from a hostname would be a
    /// guess about someone else's infrastructure.
    pub failure_domain: String,

    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub asn: Option<String>,
    #[serde(default = "default_peer_path")]
    pub path: String,
    /// Optional endpoint id whose per-user credential should be reused by
    /// this route alias (for example DE-direct and DE-via-RU).
    #[serde(default)]
    pub credential_ref: Option<String>,

    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

fn default_peer_fingerprint() -> String {
    "chrome".to_string()
}

fn default_peer_path() -> String {
    "direct".to_string()
}

impl PeerEndpointSection {
    fn validate(&self) -> Result<(), CompatError> {
        // Key-shaped names are reported first, whichever order the keys
        // happen to be in, so an operator who pasted a private key sees
        // *that* rather than an incidental complaint about some other
        // typo in the same block.
        if let Some(key) = self.extra.keys().find(|k| {
            let l = k.to_ascii_lowercase();
            l.contains("private") || l.contains("secret")
        }) {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: key {key:?} looks like private key material. This \
                 server has no use for a peer's private key and must never hold one — it \
                 never leaves the peer server. Remove the key from deployment.toml.",
                self.id
            )));
        }
        if let Some(key) = self.extra.keys().next() {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: unknown key {key:?}. Unknown keys are refused rather \
                 than ignored, because a silently-dropped key produces an endpoint no client \
                 can use.",
                self.id
            )));
        }

        if self.id.trim().is_empty() {
            return Err(CompatError::Parse("[[peer_endpoints]] id is empty".into()));
        }
        if self.tag.trim().is_empty() {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: tag is empty",
                self.id
            )));
        }
        if self.failure_domain.trim().is_empty() {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: failure_domain is empty. A peer exists to fail \
                 independently; say which domain it belongs to.",
                self.id
            )));
        }
        if self.path.trim().is_empty() {
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
        if self.host.trim().is_empty() || self.host.chars().any(|c| c.is_whitespace() || c == '/') {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: host {:?} is not dialable",
                self.id, self.host
            )));
        }
        if self.port == 0 {
            return Err(CompatError::Parse(format!(
                "[[peer_endpoints]] {}: port 0 is not dialable",
                self.id
            )));
        }

        match self.transport {
            crate::model::CompatTransport::VlessReality => {
                let pk = self.reality_public_key.as_deref().unwrap_or_default();
                crate::credentials::validate_reality_public_key_shape(pk).map_err(|e| {
                    CompatError::Parse(format!(
                        "[[peer_endpoints]] {}: reality_public_key: {e}",
                        self.id
                    ))
                })?;
                let sid = self.reality_short_id.as_deref().unwrap_or_default();
                if sid.is_empty()
                    || !sid.len().is_multiple_of(2)
                    || sid.len() > 16
                    || !sid.chars().all(|c| c.is_ascii_hexdigit())
                {
                    return Err(CompatError::Parse(format!(
                        "[[peer_endpoints]] {}: reality_short_id {sid:?} is not an even-length \
                         hex string of at most 16 characters",
                        self.id
                    )));
                }
            }
            crate::model::CompatTransport::Hysteria2 => {
                if self.reality_public_key.is_some() || self.reality_short_id.is_some() {
                    return Err(CompatError::Parse(format!(
                        "[[peer_endpoints]] {}: REALITY parameters do not apply to a hysteria2 \
                         endpoint",
                        self.id
                    )));
                }
            }
        }
        Ok(())
    }

    /// The runtime endpoint this declaration describes.
    ///
    /// Carries no credential: a peer endpoint's credential is per-user
    /// and lives in `CompatUser::peer_credentials`, resolved when a
    /// specific user's document is assembled.
    pub fn to_compat_endpoint(&self) -> Result<crate::model::CompatEndpoint, CompatError> {
        self.validate()?;
        let public_parameters = match self.transport {
            crate::model::CompatTransport::VlessReality => {
                crate::model::PublicParameters::Reality {
                    public_key_hex: self.reality_public_key.clone().unwrap_or_default(),
                    short_id: self.reality_short_id.clone().unwrap_or_default(),
                    fingerprint: self.reality_fingerprint.clone(),
                }
            }
            crate::model::CompatTransport::Hysteria2 => crate::model::PublicParameters::Hysteria2 {
                obfs_password: self.obfs_password.clone(),
            },
        };
        Ok(crate::model::CompatEndpoint {
            id: self.id.clone(),
            transport: self.transport,
            host: self.host.clone(),
            port: self.port,
            server_name: Some(
                self.server_name
                    .clone()
                    .unwrap_or_else(|| self.host.clone()),
            ),
            label: self.tag.clone(),
            public_parameters,
            origin: crate::model::EndpointOrigin::Peer,
            failure_domain: Some(self.failure_domain.clone()),
            region: self.region.clone(),
            provider: self.provider.clone(),
            asn: self.asn.clone(),
            path: Some(self.path.clone()),
            credential_ref: self.credential_ref.clone(),
        })
    }
}

/// Endpoint ids this server generates for its own listeners. A peer may
/// never reuse one: the two would collide in the assembled document, and
/// the client would have no way to tell which server it was dialling.
pub const LOCAL_ENDPOINT_IDS: &[&str] = &["reality-1", "hysteria2-1"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UdpProbeSection {
    /// IPv4 resolver IPs to try for UDP probes.
    #[serde(default = "default_ipv4_resolvers")]
    pub ipv4_resolvers: Vec<String>,
    /// IPv6 resolver IPs to try for UDP probes.
    #[serde(default = "default_ipv6_resolvers")]
    pub ipv6_resolvers: Vec<String>,
    /// Number of attempts per resolver candidate.
    #[serde(default = "default_udp_retries")]
    pub retries: usize,
    /// Per-attempt timeout in milliseconds.
    #[serde(default = "default_udp_timeout_ms")]
    pub timeout_ms: u64,
    /// Delay between attempts in milliseconds.
    #[serde(default = "default_udp_delay_ms")]
    pub delay_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealitySection {
    pub listen_port: u16,
    /// Real TLS site dialed for the REALITY handshake disguise (must be a
    /// TLS 1.3 site supporting the chosen fingerprint).
    pub handshake_server: String,
    #[serde(default = "default_handshake_port")]
    pub handshake_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hysteria2Section {
    pub listen_port: u16,
    /// Fixed-rate (Brutal) congestion-control bandwidth, Mbps. Leave
    /// unset (the default) unless the real sustained throughput of this
    /// VPS has actually been measured (`vpn benchmark` /
    /// docs/PERFORMANCE_OPTIMIZATION_PLAN.md) — an unmeasured/guessed
    /// value causes self-induced congestion, not a speedup. Both fields
    /// must be set together; setting only one is rejected by
    /// `DeploymentConfig::validate` (see `store.rs`/tests below).
    #[serde(default)]
    pub up_mbps: Option<u32>,
    #[serde(default)]
    pub down_mbps: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionSection {
    /// Loopback-only listen port for the subscription HTTP service; a
    /// reverse proxy terminates public HTTPS (default 8443) in front of
    /// it (spec §27).
    pub listen_port: u16,
    #[serde(default = "default_public_port")]
    pub public_port: u16,
}

fn default_state_dir() -> PathBuf {
    PathBuf::from("/etc/vpn/compat")
}

fn default_singbox_binary() -> PathBuf {
    PathBuf::from("/usr/local/bin/sing-box")
}

fn default_handshake_port() -> u16 {
    443
}

fn default_public_port() -> u16 {
    8443
}

fn default_ipv4_resolvers() -> Vec<String> {
    vec!["1.1.1.1".into(), "8.8.8.8".into()]
}

fn default_ipv6_resolvers() -> Vec<String> {
    vec!["2606:4700:4700::1111".into(), "2001:4860:4860::8888".into()]
}

fn default_udp_retries() -> usize {
    2
}

fn default_udp_timeout_ms() -> u64 {
    2000
}

fn default_udp_delay_ms() -> u64 {
    250
}

impl Default for UdpProbeSection {
    fn default() -> Self {
        UdpProbeSection {
            ipv4_resolvers: default_ipv4_resolvers(),
            ipv6_resolvers: default_ipv6_resolvers(),
            retries: default_udp_retries(),
            timeout_ms: default_udp_timeout_ms(),
            delay_ms: default_udp_delay_ms(),
        }
    }
}

impl DeploymentConfig {
    pub fn load(path: &Path) -> Result<Self, CompatError> {
        let text = std::fs::read_to_string(path).map_err(|e| CompatError::Io(e.to_string()))?;
        let cfg: Self = toml::from_str(&text).map_err(|e| CompatError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Structural checks that TOML deserialization alone can't express
    /// (field-value-shape defaults, not schema shape).
    ///
    /// Every relay-related rule here fails closed: a declaration this
    /// binary cannot turn into an exact, restricted forwarding policy and an
    /// exact Core `detour` chain is refused at load time, before anything
    /// is rendered, served or applied.
    pub fn validate(&self) -> Result<(), CompatError> {
        // Fail closed on a schema newer than this binary understands —
        // see DEPLOYMENT_SCHEMA_VERSION's doc comment.
        if self.schema_version > DEPLOYMENT_MAX_SCHEMA_VERSION {
            return Err(CompatError::UnsupportedSchema {
                what: "deployment.toml",
                found: self.schema_version,
                max_supported: DEPLOYMENT_MAX_SCHEMA_VERSION,
            });
        }
        if self.schema_version >= 2 && self.node_id.trim().is_empty() {
            return Err(CompatError::Parse(
                "schema-v2 deployment.toml requires a non-empty node_id".into(),
            ));
        }
        if !self.node_id.is_empty() {
            validate_node_id(&self.node_id)?;
        }

        let mut seen_access_path_ids: Vec<&str> = Vec::with_capacity(self.access_paths.len());
        for path in &self.access_paths {
            path.validate()?;
            if seen_access_path_ids.contains(&path.id.as_str()) {
                return Err(CompatError::Parse(format!(
                    "duplicate [[access_paths]] id {:?}",
                    path.id
                )));
            }
            seen_access_path_ids.push(&path.id);
        }

        let mut seen_peer_ids: Vec<&str> = Vec::with_capacity(self.peer_endpoints.len());
        let mut seen_tags: Vec<&str> = Vec::with_capacity(self.peer_endpoints.len());
        for peer in &self.peer_endpoints {
            peer.validate()?;
            if LOCAL_ENDPOINT_IDS.contains(&peer.id.as_str()) {
                return Err(CompatError::Parse(format!(
                    "[[peer_endpoints]] id {:?} collides with an endpoint id this server \
                     generates for its own listeners ({LOCAL_ENDPOINT_IDS:?}). Both would end \
                     up in one document and a client could not tell them apart.",
                    peer.id
                )));
            }
            if seen_peer_ids.contains(&peer.id.as_str()) {
                return Err(CompatError::Parse(format!(
                    "duplicate [[peer_endpoints]] id {:?}",
                    peer.id
                )));
            }
            seen_peer_ids.push(&peer.id);
            if RESERVED_ENDPOINT_TAGS.contains(&peer.tag.as_str())
                || seen_tags.contains(&peer.tag.as_str())
            {
                return Err(CompatError::Parse(format!(
                    "[[peer_endpoints]] {}: tag {:?} is reserved or already used by another \
                     endpoint; tags become Core outbound tags and must be unique",
                    peer.id, peer.tag
                )));
            }
            seen_tags.push(&peer.tag);

            // Old schema-version-1 files were allowed to carry opaque path
            // labels without an access_paths list. Preserve that. Once the
            // operator opts into access_paths, though, every non-direct peer
            // path must resolve to one of those declarations.
            if !self.access_paths.is_empty()
                && peer.path != "direct"
                && !self.access_paths.iter().any(|path| path.id == peer.path)
            {
                return Err(CompatError::Parse(format!(
                    "[[peer_endpoints]] {}: path {:?} has no matching [[access_paths]] declaration",
                    peer.id, peer.path
                )));
            }
        }

        self.validate_access_path_first_hops()?;
        self.validate_credential_refs()?;
        self.validate_relay_routes()?;
        self.validate_role()?;

        if self.hysteria2.up_mbps.is_some() != self.hysteria2.down_mbps.is_some() {
            return Err(CompatError::Parse(
                "[hysteria2] up_mbps and down_mbps must be set together (both, or neither) — \
                 see docs/PERFORMANCE_OPTIMIZATION_PLAN.md"
                    .to_string(),
            ));
        }

        match (&self.amneziawg, self.schema_version) {
            (Some(section), v) if v >= DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG => section.validate(self)?,
            (Some(_), v) => {
                return Err(CompatError::Parse(format!(
                    "[amneziawg] requires schema_version = {DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG} \
                     (found {v}); enable it with `vpn-admin transport enable amneziawg` so older \
                     binaries refuse this file instead of dropping AmneziaWG silently"
                )))
            }
            (None, v) if v >= DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG => {
                return Err(CompatError::Parse(format!(
                    "schema_version {v} declares AmneziaWG but no [amneziawg] section exists; \
                     use `vpn-admin transport disable amneziawg` to return to schema \
                     {DEPLOYMENT_SCHEMA_VERSION}"
                )))
            }
            (None, _) => {}
        }
        Ok(())
    }

    fn relay_path(&self, id: &str) -> Option<&AccessPathSection> {
        self.access_paths
            .iter()
            .find(|path| path.id == id && path.kind == RELAY_ACCESS_PATH_KIND)
    }

    /// Every declared `via_endpoint_id` must name a real, TCP-capable first
    /// hop, even on a path no route uses yet: a dangling or UDP first hop
    /// is a malformed pairing, not metadata.
    fn validate_access_path_first_hops(&self) -> Result<(), CompatError> {
        for path in &self.access_paths {
            let Some(via) = path.via_endpoint_id.as_deref() else {
                continue;
            };
            if path.kind != RELAY_ACCESS_PATH_KIND {
                return Err(CompatError::Parse(format!(
                    "[[access_paths]] {}: via_endpoint_id is only meaningful for kind = \"relay\" \
                     (found kind {:?})",
                    path.id, path.kind
                )));
            }
            if via == LOCAL_HYSTERIA2_ENDPOINT_ID {
                return Err(CompatError::Parse(format!(
                    "[[access_paths]] {}: via_endpoint_id {via:?} is the local Hysteria2 listener; \
                     relay chaining is TCP/VLESS+REALITY-only and UDP relay is not implemented",
                    path.id
                )));
            }
            if via == LOCAL_REALITY_ENDPOINT_ID {
                if self.role != NodeRole::Relay {
                    return Err(CompatError::Parse(format!(
                        "[[access_paths]] {}: via_endpoint_id {via:?} makes this node's own \
                         listener a relay first hop, which requires role = \"relay\" (this node \
                         is {:?}). An exit must never be advertised as relay infrastructure.",
                        path.id,
                        self.role.as_str()
                    )));
                }
            } else {
                let first_hop = self
                    .peer_endpoints
                    .iter()
                    .find(|candidate| candidate.id == via)
                    .ok_or_else(|| {
                        CompatError::Parse(format!(
                            "[[access_paths]] {}: via_endpoint_id {via:?} is neither local {LOCAL_REALITY_ENDPOINT_ID} nor a declared peer endpoint",
                            path.id
                        ))
                    })?;
                if first_hop.transport != crate::model::CompatTransport::VlessReality {
                    return Err(CompatError::Parse(format!(
                        "[[access_paths]] {}: via_endpoint_id {via:?} is not VLESS+REALITY; relay chaining is TCP/VLESS-only in this MVP",
                        path.id
                    )));
                }
                if first_hop.path != "direct" || first_hop.credential_ref.is_some() {
                    return Err(CompatError::Parse(format!(
                        "[[access_paths]] {}: first hop {via:?} must itself be a direct endpoint \
                         with its own credential; chains longer than two hops are not supported",
                        path.id
                    )));
                }
            }
            if let Some(capability) = path
                .capabilities
                .iter()
                .find(|capability| capability.as_str() != "tcp")
            {
                return Err(CompatError::Parse(format!(
                    "[[access_paths]] {}: capability {capability:?} is not implemented for relay \
                     paths (only \"tcp\"); advertise only capabilities actually enforced",
                    path.id
                )));
            }
        }
        Ok(())
    }

    /// A route alias may reuse a credential only when it names the direct
    /// endpoint of the SAME exit server: the credential was issued by that
    /// server and authenticates nowhere else.
    fn validate_credential_refs(&self) -> Result<(), CompatError> {
        for peer in &self.peer_endpoints {
            let Some(credential_ref) = &peer.credential_ref else {
                continue;
            };
            if *credential_ref == peer.id {
                return Err(CompatError::Parse(format!(
                    "[[peer_endpoints]] {}: credential_ref points at itself",
                    peer.id
                )));
            }
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
            if target.credential_ref.is_some() || target.path != "direct" {
                return Err(CompatError::Parse(format!(
                    "[[peer_endpoints]] {}: credential_ref {:?} must name a direct endpoint that \
                     owns its credential, not another alias",
                    peer.id, credential_ref
                )));
            }
            if target.host != peer.host
                || target.port != peer.port
                || target.reality_public_key != peer.reality_public_key
            {
                return Err(CompatError::Parse(format!(
                    "[[peer_endpoints]] {}: credential_ref {:?} names a different server \
                     (host/port/REALITY public key differ); a credential issued by one exit \
                     cannot authenticate to another",
                    peer.id, credential_ref
                )));
            }
        }
        Ok(())
    }

    /// Routes that use a relay path: VLESS+REALITY only, never through
    /// themselves, and — when this node is the first hop — to a target a
    /// forwarding rule can match exactly.
    fn validate_relay_routes(&self) -> Result<(), CompatError> {
        for peer in &self.peer_endpoints {
            if peer.path == "direct" {
                continue;
            }
            let Some(path) = self.relay_path(&peer.path) else {
                continue;
            };
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
            if via == LOCAL_REALITY_ENDPOINT_ID {
                validate_relay_target_host(&peer.host).map_err(|reason| {
                    CompatError::Parse(format!(
                        "[[peer_endpoints]] {}: invalid relay destination: {reason}",
                        peer.id
                    ))
                })?;
            }
        }
        Ok(())
    }

    /// A relay must declare its own listener as first-hop infrastructure.
    /// Without that declaration the subscription service would advertise
    /// `reality-1` as an ordinary exit that the server then refuses to
    /// forward — a confidently broken route instead of an explicit one.
    fn validate_role(&self) -> Result<(), CompatError> {
        if self.role == NodeRole::Relay
            && !self.access_paths.iter().any(|path| {
                path.kind == RELAY_ACCESS_PATH_KIND
                    && path.via_endpoint_id.as_deref() == Some(LOCAL_REALITY_ENDPOINT_ID)
            })
        {
            return Err(CompatError::Parse(format!(
                "role = \"relay\" requires an [[access_paths]] entry with kind = \"relay\" and \
                 via_endpoint_id = \"{LOCAL_REALITY_ENDPOINT_ID}\". It marks this node's own \
                 listener as first-hop infrastructure so it is never offered as a final exit."
            )));
        }
        Ok(())
    }

    /// Concrete destinations this node may dial when it is a relay: the
    /// host/port of every declared peer route whose relay path uses this
    /// node's own `reality-1` as its first hop. Sorted and de-duplicated.
    ///
    /// Empty for an exit, and empty for a relay that is not paired with any
    /// exit yet — the role-aware server renderer treats that as reject-all.
    pub fn relay_targets(&self) -> Vec<RelayTarget> {
        if self.role != NodeRole::Relay {
            return Vec::new();
        }
        let mut targets: Vec<RelayTarget> = self
            .peer_endpoints
            .iter()
            .filter(|peer| {
                self.relay_path(&peer.path).is_some_and(|path| {
                    path.via_endpoint_id.as_deref() == Some(LOCAL_REALITY_ENDPOINT_ID)
                })
            })
            .map(|peer| RelayTarget {
                host: peer.host.clone(),
                port: peer.port,
            })
            .collect();
        targets.sort();
        targets.dedup();
        targets
    }

    /// The exact endpoint set `vpn-subscription` serves and `vpn-admin`
    /// reproduces: this node's own listeners, then declared peers in
    /// declaration order.
    ///
    /// A relay's Hysteria2 listener is not offered: relay chaining is
    /// TCP-only, and a relay's listeners never reach the Internet. Its
    /// `reality-1` stays in the set because it is the first hop the relay
    /// routes detour through; the access-path declaration `validate_role`
    /// requires is what keeps it out of every selectable catalog.
    pub fn served_endpoints(
        &self,
        reality_public_key_hex: &str,
        reality_short_id: &str,
        hysteria_obfs_password: Option<&str>,
    ) -> Result<Vec<crate::model::CompatEndpoint>, CompatError> {
        let mut endpoints = crate::render::standard_endpoints(
            &self.public_host,
            self.reality.listen_port,
            self.hysteria2.listen_port,
            reality_public_key_hex,
            reality_short_id,
            &self.reality.handshake_server,
            hysteria_obfs_password,
        );
        if self.role == NodeRole::Relay {
            endpoints.retain(|endpoint| endpoint.id == LOCAL_REALITY_ENDPOINT_ID);
        }
        for peer in &self.peer_endpoints {
            endpoints.push(peer.to_compat_endpoint().map_err(|e| {
                CompatError::Parse(format!(
                    "invalid [[peer_endpoints]] entry {:?}: {e}",
                    peer.id
                ))
            })?);
        }
        Ok(endpoints)
    }

    /// Contract form of every declared access path, validated.
    pub fn contract_access_paths(
        &self,
    ) -> Result<Vec<provisioning_contract::AccessPath>, CompatError> {
        self.access_paths
            .iter()
            .map(AccessPathSection::to_contract_access_path)
            .collect()
    }

    /// Return the effective UDP probe configuration, falling back to
    /// defaults if the section is omitted from the TOML.
    pub fn udp_probe_config(&self) -> UdpProbeSection {
        self.udp_probe.clone().unwrap_or_default()
    }

    pub fn users_file(&self) -> PathBuf {
        self.state_dir.join("users/users.json")
    }

    pub fn reality_dir(&self) -> PathBuf {
        self.state_dir.join("reality")
    }

    pub fn reality_private_key_file(&self) -> PathBuf {
        self.reality_dir().join("private.key")
    }

    pub fn reality_public_key_file(&self) -> PathBuf {
        self.reality_dir().join("public.key")
    }

    pub fn hysteria_dir(&self) -> PathBuf {
        self.state_dir.join("hysteria")
    }

    /// Deployment-wide Hysteria2 salamander obfuscation password. Shared
    /// with clients via subscriptions (not a per-user secret, see
    /// `PublicParameters::Hysteria2`'s doc comment) and stored like the
    /// REALITY public key: present on disk once `vpn-admin init`/`hysteria-
    /// obfs-rotate` has run, absent (obfuscation disabled) otherwise so
    /// pre-existing deployments upgrading are never surprised by a client-
    /// breaking config change they didn't ask for.
    ///
    /// Deliberately lives under `reality_dir()`, not `hysteria_dir()`,
    /// despite the name: `hysteria_dir()` is `root:sing-box 0750` (sing-box
    /// only — see `deploy/almalinux/install.sh`'s ownership matrix), which
    /// `vpn-subscription` cannot even traverse. `reality_dir()` is the
    /// shared `root:vpn-compat 0750` directory both services already
    /// traverse — the file itself is still owned `root:vpn-subscription
    /// 0640`, exactly like `reality/public.key`.
    pub fn hysteria_obfs_password_file(&self) -> PathBuf {
        self.reality_dir().join("hysteria_obfs_password.txt")
    }

    pub fn singbox_config_file(&self) -> PathBuf {
        self.state_dir.join("sing-box/config.json")
    }

    /// `root:vpn-compat 0750`: shared by vpn-admin and vpn-subscription.
    pub fn amneziawg_dir(&self) -> PathBuf {
        self.state_dir.join("amneziawg")
    }

    /// `root:root 0600`: vpn-subscription can never read it.
    pub fn amneziawg_server_private_key_file(&self) -> PathBuf {
        self.amneziawg_dir().join("server.key")
    }

    pub fn amneziawg_server_public_key_file(&self) -> PathBuf {
        self.amneziawg_dir().join("server.pub")
    }

    /// Obfuscation parameters, including the header protection key shared
    /// with every client (same class as the Salamander password).
    pub fn amneziawg_params_file(&self) -> PathBuf {
        self.amneziawg_dir().join("params.json")
    }

    /// Rendered `awg setconf` file; contains the server private key.
    pub fn amneziawg_setconf_file(&self) -> PathBuf {
        let iface = self
            .amneziawg
            .as_ref()
            .map(|s| s.interface.as_str())
            .unwrap_or("awg0");
        self.amneziawg_dir().join(format!("{iface}.conf"))
    }
}

/// Outcome of `migrate_deployment_toml`, reported by `vpn-admin config
/// migrate` (see requirement: reinstall/update must explicitly report
/// what it detected/did — never silent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentMigrationOutcome {
    /// `path` does not exist — nothing to migrate (fresh install).
    Missing,
    /// Already at `DEPLOYMENT_SCHEMA_VERSION` — no-op, safe to re-run.
    AlreadyCurrent,
    /// Migrated; the pre-migration file is at this path.
    Migrated { backup_path: PathBuf },
}

/// The bare key a top-level `key = value` TOML line assigns, if any.
fn top_level_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    Some(key.trim())
}

/// Idempotent text-level migration to `DEPLOYMENT_SCHEMA_VERSION`:
/// stamp `schema_version`, and add `node_id`/`role` if the file does not
/// declare them yet. Deliberately a textual patch, not a parse+reserialize
/// round trip — TOML reserialization would reorder keys and drop comments,
/// and this file explicitly invites hand-editing
/// (docs/ALMALINUX_DEPLOYMENT.md); operator formatting must survive.
///
/// Only the top-level region (everything before the first `[table]`
/// header) is inspected, so a key of the same name inside a table can
/// never be mistaken for the top-level one. An existing `role` is never
/// rewritten; a file with no `role` predates roles and was an ordinary
/// exit, so that is what is stamped — migration never invents a relay.
/// `node_id` defaults to [`default_node_id_for_host`] of `public_host`.
///
/// Returns `None` if the file is already current (nothing to do).
pub fn migrate_deployment_toml_text(original: &str) -> Option<String> {
    let top_level: Vec<&str> = original
        .lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .collect();
    let value_of = |wanted: &str| {
        top_level.iter().find_map(|line| {
            (top_level_key(line)? == wanted)
                .then(|| line.split_once('=').map(|(_, value)| value.trim()))
                .flatten()
        })
    };
    let explicit_version = value_of("schema_version").and_then(|value| value.parse::<u32>().ok());
    let has_node_id = value_of("node_id").is_some();
    let has_role = value_of("role").is_some();
    if explicit_version.is_some_and(|v| v >= DEPLOYMENT_SCHEMA_VERSION) && has_node_id && has_role {
        return None;
    }
    // Never downgrade a file that already declares a newer feature schema.
    let stamped_version = explicit_version
        .filter(|v| *v > DEPLOYMENT_SCHEMA_VERSION)
        .unwrap_or(DEPLOYMENT_SCHEMA_VERSION);

    let mut identity = String::new();
    if !has_node_id {
        let host = value_of("public_host")
            .map(|value| value.trim_matches('"'))
            .unwrap_or_default();
        identity.push_str(&format!("node_id = {:?}\n", default_node_id_for_host(host)));
    }
    if !has_role {
        identity.push_str("role = \"exit\"\n");
    }

    let header = format!("schema_version = {stamped_version}\n{identity}");
    let mut body = String::new();
    let mut wrote_header = false;
    let mut in_top_level = true;
    for line in original.lines() {
        if line.trim_start().starts_with('[') {
            in_top_level = false;
        }
        if in_top_level && top_level_key(line) == Some("schema_version") {
            if !wrote_header {
                body.push_str(&header);
                wrote_header = true;
            }
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    if !wrote_header {
        body = format!("{header}{body}");
    }
    Some(body)
}

/// Migrate `deployment.toml` at `path` to `DEPLOYMENT_SCHEMA_VERSION`.
/// Refuses (leaving the file untouched) if the original does not parse,
/// or is already newer than this binary supports. Backs up before
/// mutating, validates the migrated text reparses to a config that is
/// identical to the original in every field except `schema_version`
/// (operator settings preserved), then commits atomically.
pub fn migrate_deployment_toml(path: &Path) -> Result<DeploymentMigrationOutcome, CompatError> {
    if !path.exists() {
        return Ok(DeploymentMigrationOutcome::Missing);
    }
    let original = std::fs::read_to_string(path).map_err(|e| CompatError::Io(e.to_string()))?;
    let original_cfg: DeploymentConfig = toml::from_str(&original).map_err(|e| {
        CompatError::Parse(format!(
            "cannot migrate {path:?}: existing file does not parse ({e}); no changes made"
        ))
    })?;
    // Also refuses a schema newer than this binary supports — never
    // "migrate" forward from a file a future vpn-admin already wrote.
    original_cfg.validate()?;

    let Some(patched) = migrate_deployment_toml_text(&original) else {
        return Ok(DeploymentMigrationOutcome::AlreadyCurrent);
    };

    let migrated_cfg: DeploymentConfig = toml::from_str(&patched).map_err(|e| {
        CompatError::Parse(format!(
            "migrated deployment.toml failed to reparse ({e}) — this is a bug, not applying"
        ))
    })?;
    migrated_cfg.validate()?;
    if migrated_cfg.schema_version < DEPLOYMENT_SCHEMA_VERSION {
        return Err(CompatError::Parse(format!(
            "migration produced schema_version {} (expected {DEPLOYMENT_SCHEMA_VERSION}) — refusing to apply",
            migrated_cfg.schema_version
        )));
    }
    // Every field except schema_version itself must be unchanged —
    // compare via a schema_version-normalized JSON projection rather
    // than requiring DeploymentConfig: PartialEq.
    // Role is compared, never normalized away: a migration that changed
    // it (an exit becoming a relay, or a relay silently becoming an
    // unrestricted exit) must be impossible. `node_id` may only be ADDED.
    if migrated_cfg.role != original_cfg.role {
        return Err(CompatError::Parse(format!(
            "migration would change role from {:?} to {:?} — refusing to apply",
            original_cfg.role.as_str(),
            migrated_cfg.role.as_str()
        )));
    }
    let mut original_normalized =
        serde_json::to_value(&original_cfg).map_err(|e| CompatError::Parse(e.to_string()))?;
    let mut migrated_normalized =
        serde_json::to_value(&migrated_cfg).map_err(|e| CompatError::Parse(e.to_string()))?;
    for obj in [
        original_normalized.as_object_mut(),
        migrated_normalized.as_object_mut(),
    ]
    .into_iter()
    .flatten()
    {
        obj.remove("schema_version");
        if original_cfg.node_id.is_empty() {
            obj.remove("node_id");
        }
    }
    if original_normalized != migrated_normalized {
        return Err(CompatError::Parse(
            "migration would change an operator value other than schema_version/new node \
             identity — refusing to apply (this is a bug in migrate_deployment_toml_text)"
                .to_string(),
        ));
    }

    let backup_path = crate::migrate::backup_before_mutate(path)?;
    crate::migrate::atomic_write(path, patched.as_bytes(), 0o644)?;
    Ok(DeploymentMigrationOutcome::Migrated { backup_path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_toml_parses_with_defaults() {
        let toml_str = r#"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#;
        let cfg: DeploymentConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.reality.handshake_port, 443);
        assert_eq!(cfg.subscription.public_port, 8443);
        assert_eq!(cfg.state_dir, PathBuf::from("/etc/vpn/compat"));
        assert_eq!(
            cfg.users_file(),
            PathBuf::from("/etc/vpn/compat/users/users.json")
        );
    }

    #[test]
    fn udp_probe_section_parses_and_defaults_apply() {
        let toml_str = r#"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100

[udp_probe]
ipv4_resolvers = ["9.9.9.9"]
retries = 3
timeout_ms = 1500
"#;
        let cfg: DeploymentConfig = toml::from_str(toml_str).unwrap();
        let udp = cfg.udp_probe_config();
        assert_eq!(udp.ipv4_resolvers, vec!["9.9.9.9".to_string()]);
        assert_eq!(udp.retries, 3usize);
        assert_eq!(udp.timeout_ms, 1500u64);
        // unspecified fields take defaults
        assert!(!udp.ipv6_resolvers.is_empty());
        assert_eq!(udp.delay_ms, 250u64);
    }

    fn base_toml() -> String {
        r#"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443
"#
        .to_string()
    }

    #[test]
    fn hysteria2_bandwidth_defaults_to_unset() {
        let toml_str = format!("{}\n[subscription]\nlisten_port = 9100\n", base_toml());
        let cfg: DeploymentConfig = toml::from_str(&toml_str).unwrap();
        assert!(cfg.hysteria2.up_mbps.is_none());
        assert!(cfg.hysteria2.down_mbps.is_none());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn hysteria2_bandwidth_set_together_is_valid() {
        let toml_str = format!(
            "{}up_mbps = 100\ndown_mbps = 80\n\n[subscription]\nlisten_port = 9100\n",
            base_toml()
        );
        let cfg: DeploymentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg.hysteria2.up_mbps, Some(100));
        assert_eq!(cfg.hysteria2.down_mbps, Some(80));
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn hysteria2_bandwidth_set_alone_fails_validation() {
        let toml_str = format!(
            "{}up_mbps = 100\n\n[subscription]\nlisten_port = 9100\n",
            base_toml()
        );
        let cfg: DeploymentConfig = toml::from_str(&toml_str).unwrap();
        assert!(cfg.validate().is_err());
    }

    // --- schema versioning / migration ---

    fn legacy_toml() -> String {
        format!("{}\n[subscription]\nlisten_port = 9100\n", base_toml())
    }

    #[test]
    fn missing_schema_version_defaults_to_zero_and_still_loads() {
        let cfg: DeploymentConfig = toml::from_str(&legacy_toml()).unwrap();
        assert_eq!(cfg.schema_version, 0);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn explicit_current_schema_version_loads() {
        let toml_str = format!(
            "schema_version = {DEPLOYMENT_SCHEMA_VERSION}\nnode_id = \"test-node\"\nrole = \"exit\"\n{}",
            legacy_toml()
        );
        let cfg: DeploymentConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg.schema_version, DEPLOYMENT_SCHEMA_VERSION);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn future_schema_version_is_refused() {
        let toml_str = format!("schema_version = 99\n{}", legacy_toml());
        let cfg: DeploymentConfig = toml::from_str(&toml_str).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            CompatError::UnsupportedSchema { found: 99, .. }
        ));
    }

    #[test]
    fn migrate_text_inserts_marker_once_and_is_idempotent() {
        let original = legacy_toml();
        let patched = migrate_deployment_toml_text(&original).expect("should patch");
        assert!(patched.starts_with(&format!("schema_version = {DEPLOYMENT_SCHEMA_VERSION}\n")));
        assert!(patched.contains("node_id = \"vpn\""));
        assert!(patched.contains("role = \"exit\""));
        // idempotent: already-versioned text is untouched
        assert_eq!(migrate_deployment_toml_text(&patched), None);
    }

    #[test]
    fn migrate_deployment_toml_end_to_end_backs_up_migrates_preserves_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        let original = legacy_toml();
        std::fs::write(&path, &original).unwrap();

        let outcome = migrate_deployment_toml(&path).unwrap();
        let backup_path = match outcome {
            DeploymentMigrationOutcome::Migrated { backup_path } => backup_path,
            other => panic!("expected Migrated, got {other:?}"),
        };
        assert!(backup_path.exists());
        assert_eq!(std::fs::read_to_string(&backup_path).unwrap(), original);

        let migrated_text = std::fs::read_to_string(&path).unwrap();
        let migrated_cfg: DeploymentConfig = toml::from_str(&migrated_text).unwrap();
        assert_eq!(migrated_cfg.schema_version, DEPLOYMENT_SCHEMA_VERSION);
        assert_eq!(migrated_cfg.node_id, "vpn");
        assert_eq!(migrated_cfg.role, NodeRole::Exit);

        let original_cfg: DeploymentConfig = toml::from_str(&original).unwrap();
        assert_eq!(migrated_cfg.public_host, original_cfg.public_host);
        assert_eq!(
            migrated_cfg.reality.handshake_server,
            original_cfg.reality.handshake_server
        );
        assert_eq!(
            migrated_cfg.subscription.listen_port,
            original_cfg.subscription.listen_port
        );

        // idempotent: running again on the already-migrated file is a no-op
        let second = migrate_deployment_toml(&path).unwrap();
        assert_eq!(second, DeploymentMigrationOutcome::AlreadyCurrent);
    }

    #[test]
    fn migrate_deployment_toml_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert_eq!(
            migrate_deployment_toml(&path).unwrap(),
            DeploymentMigrationOutcome::Missing
        );
    }

    #[test]
    fn migrate_deployment_toml_refuses_corrupted_input_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        let corrupted = "this is not valid = = toml [[[";
        std::fs::write(&path, corrupted).unwrap();

        let err = migrate_deployment_toml(&path).unwrap_err();
        assert!(matches!(err, CompatError::Parse(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupted);
    }

    #[test]
    fn migrate_deployment_toml_refuses_future_schema_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        let future = format!("schema_version = 99\n{}", legacy_toml());
        std::fs::write(&path, &future).unwrap();

        let err = migrate_deployment_toml(&path).unwrap_err();
        assert!(matches!(
            err,
            CompatError::UnsupportedSchema { found: 99, .. }
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), future);
    }

    #[cfg(unix)]
    #[test]
    fn migrate_deployment_toml_backup_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, legacy_toml()).unwrap();
        let outcome = migrate_deployment_toml(&path).unwrap();
        let backup_path = match outcome {
            DeploymentMigrationOutcome::Migrated { backup_path } => backup_path,
            other => panic!("expected Migrated, got {other:?}"),
        };
        let mode = std::fs::metadata(&backup_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
