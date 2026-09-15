//! Provider-independent VPS provisioning interface.
//!
//! Business logic (bootstrap, replacement) talks only to
//! [`ProviderDriver`]. A real cloud driver lives in its own crate and must
//! pass [`contract::run`]. This crate ships only [`MockProvider`]:
//! nothing in the repository establishes which cloud provider comes
//! first, and inventing one would be an unfounded business decision.

use crate::transport::PortRule;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A provider API credential. Never `Debug`-printed, never serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderToken(String);

impl ProviderToken {
    pub fn new(value: impl Into<String>) -> Self {
        ProviderToken(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderToken(<redacted>)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Idempotency key: creating twice with the same key returns the same
    /// node instead of a second one.
    pub request_id: String,
    pub node_id: String,
    pub region: String,
    pub size: String,
    pub image: String,
    /// Reference to an SSH public key already registered with the
    /// provider; never private key material.
    pub ssh_key_ref: String,
    /// cloud-init user-data. Must not contain long-lived secrets; the
    /// bootstrap token inside it is one-time and short-lived.
    pub user_data: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNodeState {
    Pending,
    Running,
    Stopped,
    Deleting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderNode {
    pub provider_id: String,
    pub node_id: String,
    pub region: String,
    pub state: ProviderNodeState,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub created_at_ms: TimestampMs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Ready,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpReplacement {
    Unsupported,
    Replaced {
        ipv4: Option<String>,
        ipv6: Option<String>,
    },
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    #[error("provider rate limit; retry after {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("provider quota exceeded")]
    QuotaExceeded,
    #[error("provider authentication failed")]
    AuthFailed,
    #[error("provider resource not found: {0}")]
    NotFound(String),
    #[error("provider unavailable")]
    Unavailable,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

impl ProviderError {
    /// Whether a bounded retry may help.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimited { .. } | ProviderError::Unavailable
        )
    }
}

pub trait ProviderDriver {
    fn name(&self) -> &str;
    fn create_node(
        &mut self,
        spec: &NodeSpec,
        now_ms: TimestampMs,
    ) -> Result<ProviderNode, ProviderError>;
    fn destroy_node(&mut self, provider_id: &str) -> Result<(), ProviderError>;
    fn get_node(&self, provider_id: &str) -> Result<ProviderNode, ProviderError>;
    fn list_nodes(&self) -> Result<Vec<ProviderNode>, ProviderError>;
    fn assign_metadata(
        &mut self,
        provider_id: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<(), ProviderError>;
    fn configure_firewall(
        &mut self,
        provider_id: &str,
        rules: &[PortRule],
    ) -> Result<(), ProviderError>;
    fn wait_until_ready(
        &mut self,
        provider_id: &str,
        deadline_ms: TimestampMs,
    ) -> Result<Readiness, ProviderError>;
    fn replace_ip(&mut self, provider_id: &str) -> Result<IpReplacement, ProviderError>;
}

/// Scripted faults for [`MockProvider`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MockFaults {
    /// Errors returned by the next `create_node` calls, in order.
    pub create: Vec<ProviderError>,
    pub destroy: Vec<ProviderError>,
    /// Number of `wait_until_ready` calls that time out before success.
    pub ready_timeouts: u32,
    pub firewall: Vec<ProviderError>,
    /// Every call fails with `Unavailable` while set.
    pub outage: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MockProvider {
    nodes: BTreeMap<String, ProviderNode>,
    by_request: BTreeMap<String, String>,
    firewall: BTreeMap<String, Vec<PortRule>>,
    next_id: u64,
    pub faults: MockFaults,
    pub supports_ip_replacement: bool,
    pub quota: Option<usize>,
    pub calls: Vec<String>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn firewall_rules(&self, provider_id: &str) -> Option<&[PortRule]> {
        self.firewall.get(provider_id).map(Vec::as_slice)
    }

    fn check_outage(&self) -> Result<(), ProviderError> {
        if self.faults.outage {
            Err(ProviderError::Unavailable)
        } else {
            Ok(())
        }
    }
}

fn pop_fault(v: &mut Vec<ProviderError>) -> Option<ProviderError> {
    (!v.is_empty()).then(|| v.remove(0))
}

impl ProviderDriver for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn create_node(
        &mut self,
        spec: &NodeSpec,
        now_ms: TimestampMs,
    ) -> Result<ProviderNode, ProviderError> {
        self.calls.push(format!("create:{}", spec.node_id));
        self.check_outage()?;
        if let Some(e) = pop_fault(&mut self.faults.create) {
            return Err(e);
        }
        if spec.node_id.trim().is_empty() || spec.region.trim().is_empty() {
            return Err(ProviderError::InvalidRequest(
                "node_id and region are required".into(),
            ));
        }
        if let Some(existing) = self.by_request.get(&spec.request_id) {
            return Ok(self.nodes[existing].clone());
        }
        if self.quota.is_some_and(|q| self.nodes.len() >= q) {
            return Err(ProviderError::QuotaExceeded);
        }
        self.next_id += 1;
        let provider_id = format!("mock-{}", self.next_id);
        let node = ProviderNode {
            provider_id: provider_id.clone(),
            node_id: spec.node_id.clone(),
            region: spec.region.clone(),
            state: ProviderNodeState::Pending,
            ipv4: Some(format!("198.51.100.{}", self.next_id % 250 + 1)),
            ipv6: None,
            labels: spec.labels.clone(),
            created_at_ms: now_ms,
        };
        self.by_request
            .insert(spec.request_id.clone(), provider_id.clone());
        self.nodes.insert(provider_id, node.clone());
        Ok(node)
    }

    fn destroy_node(&mut self, provider_id: &str) -> Result<(), ProviderError> {
        self.calls.push(format!("destroy:{provider_id}"));
        self.check_outage()?;
        if let Some(e) = pop_fault(&mut self.faults.destroy) {
            return Err(e);
        }
        // Idempotent: destroying an absent node succeeds.
        self.nodes.remove(provider_id);
        self.firewall.remove(provider_id);
        self.by_request.retain(|_, id| id != provider_id);
        Ok(())
    }

    fn get_node(&self, provider_id: &str) -> Result<ProviderNode, ProviderError> {
        self.check_outage()?;
        self.nodes
            .get(provider_id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(provider_id.into()))
    }

    fn list_nodes(&self) -> Result<Vec<ProviderNode>, ProviderError> {
        self.check_outage()?;
        Ok(self.nodes.values().cloned().collect())
    }

    fn assign_metadata(
        &mut self,
        provider_id: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<(), ProviderError> {
        self.check_outage()?;
        let node = self
            .nodes
            .get_mut(provider_id)
            .ok_or_else(|| ProviderError::NotFound(provider_id.into()))?;
        node.labels
            .extend(labels.iter().map(|(k, v)| (k.clone(), v.clone())));
        Ok(())
    }

    fn configure_firewall(
        &mut self,
        provider_id: &str,
        rules: &[PortRule],
    ) -> Result<(), ProviderError> {
        self.calls.push(format!("firewall:{provider_id}"));
        self.check_outage()?;
        if let Some(e) = pop_fault(&mut self.faults.firewall) {
            return Err(e);
        }
        if !self.nodes.contains_key(provider_id) {
            return Err(ProviderError::NotFound(provider_id.into()));
        }
        // Replace, never append: re-applying is idempotent.
        self.firewall.insert(provider_id.into(), rules.to_vec());
        Ok(())
    }

    fn wait_until_ready(
        &mut self,
        provider_id: &str,
        _deadline_ms: TimestampMs,
    ) -> Result<Readiness, ProviderError> {
        self.check_outage()?;
        let node = self
            .nodes
            .get_mut(provider_id)
            .ok_or_else(|| ProviderError::NotFound(provider_id.into()))?;
        if self.faults.ready_timeouts > 0 {
            self.faults.ready_timeouts -= 1;
            return Ok(Readiness::TimedOut);
        }
        node.state = ProviderNodeState::Running;
        Ok(Readiness::Ready)
    }

    fn replace_ip(&mut self, provider_id: &str) -> Result<IpReplacement, ProviderError> {
        self.check_outage()?;
        if !self.supports_ip_replacement {
            return Ok(IpReplacement::Unsupported);
        }
        self.next_id += 1;
        let node = self
            .nodes
            .get_mut(provider_id)
            .ok_or_else(|| ProviderError::NotFound(provider_id.into()))?;
        node.ipv4 = Some(format!("203.0.113.{}", self.next_id % 250 + 1));
        Ok(IpReplacement::Replaced {
            ipv4: node.ipv4.clone(),
            ipv6: node.ipv6.clone(),
        })
    }
}

/// Behavioural contract every driver must satisfy. Real drivers run this
/// against a disposable test project/account, never production.
pub mod contract {
    use super::*;
    use crate::transport::L4Protocol;

    pub fn spec(request_id: &str, node_id: &str) -> NodeSpec {
        NodeSpec {
            request_id: request_id.into(),
            node_id: node_id.into(),
            region: "test-region".into(),
            size: "small".into(),
            image: "almalinux-9".into(),
            ssh_key_ref: "operator-key".into(),
            user_data: "#cloud-config\n".into(),
            labels: [("managed-by".to_string(), "singbox-vpn".to_string())].into(),
        }
    }

    /// Returns a description of the first violated rule.
    pub fn run(driver: &mut dyn ProviderDriver) -> Result<(), String> {
        let a = driver
            .create_node(&spec("req-1", "contract-a"), 1)
            .map_err(|e| format!("create: {e}"))?;
        let again = driver
            .create_node(&spec("req-1", "contract-a"), 2)
            .map_err(|e| format!("idempotent create: {e}"))?;
        if again.provider_id != a.provider_id {
            return Err("create with the same request_id must be idempotent".into());
        }
        if driver
            .wait_until_ready(&a.provider_id, 1_000)
            .map_err(|e| e.to_string())?
            != Readiness::Ready
        {
            return Err("fresh node did not become ready".into());
        }
        let got = driver.get_node(&a.provider_id).map_err(|e| e.to_string())?;
        if got.state != ProviderNodeState::Running || got.node_id != "contract-a" {
            return Err("get_node must reflect readiness and node id".into());
        }
        let rules = [
            PortRule {
                l4: L4Protocol::Tcp,
                port: 443,
            },
            PortRule {
                l4: L4Protocol::Udp,
                port: 51820,
            },
        ];
        driver
            .configure_firewall(&a.provider_id, &rules)
            .map_err(|e| e.to_string())?;
        driver
            .configure_firewall(&a.provider_id, &rules)
            .map_err(|e| format!("firewall re-apply: {e}"))?;
        driver
            .assign_metadata(
                &a.provider_id,
                &[("role".to_string(), "exit".to_string())].into(),
            )
            .map_err(|e| e.to_string())?;
        if driver
            .get_node(&a.provider_id)
            .map_err(|e| e.to_string())?
            .labels
            .get("role")
            .map(String::as_str)
            != Some("exit")
        {
            return Err("assign_metadata must be visible via get_node".into());
        }
        if !driver
            .list_nodes()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|n| n.provider_id == a.provider_id)
        {
            return Err("list_nodes must include created nodes".into());
        }
        match driver.get_node("does-not-exist") {
            Err(ProviderError::NotFound(_)) => {}
            other => return Err(format!("unknown node must be NotFound, got {other:?}")),
        }
        driver
            .destroy_node(&a.provider_id)
            .map_err(|e| format!("destroy: {e}"))?;
        driver
            .destroy_node(&a.provider_id)
            .map_err(|e| format!("destroy must be idempotent: {e}"))?;
        if !matches!(
            driver.get_node(&a.provider_id),
            Err(ProviderError::NotFound(_))
        ) {
            return Err("destroyed node must be NotFound".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_provider_passes_the_driver_contract() {
        contract::run(&mut MockProvider::new()).unwrap();
    }

    #[test]
    fn token_is_never_debug_printed() {
        let t = ProviderToken::new("super-secret-api-token");
        assert!(!format!("{t:?}").contains("super-secret"));
    }

    #[test]
    fn faults_and_quota() {
        let mut p = MockProvider::new();
        p.faults.create = vec![ProviderError::RateLimited { retry_after_ms: 5 }];
        let err = p.create_node(&contract::spec("r", "n"), 0).unwrap_err();
        assert!(err.is_transient());
        p.quota = Some(1);
        p.create_node(&contract::spec("r", "n"), 0).unwrap();
        assert_eq!(
            p.create_node(&contract::spec("r2", "n2"), 0),
            Err(ProviderError::QuotaExceeded)
        );
        p.faults.outage = true;
        assert_eq!(p.list_nodes(), Err(ProviderError::Unavailable));
        assert!(!ProviderError::AuthFailed.is_transient());
    }

    #[test]
    fn ip_replacement_is_optional() {
        let mut p = MockProvider::new();
        let n = p.create_node(&contract::spec("r", "n"), 0).unwrap();
        assert_eq!(p.replace_ip(&n.provider_id), Ok(IpReplacement::Unsupported));
        p.supports_ip_replacement = true;
        assert!(matches!(
            p.replace_ip(&n.provider_id),
            Ok(IpReplacement::Replaced { .. })
        ));
    }
}
