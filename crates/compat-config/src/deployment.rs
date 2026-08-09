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

    /// Root of the `/etc/vpn/compat` state tree. Defaults applied by
    /// `default_state_dir` if omitted from the TOML file.
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,

    #[serde(default = "default_singbox_binary")]
    pub singbox_binary: PathBuf,
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

impl DeploymentConfig {
    pub fn load(path: &Path) -> Result<Self, CompatError> {
        let text = std::fs::read_to_string(path).map_err(|e| CompatError::Io(e.to_string()))?;
        toml::from_str(&text).map_err(|e| CompatError::Parse(e.to_string()))
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
}
