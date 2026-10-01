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
    /// Hard lifetime of one native authorization window, seconds (clamped to a safe
    /// leasable minimum and at most 1800 seconds).
    /// A leased credential never outlives its generation: the node renders
    /// it out and rotates the secret at `valid_until`, control plane or not.
    #[serde(default = "default_lease_slot_lifetime_secs")]
    pub lease_slot_lifetime_secs: u64,
    /// Where the node persists its lease table (0600). Survives agent
    /// restarts so expiry/rotation continues across them.
    #[serde(default = "default_lease_state_file")]
    pub lease_state_file: String,
    /// Where the node persists pending job /complete and /fail reports
    /// (0600) that have not yet been acknowledged by the Worker. This
    /// queue is drained by an independent background task (see
    /// `report_queue.rs`) so a wedged Worker endpoint never blocks
    /// heartbeat, health probe, next-job poll, or lease expiry
    /// enforcement (Phase 8). Survives agent restarts.
    #[serde(default = "default_report_queue_file")]
    pub report_queue_file: String,
    /// Rotation batch window, seconds (clamped 60..=600). Every slot's
    /// valid_until lies on this grid; non-urgent rotations (each of which
    /// restarts sing-box and drops every open connection on the node) are
    /// coalesced to at most one apply per window. Expiry and urgent
    /// revocations are never deferred.
    #[serde(default = "default_rotation_batch_interval_secs")]
    pub rotation_batch_interval_secs: u64,
    /// Heartbeat cadence in seconds, clamped to 10..=60. Defaults to 60,
    /// which vpn-web's silence detection (3 x 60 s) assumes; shorter is
    /// always safe for that rule.
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u64,
    /// Protocol-level synthetic probing (REALITY/Hysteria2 handshakes to
    /// peers and self). Absent disables it entirely.
    #[serde(default)]
    pub protocol_probe: Option<ProtocolProbeConfig>,
    /// Where the node persists its operation-id dedup log (0600). A4
    /// (Batch 5): if the same job (identified by the Worker's `job.id`)
    /// is claimed and applied twice — e.g. because the agent restarted
    /// between mutating state and reporting completion, and the Worker
    /// re-delivered the same job — this file lets the second application
    /// converge to the previously recorded outcome instead of re-running
    /// vpn-admin. See `op_dedup.rs`. Survives agent restarts, same as
    /// `lease_state_file`/`report_queue_file`.
    #[serde(default = "default_op_dedup_file")]
    pub op_dedup_file: String,
}

fn default_rotation_batch_interval_secs() -> u64 {
    crate::lease_pool::DEFAULT_BATCH_SECS as u64
}

fn default_lease_pool_size() -> usize {
    // Fail safe: unmanaged/self-host nodes must not synthesize lease users
    // (and restart sing-box to rotate them) merely because the setting was
    // omitted. Fleet bootstrap opts in explicitly when managed leases are
    // actually enabled for the node.
    0
}

fn default_lease_slot_lifetime_secs() -> u64 {
    1800
}

fn default_lease_state_file() -> String {
    "/var/lib/vpn-provisioning-agent/lease-pool.json".to_string()
}

fn default_report_queue_file() -> String {
    "/var/lib/vpn-provisioning-agent/report-queue.json".to_string()
}

fn default_op_dedup_file() -> String {
    "/var/lib/vpn-provisioning-agent/op-dedup.json".to_string()
}

/// `[protocol_probe]` — see `protocol_probe.rs` for what each dimension
/// measures and how probe credentials are provisioned.
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct ProtocolProbeConfig {
    pub singbox_binary: String,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    /// Name of this node's reserved, non-customer probe user.
    pub probe_user_name: String,
    /// Create/reuse the probe user and publish its links to the control
    /// plane so peers can probe this node.
    pub publish_self: bool,
    /// Ask the control plane for peer targets.
    pub fetch_targets: bool,
    /// Also probe this node's own listeners over loopback (fallback when
    /// no peer covers it; the control plane prefers peer results).
    pub self_probe: bool,
    pub self_probe_host: String,
    pub hysteria2_cert_path: Option<String>,
    /// Skip Hysteria2 certificate verification. Test rigs with
    /// self-signed certificates only; production certificates verify.
    pub tls_insecure_for_tests: bool,
    /// Extra targets from local config (tests / offline operation).
    pub static_targets: Vec<StaticProbeTarget>,
}

impl Default for ProtocolProbeConfig {
    fn default() -> Self {
        Self {
            singbox_binary: "sing-box".into(),
            interval_secs: 60,
            timeout_secs: 8,
            probe_user_name: compat_config::model::PROBE_USER_NAME.into(),
            publish_self: true,
            fetch_targets: true,
            self_probe: true,
            self_probe_host: "127.0.0.1".into(),
            hysteria2_cert_path: Some("/etc/vpn/compat/hysteria/cert.pem".into()),
            tls_insecure_for_tests: false,
            static_targets: Vec::new(),
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct StaticProbeTarget {
    pub node_id: String,
    #[serde(default)]
    pub expected_ipv4: Option<String>,
    #[serde(default)]
    pub reality_uri: Option<String>,
    #[serde(default)]
    pub hysteria2_uri: Option<String>,
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
            .field("report_queue_file", &self.report_queue_file)
            .field("op_dedup_file", &self.op_dedup_file)
            .field(
                "rotation_batch_interval_secs",
                &self.rotation_batch_interval_secs,
            )
            .field("heartbeat_interval_secs", &self.heartbeat_interval_secs)
            .field(
                "protocol_probe",
                &self.protocol_probe.as_ref().map(|_| "<configured>"),
            )
            .finish()
    }
}

fn default_heartbeat_interval_secs() -> u64 {
    60
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
        assert_eq!(
            cfg.lease_pool_size, 0,
            "lease pool must be opt-in; an omitted setting must not create periodic node-wide restarts (F02)"
        );
    }

    #[test]
    fn load_respects_an_explicit_zero_lease_pool_size() {
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
lease_pool_size = 0
"#,
        )
        .unwrap();

        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.lease_pool_size, 0);
    }

    #[test]
    fn load_respects_an_explicit_nonzero_lease_pool_size() {
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
lease_pool_size = 16
"#,
        )
        .unwrap();

        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.lease_pool_size, 16);
    }

    #[test]
    fn load_rejects_a_non_numeric_lease_pool_size() {
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
lease_pool_size = "not-a-number"
"#,
        )
        .unwrap();

        assert!(
            AgentConfig::load(&path).is_err(),
            "an invalid lease_pool_size must not silently coerce to a default"
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
    fn protocol_probe_section_parses_with_defaults() {
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
[protocol_probe]
interval_secs = 30
[[protocol_probe.static_targets]]
node_id = "peer"
reality_uri = "vless://secret-uuid@h:1?security=reality&pbk=k"
"#,
        )
        .unwrap();
        let cfg = AgentConfig::load(&path).unwrap();
        assert_eq!(cfg.heartbeat_interval_secs, 60);
        let p = cfg.protocol_probe.as_ref().unwrap();
        assert_eq!(p.interval_secs, 30);
        assert!(p.publish_self && p.self_probe && !p.tls_insecure_for_tests);
        assert_eq!(p.static_targets.len(), 1);
        assert!(!format!("{cfg:?}").contains("secret-uuid"));
    }

    #[test]
    fn load_fails_on_missing_file() {
        let result = AgentConfig::load(Path::new("/nonexistent/provisioning-agent.toml"));
        assert!(result.is_err());
    }
}
