//! Probe configuration file (`probe.toml`).
//!
//! Profiles referenced here are credentials of a dedicated probe user on
//! each node (never a real user's). They must be mode 0600; the agent
//! refuses group- or world-readable profiles.

use anyhow::{bail, Context, Result};
use platform_core::evidence::{NetworkClass, Vantage};
use platform_core::health::HealthLayer;
use platform_core::probe::{ProbeMatrix, ProbeTarget};
use platform_core::transport::TransportKind;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeConfig {
    pub vantage_id: String,
    pub vantage_class: NetworkClass,
    #[serde(default)]
    pub vantage_country: Option<String>,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    /// Fetched through the tunnel for L4. Expect HTTP 200 or 204.
    pub l4_url: String,
    /// Bounded download for L5; only used for targets listing L5.
    #[serde(default)]
    pub l5_url: Option<String>,
    #[serde(default = "default_l5_bytes")]
    pub l5_bytes: u64,
    #[serde(default = "default_l5_deadline")]
    pub l5_deadline_seconds: u64,
    #[serde(default = "default_singbox")]
    pub singbox_binary: PathBuf,
    #[serde(default = "default_awg_go")]
    pub amneziawg_go_binary: PathBuf,
    #[serde(default = "default_awg")]
    pub awg_binary: PathBuf,
    /// Resolver written into the AmneziaWG probe namespace.
    #[serde(default = "default_resolver")]
    pub awg_namespace_resolver: String,
    pub targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub route_id: String,
    pub node_id: String,
    pub transport: TransportKind,
    pub layers: Vec<HealthLayer>,
    /// sing-box outbound JSON (vless-reality/hysteria2) or awg-quick INI
    /// (amneziawg) for the dedicated probe user.
    pub profile: PathBuf,
}

fn default_interval() -> u64 {
    900
}
fn default_l5_bytes() -> u64 {
    4 * 1024 * 1024
}
fn default_l5_deadline() -> u64 {
    30
}
fn default_singbox() -> PathBuf {
    "/usr/local/bin/sing-box".into()
}
fn default_awg_go() -> PathBuf {
    "/usr/local/bin/amneziawg-go".into()
}
fn default_awg() -> PathBuf {
    "/usr/local/bin/awg".into()
}
fn default_resolver() -> String {
    "1.1.1.1".into()
}

impl ProbeConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
        let cfg: ProbeConfig = toml::from_str(&text).with_context(|| format!("parsing {path:?}"))?;
        cfg.matrix().validate().map_err(|e| anyhow::anyhow!("invalid probe matrix: {e}"))?;
        if cfg.targets.iter().any(|t| t.layers.contains(&HealthLayer::Transfer)) && cfg.l5_url.is_none() {
            bail!("a target requests L5 but l5_url is not set");
        }
        if cfg.awg_namespace_resolver.parse::<std::net::IpAddr>().is_err() {
            bail!("awg_namespace_resolver must be an IP literal");
        }
        for t in &cfg.targets {
            if t.transport.capabilities().is_none() {
                bail!("target {}: transport {} is not supported by this build", t.route_id, t.transport);
            }
            check_profile_permissions(&t.profile)?;
        }
        Ok(cfg)
    }

    pub fn vantage(&self) -> Vantage {
        Vantage::new(self.vantage_class, self.vantage_country.as_deref())
    }

    pub fn matrix(&self) -> ProbeMatrix {
        ProbeMatrix {
            vantage_id: self.vantage_id.clone(),
            vantage: self.vantage(),
            interval_ms: self.interval_seconds.saturating_mul(1000),
            l4_url: self.l4_url.clone(),
            l5_bytes: self.l5_bytes,
            l5_deadline_ms: self.l5_deadline_seconds.saturating_mul(1000),
            targets: self
                .targets
                .iter()
                .map(|t| ProbeTarget {
                    route_id: t.route_id.clone(),
                    node_id: t.node_id.clone(),
                    transport: t.transport.clone(),
                    layers: t.layers.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(unix)]
fn check_profile_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).with_context(|| format!("probe profile {path:?}"))?;
    if meta.permissions().mode() & 0o077 != 0 {
        bail!("probe profile {path:?} is readable by group/others; it holds a credential and must be mode 0600");
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_profile_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write(dir: &Path, profile_mode: u32, extra: &str) -> PathBuf {
        let profile = dir.join("fi1-reality.json");
        std::fs::write(&profile, "{}").unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(profile_mode)).unwrap();
        let path = dir.join("probe.toml");
        std::fs::write(
            &path,
            format!(
                "vantage_id = \"ru-probe-1\"\nvantage_class = \"datacenter\"\nvantage_country = \"RU\"\nl4_url = \"https://probe.example.net/generate_204\"\n{extra}\n[[targets]]\nroute_id = \"fi1/vless-reality\"\nnode_id = \"fi1\"\ntransport = \"vless-reality\"\nlayers = [\"L1\", \"L2\", \"L4\"]\nprofile = \"{}\"\n",
                profile.display()
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn loads_a_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ProbeConfig::load(&write(dir.path(), 0o600, "")).unwrap();
        assert_eq!(cfg.vantage().verified_label(), "RUSSIAN-DATACENTER-NETWORK VERIFIED");
    }

    #[test]
    fn refuses_readable_profiles_short_intervals_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ProbeConfig::load(&write(dir.path(), 0o644, "")).is_err());
        assert!(ProbeConfig::load(&write(dir.path(), 0o600, "interval_seconds = 5")).is_err());
        assert!(ProbeConfig::load(&write(dir.path(), 0o600, "surprise = 1")).is_err());
    }
}
