//! Transport vocabulary and the provider abstraction.
//!
//! Orchestration, the route catalog, scoring and failover only ever see
//! [`TransportKind`] and [`TransportCapabilities`]. Protocol-specific
//! behaviour (credential shapes, server config syntax, client profile
//! fields) lives exclusively in implementations of [`TransportProvider`]
//! — for this repository, in `compat-config`.

use crate::health::HealthLayer;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A transport family. Unknown wire values round-trip as `Other` so a
/// newer document never fails to parse on an older consumer; consumers
/// must skip routes whose transport they cannot run.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TransportKind {
    VlessReality,
    Hysteria2,
    AmneziaWg,
    Other(String),
}

impl TransportKind {
    pub fn as_str(&self) -> &str {
        match self {
            TransportKind::VlessReality => "vless-reality",
            TransportKind::Hysteria2 => "hysteria2",
            TransportKind::AmneziaWg => "amneziawg",
            TransportKind::Other(s) => s,
        }
    }

    pub fn from_wire(s: &str) -> Self {
        match s {
            "vless-reality" => TransportKind::VlessReality,
            "hysteria2" => TransportKind::Hysteria2,
            "amneziawg" => TransportKind::AmneziaWg,
            other => TransportKind::Other(other.to_string()),
        }
    }

    /// Known capabilities, or `None` for a transport this build does not
    /// implement.
    pub fn capabilities(&self) -> Option<TransportCapabilities> {
        match self {
            TransportKind::VlessReality => Some(TransportCapabilities {
                engine: DataPlaneEngine::SingBox,
                l4: L4Protocol::Tcp,
                carries_udp_payload: true,
                detour_capable: true,
                per_user_credentials: true,
                supports_rotation: true,
                supports_live_revocation: true,
                fallback_formats: vec![FallbackFormat::ShareLink, FallbackFormat::SingBoxJson],
                requires_kernel_module: false,
                relative_cpu_cost: CpuCost::Medium,
            }),
            TransportKind::Hysteria2 => Some(TransportCapabilities {
                engine: DataPlaneEngine::SingBox,
                l4: L4Protocol::Udp,
                carries_udp_payload: true,
                detour_capable: false,
                per_user_credentials: true,
                supports_rotation: true,
                supports_live_revocation: true,
                fallback_formats: vec![FallbackFormat::ShareLink, FallbackFormat::SingBoxJson],
                requires_kernel_module: false,
                relative_cpu_cost: CpuCost::High,
            }),
            TransportKind::AmneziaWg => Some(TransportCapabilities {
                engine: DataPlaneEngine::AmneziaWg,
                l4: L4Protocol::Udp,
                carries_udp_payload: true,
                detour_capable: false,
                per_user_credentials: true,
                supports_rotation: true,
                supports_live_revocation: true,
                fallback_formats: vec![FallbackFormat::AwgQuickIni],
                requires_kernel_module: false,
                relative_cpu_cost: CpuCost::Low,
            }),
            TransportKind::Other(_) => None,
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TransportKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TransportKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(TransportKind::from_wire(&s))
    }
}

/// The engine that terminates a transport. A client runs one engine at a
/// time; switching engines is a full tunnel rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataPlaneEngine {
    SingBox,
    AmneziaWg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum L4Protocol {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackFormat {
    ShareLink,
    SingBoxJson,
    AwgQuickIni,
}

/// Declared relative CPU cost used as a scoring tie-breaker until real
/// measurements replace it (`docs/platform-v2/BENCHMARK_SPEC.md`). A
/// declaration, not a benchmark result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuCost {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportCapabilities {
    pub engine: DataPlaneEngine,
    pub l4: L4Protocol,
    /// Whether user UDP traffic can be carried inside the tunnel.
    pub carries_udp_payload: bool,
    /// Whether a route may use this transport as the exit of a Core
    /// `detour` chain through a relay. Only TCP transports qualify today.
    pub detour_capable: bool,
    pub per_user_credentials: bool,
    pub supports_rotation: bool,
    pub supports_live_revocation: bool,
    pub fallback_formats: Vec<FallbackFormat>,
    pub requires_kernel_module: bool,
    pub relative_cpu_cost: CpuCost,
}

impl TransportCapabilities {
    /// Whether a relay hop using `first` can carry an exit using `self`.
    pub fn can_chain_through(&self, first: &TransportCapabilities) -> bool {
        self.detour_capable
            && first.detour_capable
            && self.engine == DataPlaneEngine::SingBox
            && first.engine == DataPlaneEngine::SingBox
            && self.l4 == L4Protocol::Tcp
            && first.l4 == L4Protocol::Tcp
    }
}

/// A health probe a transport supports, and which layer it proves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeSpec {
    pub layer: HealthLayer,
    pub name: String,
    /// Whether the probe needs elevated privileges (e.g. an AWG netns).
    pub requires_privilege: bool,
}

