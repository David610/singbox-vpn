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
pub const DEPLOYMENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {
    /// On-disk schema version. `0` (the default when the key is absent)
    /// means "legacy, pre-versioning" — `vpn-admin config migrate` stamps
    /// it to `DEPLOYMENT_SCHEMA_VERSION` explicitly without touching any
    /// other value. See `DEPLOYMENT_SCHEMA_VERSION`'s doc comment.
    #[serde(default)]
    pub schema_version: u32,

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
    /// actively refused — see [`PeerEndpointSection::validate`].
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
    pub fn validate(&self) -> Result<(), CompatError> {
        // Fail closed on a schema newer than this binary understands —
        // see DEPLOYMENT_SCHEMA_VERSION's doc comment. A version <=
        // current (including the legacy default of 0) is always safe to
        // load: nothing about the shape has changed since versioning was
        // introduced, so there is nothing to reinterpret.
        if self.schema_version > DEPLOYMENT_SCHEMA_VERSION {
            return Err(CompatError::UnsupportedSchema {
                what: "deployment.toml",
                found: self.schema_version,
                max_supported: DEPLOYMENT_SCHEMA_VERSION,
            });
        }
        let mut seen_peer_ids: Vec<&str> = Vec::with_capacity(self.peer_endpoints.len());
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
        }

        if self.hysteria2.up_mbps.is_some() != self.hysteria2.down_mbps.is_some() {
            return Err(CompatError::Parse(
                "[hysteria2] up_mbps and down_mbps must be set together (both, or neither) — \
                 see docs/PERFORMANCE_OPTIMIZATION_PLAN.md"
                    .to_string(),
            ));
        }
        Ok(())
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

/// Idempotent text-level migration: insert an explicit `schema_version =
/// N` marker as the very first line if none is present yet. Deliberately
/// a textual patch, not a full parse+reserialize round trip — TOML
/// reserialization would reorder keys and drop comments, and this file
/// "explicitly invites hand-editing" (see docs/ALMALINUX_DEPLOYMENT.md);
/// operator formatting must survive byte-for-byte. Returns `None` if the
/// file already has an explicit `schema_version` key anywhere (nothing
/// to do).
pub fn migrate_deployment_toml_text(original: &str) -> Option<String> {
    let already_versioned = original
        .lines()
        .any(|l| l.trim_start().starts_with("schema_version"));
    if already_versioned {
        return None;
    }
    Some(format!(
        "schema_version = {DEPLOYMENT_SCHEMA_VERSION}\n{original}"
    ))
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
    if migrated_cfg.schema_version != DEPLOYMENT_SCHEMA_VERSION {
        return Err(CompatError::Parse(format!(
            "migration produced schema_version {} (expected {DEPLOYMENT_SCHEMA_VERSION}) — refusing to apply",
            migrated_cfg.schema_version
        )));
    }
    // Every field except schema_version itself must be unchanged —
    // compare via a schema_version-normalized JSON projection rather
    // than requiring DeploymentConfig: PartialEq.
    let mut original_normalized =
        serde_json::to_value(&original_cfg).map_err(|e| CompatError::Parse(e.to_string()))?;
    let mut migrated_normalized =
        serde_json::to_value(&migrated_cfg).map_err(|e| CompatError::Parse(e.to_string()))?;
    if let Some(obj) = original_normalized.as_object_mut() {
        obj.remove("schema_version");
    }
    if let Some(obj) = migrated_normalized.as_object_mut() {
        obj.remove("schema_version");
    }
    if original_normalized != migrated_normalized {
        return Err(CompatError::Parse(
            "migration would change a value other than schema_version — refusing to apply \
             (this is a bug in migrate_deployment_toml_text)"
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
            "schema_version = {DEPLOYMENT_SCHEMA_VERSION}\n{}",
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
        assert!(patched.starts_with("schema_version = 1\n"));
        assert!(patched.ends_with(&original));
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
