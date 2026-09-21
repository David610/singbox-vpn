use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Agent configuration, loaded from a TOML file (default
/// `/etc/vpn/provisioning-agent.toml` on a real VPS, an arbitrary path in
/// tests). Deliberately separate from `vpn-admin`'s own
/// `deployment.toml` — this agent's concerns (which Worker to poll, with
/// which credential) are unrelated to VPN deployment topology, and
/// keeping them in separate files means neither can accidentally corrupt
/// the other.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Base URL of the vpn-web Worker API, e.g. `https://example.com` —
    /// no trailing slash.
    pub worker_url: String,
    /// This agent's node_id, must match a row in vpn-web's `nodes` table.
    pub node_id: String,
    /// The raw per-node API key `scripts/register-node.mjs` printed when
    /// this node was registered. Sent as `Authorization: Bearer
    /// <agent_api_key>` on every Worker API call.
    pub agent_api_key: String,
    /// How often to poll for a new job, in seconds.
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Path to the `vpn-admin` (or `vpn`) binary this agent shells out to.
    pub vpn_admin_binary: String,
    /// Path to the `deployment.toml` this agent's `vpn-admin` invocations
    /// should use — passed as `vpn-admin --config <this>`.
    pub vpn_admin_config: String,
}

fn default_poll_interval_secs() -> u64 {
    15
}

impl AgentConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading agent config from {path:?}"))?;
        let cfg: AgentConfig =
            toml::from_str(&text).with_context(|| format!("parsing agent config from {path:?}"))?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_parses_a_minimal_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provisioning-agent.toml");
        std::fs::write(
            &path,
            r#"
worker_url = "http://127.0.0.1:8788"
node_id = "node-1"
agent_api_key = "test-key"
vpn_admin_binary = "/usr/local/bin/vpn-admin"
vpn_admin_config = "/etc/vpn/deployment.toml"
"#,
        )
        .unwrap();

        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.worker_url, "http://127.0.0.1:8788");
        assert_eq!(cfg.node_id, "node-1");
        assert_eq!(
            cfg.poll_interval_secs, 15,
            "default should apply when omitted"
        );
    }

    #[test]
    fn load_respects_an_explicit_poll_interval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("provisioning-agent.toml");
        std::fs::write(
            &path,
            r#"
worker_url = "http://127.0.0.1:8788"
node_id = "node-1"
agent_api_key = "test-key"
poll_interval_secs = 5
vpn_admin_binary = "/usr/local/bin/vpn-admin"
vpn_admin_config = "/etc/vpn/deployment.toml"
"#,
        )
        .unwrap();

        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.poll_interval_secs, 5);
    }

    #[test]
    fn load_fails_on_missing_file() {
        let result = AgentConfig::load(Path::new("/nonexistent/provisioning-agent.toml"));
        assert!(result.is_err());
    }
}