/// A pinned external artifact a transport needs on a node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedArtifact {
    pub name: String,
    pub version: String,
    /// Exactly one of `sha256` (for a downloaded binary) or `git_commit`
    /// (for a source build) identifies the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    pub license: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRule {
    pub l4: L4Protocol,
    pub port: u16,
}

/// Declarative install requirements, consumed by the shell installer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub transport: TransportKind,
    pub artifacts: Vec<PinnedArtifact>,
    pub systemd_units: Vec<String>,
    pub firewall: Vec<PortRule>,
    /// Reversible sysctl keys the transport needs (e.g. IP forwarding).
    pub sysctls: Vec<(String, String)>,
}

impl InstallPlan {
    /// Every artifact must be pinned to exact bytes or an exact commit.
    pub fn validate(&self) -> Result<(), TransportError> {
        for artifact in &self.artifacts {
            let sha_ok = artifact
                .sha256
                .as_deref()
                .is_some_and(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()));
            let commit_ok = artifact
                .git_commit
                .as_deref()
                .is_some_and(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()));
            if sha_ok == commit_ok {
                return Err(TransportError::Unpinned(artifact.name.clone()));
            }
            if artifact.version.trim().is_empty() || artifact.license.trim().is_empty() {
                return Err(TransportError::Unpinned(artifact.name.clone()));
            }
        }
        Ok(())
    }
}

/// What revoking a credential requires before it takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationEffect {
    /// Takes effect once the re-rendered server artifact is applied.
    OnApply,
    /// Existing sessions survive until the engine restarts.
    OnRestart,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("invalid transport configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid credential: {0}")]
    InvalidCredential(String),
    #[error("credential material is reused across users or nodes: {0}")]
    CredentialReuse(String),
    #[error("address pool exhausted")]
    AddressPoolExhausted,
    #[error("artifact {0:?} is not pinned to an exact sha256 or git commit")]
    Unpinned(String),
    #[error("operation not supported by this transport: {0}")]
    Unsupported(&'static str),
}

/// Object-safe transport description used by orchestration code that
/// must not depend on protocol-specific types.
pub trait TransportDescriptor {
    fn kind(&self) -> TransportKind;
    fn capabilities(&self) -> TransportCapabilities;
    fn health_probes(&self) -> Vec<ProbeSpec>;
}

/// The full provider contract. Associated types keep credential and
/// artifact shapes owned by the implementation; nothing outside the
/// provider needs to know a VLESS UUID from an AWG key.
pub trait TransportProvider: TransportDescriptor {
    /// Node-level transport configuration (listen port, public keys, …).
    type NodeConfig;
    /// One user's credential for this transport on one node.
    type Credential;
    /// The server-side artifact (JSON inbound, `awg setconf` text, …).
    type ServerArtifact;
    /// The per-user client endpoint description.
    type ClientEndpoint;

