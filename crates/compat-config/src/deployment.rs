//! `/etc/vpn/deployment.toml` schema — shared by `vpn-admin` and
//! `services/subscription` so the domain/port/path configuration used to
//! render subscriptions and sing-box config lives in exactly one place,
//! never hardcoded into source (spec §36).

use crate::CompatError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentConfig {
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
}

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
        toml::from_str(&text).map_err(|e| CompatError::Parse(e.to_string()))
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

    pub fn singbox_config_file(&self) -> PathBuf {
        self.state_dir.join("sing-box/config.json")
    }
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
handshake_server = "www.microsoft.com"

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
handshake_server = "www.microsoft.com"

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
}
