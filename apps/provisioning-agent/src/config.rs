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
#[derive(Clone, Deserialize)]
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
    /// Base URL of sing-box's Clash API, e.g. `http://127.0.0.1:9090`.
    ///
    /// Absent (the default) disables traffic reporting entirely, so an
    /// agent deployed before sing-box has `experimental.clash_api`
    /// configured behaves exactly as it did before this feature existed
    /// rather than logging an error every poll.
    #[serde(default)]
    pub clash_api_url: Option<String>,
    /// The Clash API's `secret`, if one is configured. Never logged: it
    /// grants read access to sing-box's runtime state, so `Debug` for this
    /// struct redacts it below.
    #[serde(default)]
    pub clash_api_secret: Option<String>,
    /// Overrides the outbound tag `health_probe::probe_data_plane` probes
    /// via the Clash API delay-test endpoint. Absent (the default) probes
    /// `"direct"`, the tag every server-side sing-box config actually
    /// renders (`crates/compat-config/src/server.rs`). Only needed if an
    /// operator later changes the server-side outbound topology.
    #[serde(default)]
    pub clash_probe_outbound: Option<String>,
    /// ADR-0003: number of pre-provisioned pseudonymous lease slots this
    /// node keeps live for `/v1/vpn/authorize`. Bounded (at most 1024);
    /// `0` disables the lease pool entirely (the node then removes any
    /// lease-slot users on its next tick).
    #[serde(default = "default_lease_pool_size")]
    pub lease_pool_size: usize,
    /// Hard lifetime of one slot generation, seconds (clamped 900..=7200).
    /// A leased credential never outlives its generation: the node renders
    /// it out and rotates the secret at `valid_until`, control plane or not.
    #[serde(default = "default_lease_slot_lifetime_secs")]
    pub lease_slot_lifetime_secs: u64,
    /// Where the node persists its lease table (0600). Survives agent
    /// restarts so expiry/rotation continues across them.
    #[serde(default = "default_lease_state_file")]
    pub lease_state_file: String,
}

fn default_lease_pool_size() -> usize {
    32
}

fn default_lease_slot_lifetime_secs() -> u64 {
    1800
}

fn default_lease_state_file() -> String {
    "/var/lib/vpn-provisioning-agent/lease-pool.json".to_string()
}

// Derived Debug would print clash_api_secret and agent_api_key verbatim,
// and this struct is logged on unexpected-config errors. Implement it by
// hand so a credential cannot reach the journal that way.
impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("worker_url", &self.worker_url)
            .field("node_id", &self.node_id)
            .field("agent_api_key", &"<redacted>")
            .field("poll_interval_secs", &self.poll_interval_secs)
            .field("vpn_admin_binary", &self.vpn_admin_binary)
            .field("vpn_admin_config", &self.vpn_admin_config)
            .field("clash_api_url", &self.clash_api_url)
            .field(
                "clash_api_secret",
                &self.clash_api_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("clash_probe_outbound", &self.clash_probe_outbound)
            .field("lease_pool_size", &self.lease_pool_size)
            .field("lease_slot_lifetime_secs", &self.lease_slot_lifetime_secs)
            .field("lease_state_file", &self.lease_state_file)
            .finish()
    }
}

fn default_poll_interval_secs() -> u64 {
    3
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
            cfg.poll_interval_secs, 3,
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