    fn validate(&self, config: &Self::NodeConfig) -> Result<(), TransportError>;

    /// Issue a fresh credential for `user_id`. `existing` holds every
    /// credential already issued on this node, so implementations can
    /// refuse collisions (and allocate addresses) deterministically.
    fn issue_credentials(
        &self,
        config: &Self::NodeConfig,
        user_id: &str,
        existing: &[(&str, &Self::Credential)],
    ) -> Result<Self::Credential, TransportError>;

    /// Replace secret material while keeping stable allocations.
    fn rotate_credentials(
        &self,
        config: &Self::NodeConfig,
        user_id: &str,
        current: &Self::Credential,
        existing: &[(&str, &Self::Credential)],
    ) -> Result<Self::Credential, TransportError>;

    fn revocation_effect(&self) -> RevocationEffect;

    /// Render the server artifact for exactly the active users given.
    /// Revocation is expressed by omitting a user here.
    fn render_server_artifact(
        &self,
        config: &Self::NodeConfig,
        active: &[(&str, &Self::Credential)],
    ) -> Result<Self::ServerArtifact, TransportError>;

    fn render_client_endpoint(
        &self,
        config: &Self::NodeConfig,
        user_id: &str,
        credential: &Self::Credential,
    ) -> Result<Self::ClientEndpoint, TransportError>;

    fn install_plan(&self, config: &Self::NodeConfig) -> InstallPlan;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip_and_unknowns_are_preserved() {
        for kind in [
            TransportKind::VlessReality,
            TransportKind::Hysteria2,
            TransportKind::AmneziaWg,
            TransportKind::Other("naive".into()),
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: TransportKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
        assert!(TransportKind::Other("naive".into())
            .capabilities()
            .is_none());
    }

    #[test]
    fn only_tcp_singbox_transports_chain_through_a_relay() {
        let reality = TransportKind::VlessReality.capabilities().unwrap();
        let hy2 = TransportKind::Hysteria2.capabilities().unwrap();
        let awg = TransportKind::AmneziaWg.capabilities().unwrap();
        assert!(reality.can_chain_through(&reality));
        assert!(!hy2.can_chain_through(&reality));
        assert!(!awg.can_chain_through(&reality));
        assert!(!reality.can_chain_through(&awg));
    }

    #[test]
    fn awg_is_its_own_engine() {
        assert_eq!(
            TransportKind::AmneziaWg.capabilities().unwrap().engine,
            DataPlaneEngine::AmneziaWg
        );
    }

    fn artifact(sha: Option<&str>, commit: Option<&str>) -> PinnedArtifact {
        PinnedArtifact {
            name: "amneziawg-go".into(),
            version: "v3.1.20260828".into(),
            sha256: sha.map(str::to_string),
            git_commit: commit.map(str::to_string),
            license: "MIT".into(),
        }
    }

    fn plan(artifacts: Vec<PinnedArtifact>) -> InstallPlan {
        InstallPlan {
            transport: TransportKind::AmneziaWg,
            artifacts,
            systemd_units: vec![],
            firewall: vec![],
            sysctls: vec![],
        }
    }

    #[test]
    fn install_plan_requires_exactly_one_exact_pin() {
        let commit = "b5928efb6ca19f0153958460c3d141f04abc5c2e";
        let sha = "6afd9d43091fa157ffd0832df98c08a8237a9f51c74cf0d65138972748fe5766";
        assert!(plan(vec![artifact(None, Some(commit))]).validate().is_ok());
        assert!(plan(vec![artifact(Some(sha), None)]).validate().is_ok());
        assert!(plan(vec![artifact(None, None)]).validate().is_err());
        assert!(plan(vec![artifact(Some(sha), Some(commit))])
            .validate()
            .is_err());
        assert!(plan(vec![artifact(None, Some("main"))]).validate().is_err());
        assert!(plan(vec![artifact(Some("latest"), None)])
            .validate()
            .is_err());
    }
}
