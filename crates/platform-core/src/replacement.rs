//! Safe automated node replacement.
//!
//! ```text
//! Active ──(sustained, classified, multi-vantage evidence)──► replacement requested
//!   → create → wait ready → bootstrap → verify gates → publish new route
//!   → old node Draining(drain period) → Retired → destroyed
//! ```
//!
//! Guards: one transient failure never triggers replacement; evidence
//! must repeat inside a window from several independent vantage points;
//! a fleet-wide cooldown, a replacement budget and a concurrency cap
//! bound the blast radius; a failed successor is rolled back while the
//! old node keeps serving; a provider outage leaves the old node
//! untouched. Every step is appended to an audit log without secrets.

use crate::failure::FailureClass;
use crate::lifecycle::{LifecycleState, NodeLifecycle, TransitionGuard, REQUIRED_GATES};
use crate::provider::{NodeSpec, ProviderDriver, ProviderError, Readiness};
use crate::route::NodeStatus;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementPolicy {
    pub evidence_window_ms: u64,
    pub min_failures: usize,
    pub min_vantages: usize,
    pub qualifying_classes: Vec<FailureClass>,
    pub fleet_cooldown_ms: u64,
    pub budget_per_period: usize,
    pub budget_period_ms: u64,
    pub max_concurrent: usize,
    pub ready_deadline_ms: u64,
    pub drain_period_ms: u64,
    pub max_provider_retries: u32,
    pub provider_retry_base_ms: u64,
    /// Keep a failed successor for investigation instead of destroying it.
    pub quarantine_failed: bool,
}

impl Default for ReplacementPolicy {
    fn default() -> Self {
        ReplacementPolicy {
            evidence_window_ms: 30 * 60_000,
            min_failures: 6,
            min_vantages: 2,
            qualifying_classes: vec![
                FailureClass::EndpointDown,
                FailureClass::CensorshipSuspected,
            ],
            fleet_cooldown_ms: 6 * 60 * 60_000,
            budget_per_period: 2,
            budget_period_ms: 7 * 24 * 60 * 60_000,
            max_concurrent: 1,
            ready_deadline_ms: 10 * 60_000,
            drain_period_ms: 24 * 60 * 60_000,
            max_provider_retries: 5,
            provider_retry_base_ms: 60_000,
            quarantine_failed: false,
        }
    }
}

/// One classified failure of a node, observed from one vantage point
/// (a probe agent or an aggregated client report).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeEvidence {
    pub node_id: String,
    pub vantage_id: String,
    pub at_ms: TimestampMs,
    pub class: FailureClass,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Replace { failures: usize, vantages: usize },
    Hold { reason: HoldReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    InsufficientEvidence,
    InsufficientVantages,
    FleetCooldown,
    BudgetExhausted,
    ConcurrencyLimit,
    AlreadyReplacing,
    NodeNotActive,
    UnknownNode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    Creating {
        attempts: u32,
        retry_at_ms: TimestampMs,
    },
    WaitingReady {
        provider_id: String,
        since_ms: TimestampMs,
    },
    Bootstrapping {
        provider_id: String,
    },
    Published {
        provider_id: String,
        drain_until_ms: TimestampMs,
    },
    Completed,
    RolledBack {
        reason: String,
    },
    Abandoned {
        reason: String,
    },
}

impl Stage {
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            Stage::Completed | Stage::RolledBack { .. } | Stage::Abandoned { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub id: String,
    pub old_node_id: String,
    pub new_node_id: String,
    pub requested_at_ms: TimestampMs,
    pub stage: Stage,
}

/// Performs the software side of bringing a node up: install the pinned
/// release, run health gates, issue route configuration, and publish or
/// withdraw routes. Implemented over SSH/cloud-init in production and by
/// a fake in tests.
pub trait Bootstrapper {
    /// Returns the health gates that passed and those that failed.
    fn bootstrap(&mut self, node_id: &str, provider_id: &str) -> (Vec<String>, Vec<String>);
    /// Issue credentials and publish the new node's routes to clients.
    fn publish(&mut self, new_node_id: &str, replacing: &str) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub at_ms: TimestampMs,
    pub actor: String,
    pub replacement_id: Option<String>,
    pub node_id: String,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetNode {
    pub lifecycle: NodeLifecycle,
    pub provider_id: Option<String>,
    pub region: String,
    pub size: String,
    pub image: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementOrchestrator {
    pub policy: ReplacementPolicy,
    pub nodes: BTreeMap<String, FleetNode>,
    pub replacements: Vec<Replacement>,
    pub audit: Vec<AuditEvent>,
    started_at: Vec<TimestampMs>,
    counter: u64,
}

impl ReplacementOrchestrator {
    pub fn new(policy: ReplacementPolicy) -> Self {
        ReplacementOrchestrator {
            policy,
            ..Default::default()
        }
    }

    /// Adopt an already-running, verified node (e.g. a manually installed
    /// one) as Active.
    pub fn adopt_active(
        &mut self,
        node_id: &str,
        provider_id: Option<&str>,
        region: &str,
        at_ms: TimestampMs,
    ) {
        let mut lifecycle = NodeLifecycle::new(node_id, at_ms);
        lifecycle.state = LifecycleState::Active;
        self.nodes.insert(
            node_id.into(),
            FleetNode {
                lifecycle,
                provider_id: provider_id.map(str::to_string),
                region: region.into(),
                size: "default".into(),
                image: "almalinux-9".into(),
            },
        );
        self.log(at_ms, "operator", None, node_id, "adopted_active", None);
    }

    pub fn catalog_statuses(&self) -> BTreeMap<String, NodeStatus> {
        self.nodes
            .iter()
            .filter_map(|(id, n)| n.lifecycle.state.catalog_status().map(|s| (id.clone(), s)))
            .collect()
    }

    pub fn audit_jsonl(&self) -> String {
        self.audit
            .iter()
            .map(|e| serde_json::to_string(e).expect("audit events serialize"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn log(
        &mut self,
        at_ms: TimestampMs,
        actor: &str,
        rid: Option<&str>,
        node: &str,
        event: &str,
        detail: Option<String>,
    ) {
        self.audit.push(AuditEvent {
            at_ms,
            actor: actor.into(),
            replacement_id: rid.map(str::to_string),
            node_id: node.into(),
            event: event.into(),
            detail,
        });
    }

    fn active_replacements(&self) -> usize {
        self.replacements
            .iter()
            .filter(|r| !r.stage.is_finished())
            .count()
    }

    /// Decide whether the evidence justifies replacing `node_id` now.
    pub fn evaluate(
        &self,
        node_id: &str,
        evidence: &[NodeEvidence],
        now_ms: TimestampMs,
    ) -> Decision {
        let hold = |reason| Decision::Hold { reason };
        let Some(node) = self.nodes.get(node_id) else {
            return hold(HoldReason::UnknownNode);
        };
        if node.lifecycle.state != LifecycleState::Active {
            return hold(HoldReason::NodeNotActive);
        }
        if self
            .replacements
            .iter()
            .any(|r| r.old_node_id == node_id && !r.stage.is_finished())
        {
            return hold(HoldReason::AlreadyReplacing);
        }
        let relevant: Vec<&NodeEvidence> = evidence
            .iter()
            .filter(|e| {
                e.node_id == node_id
                    && now_ms.saturating_sub(e.at_ms) <= self.policy.evidence_window_ms
                    && e.at_ms <= now_ms
                    && self.policy.qualifying_classes.contains(&e.class)
            })
            .collect();
        let vantages: BTreeSet<&str> = relevant.iter().map(|e| e.vantage_id.as_str()).collect();
        if relevant.len() < self.policy.min_failures {
            return hold(HoldReason::InsufficientEvidence);
        }
        if vantages.len() < self.policy.min_vantages {
            return hold(HoldReason::InsufficientVantages);
        }
        if self
            .started_at
            .last()
            .is_some_and(|t| now_ms.saturating_sub(*t) < self.policy.fleet_cooldown_ms)
        {
            return hold(HoldReason::FleetCooldown);
        }
        let in_period = self
            .started_at
            .iter()
            .filter(|t| now_ms.saturating_sub(**t) < self.policy.budget_period_ms)
            .count();
        if in_period >= self.policy.budget_per_period {
            return hold(HoldReason::BudgetExhausted);
        }
        if self.active_replacements() >= self.policy.max_concurrent {
            return hold(HoldReason::ConcurrencyLimit);
        }
        Decision::Replace {
            failures: relevant.len(),
            vantages: vantages.len(),
        }
    }

    /// Start a replacement if [`evaluate`](Self::evaluate) allows it.
    /// `actor` is "auto" for evidence-driven requests or an operator name.
    pub fn request(
        &mut self,
        node_id: &str,
        evidence: &[NodeEvidence],
        actor: &str,
        now_ms: TimestampMs,
    ) -> Result<String, HoldReason> {
        let (failures, vantages) = match self.evaluate(node_id, evidence, now_ms) {
            Decision::Replace { failures, vantages } => (failures, vantages),
            Decision::Hold { reason } => {
                self.log(
                    now_ms,
                    actor,
                    None,
                    node_id,
                    "replacement_held",
                    Some(format!("{reason:?}")),
                );
                return Err(reason);
            }
        };
        self.counter += 1;
        let id = format!("rep-{}", self.counter);
        let new_node_id = format!("{node_id}-r{}", self.counter);
        let old = &self.nodes[node_id];
        let mut lifecycle = NodeLifecycle::new(&new_node_id, now_ms);
        lifecycle
            .transition(LifecycleState::Creating, TransitionGuard::None, now_ms, &id)
            .expect("requested -> creating");
        let fleet_node = FleetNode {
            lifecycle,
            provider_id: None,
            region: old.region.clone(),
            size: old.size.clone(),
            image: old.image.clone(),
        };
        self.nodes.insert(new_node_id.clone(), fleet_node);
        self.started_at.push(now_ms);
        self.replacements.push(Replacement {
            id: id.clone(),
            old_node_id: node_id.into(),
            new_node_id: new_node_id.clone(),
            requested_at_ms: now_ms,
            stage: Stage::Creating {
                attempts: 0,
                retry_at_ms: now_ms,
            },
        });
        self.log(
            now_ms,
            actor,
            Some(&id),
            node_id,
            "replacement_requested",
            Some(format!(
                "failures={failures} vantages={vantages} successor={new_node_id}"
            )),
        );
        Ok(id)
    }

    /// Advance every unfinished replacement by at most one stage.
    pub fn step(
        &mut self,
        now_ms: TimestampMs,
        provider: &mut dyn ProviderDriver,
        boot: &mut dyn Bootstrapper,
    ) {
        for idx in 0..self.replacements.len() {
            if self.replacements[idx].stage.is_finished() {
                continue;
            }
            let r = self.replacements[idx].clone();
            let next = self.advance(&r, now_ms, provider, boot);
            self.replacements[idx].stage = next;
        }
    }

    fn set_state(
        &mut self,
        node_id: &str,
        to: LifecycleState,
        guard: TransitionGuard,
        now_ms: TimestampMs,
        reason: &str,
    ) {
        if let Some(n) = self.nodes.get_mut(node_id) {
            if n.lifecycle.transition(to, guard, now_ms, reason).is_err() {
                // Illegal transitions are programming errors; record them
                // rather than panicking in a long-running orchestrator.
                let from = n.lifecycle.state;
                self.log(
                    now_ms,
                    "orchestrator",
                    None,
                    node_id,
                    "illegal_transition",
                    Some(format!("{from:?}->{to:?}")),
                );
            }
        }
    }

    fn advance(
        &mut self,
        r: &Replacement,
        now_ms: TimestampMs,
        provider: &mut dyn ProviderDriver,
        boot: &mut dyn Bootstrapper,
    ) -> Stage {
        let rid = r.id.as_str();
        match &r.stage {
            Stage::Creating {
                attempts,
                retry_at_ms,
            } => {
                if now_ms < *retry_at_ms {
                    return r.stage.clone();
                }
                let fleet = &self.nodes[&r.new_node_id];
                let spec = NodeSpec {
                    request_id: rid.to_string(),
                    node_id: r.new_node_id.clone(),
                    region: fleet.region.clone(),
                    size: fleet.size.clone(),
                    image: fleet.image.clone(),
                    ssh_key_ref: "operator".into(),
                    user_data: String::new(),
                    labels: [("replaces".to_string(), r.old_node_id.clone())].into(),
                };
                match provider.create_node(&spec, now_ms) {
                    Ok(node) => {
                        self.nodes
                            .get_mut(&r.new_node_id)
                            .expect("inserted")
                            .provider_id = Some(node.provider_id.clone());
                        self.set_state(
                            &r.new_node_id,
                            LifecycleState::WaitingReady,
                            TransitionGuard::None,
                            now_ms,
                            rid,
                        );
                        self.log(
                            now_ms,
                            "orchestrator",
                            Some(rid),
                            &r.new_node_id,
                            "node_created",
                            Some(node.provider_id.clone()),
                        );
                        Stage::WaitingReady {
                            provider_id: node.provider_id,
                            since_ms: now_ms,
                        }
                    }
                    Err(e)
                        if e.is_transient() && *attempts + 1 < self.policy.max_provider_retries =>
                    {
                        let delay = self
                            .policy
                            .provider_retry_base_ms
                            .saturating_mul(1 << (*attempts).min(16));
                        self.log(
                            now_ms,
                            "orchestrator",
                            Some(rid),
                            &r.new_node_id,
                            "provider_retry",
                            Some(provider_error_kind(&e)),
                        );
                        Stage::Creating {
                            attempts: attempts + 1,
                            retry_at_ms: now_ms + delay,
                        }
                    }
                    Err(e) => {
                        self.set_state(
                            &r.new_node_id,
                            LifecycleState::Failed,
                            TransitionGuard::None,
                            now_ms,
                            rid,
                        );
                        self.log(
                            now_ms,
                            "orchestrator",
                            Some(rid),
                            &r.old_node_id,
                            "replacement_abandoned",
                            Some(provider_error_kind(&e)),
                        );
                        Stage::Abandoned {
                            reason: provider_error_kind(&e),
                        }
                    }
                }
            }
            Stage::WaitingReady {
                provider_id,
                since_ms,
            } => match provider.wait_until_ready(provider_id, now_ms) {
                Ok(Readiness::Ready) => {
                    self.set_state(
                        &r.new_node_id,
                        LifecycleState::Bootstrapping,
                        TransitionGuard::None,
                        now_ms,
                        rid,
                    );
                    Stage::Bootstrapping {
                        provider_id: provider_id.clone(),
                    }
                }
                Ok(Readiness::TimedOut)
                    if now_ms.saturating_sub(*since_ms) < self.policy.ready_deadline_ms =>
                {
                    r.stage.clone()
                }
                Ok(Readiness::TimedOut) => {
                    self.roll_back(r, provider_id, now_ms, provider, "ready_deadline_exceeded")
                }
                Err(e) if e.is_transient() => r.stage.clone(),
                Err(e) => {
                    self.roll_back(r, provider_id, now_ms, provider, &provider_error_kind(&e))
                }
            },
            Stage::Bootstrapping { provider_id } => {
                let (passed, failed) = boot.bootstrap(&r.new_node_id, provider_id);
                self.set_state(
                    &r.new_node_id,
                    LifecycleState::Verifying,
                    TransitionGuard::None,
                    now_ms,
                    rid,
                );
                let guard = TransitionGuard::HealthGates {
                    passed: passed.clone(),
                    failed: failed.clone(),
                };
                let registered = self
                    .nodes
                    .get_mut(&r.new_node_id)
                    .expect("inserted")
                    .lifecycle
                    .transition(LifecycleState::Registered, guard, now_ms, rid);
                if let Err(e) = registered {
                    self.log(
                        now_ms,
                        "orchestrator",
                        Some(rid),
                        &r.new_node_id,
                        "health_gates_failed",
                        Some(e.to_string()),
                    );
                    return self.roll_back(r, provider_id, now_ms, provider, "health_gates_failed");
                }
                self.log(
                    now_ms,
                    "orchestrator",
                    Some(rid),
                    &r.new_node_id,
                    "health_gates_passed",
                    Some(REQUIRED_GATES.join(",")),
                );
                if let Err(e) = boot.publish(&r.new_node_id, &r.old_node_id) {
                    self.log(
                        now_ms,
                        "orchestrator",
                        Some(rid),
                        &r.new_node_id,
                        "publish_failed",
                        Some(e),
                    );
                    return self.roll_back(r, provider_id, now_ms, provider, "publish_failed");
                }
                self.set_state(
                    &r.new_node_id,
                    LifecycleState::Active,
                    TransitionGuard::None,
                    now_ms,
                    rid,
                );
                self.set_state(
                    &r.old_node_id,
                    LifecycleState::Draining,
                    TransitionGuard::None,
                    now_ms,
                    rid,
                );
                self.log(
                    now_ms,
                    "orchestrator",
                    Some(rid),
                    &r.old_node_id,
                    "draining",
                    None,
                );
                Stage::Published {
                    provider_id: provider_id.clone(),
                    drain_until_ms: now_ms + self.policy.drain_period_ms,
                }
            }
            Stage::Published { drain_until_ms, .. } => {
                if now_ms < *drain_until_ms {
                    return r.stage.clone();
                }
                self.set_state(
                    &r.old_node_id,
                    LifecycleState::Retired,
                    TransitionGuard::None,
                    now_ms,
                    rid,
                );
                let old_provider = self
                    .nodes
                    .get(&r.old_node_id)
                    .and_then(|n| n.provider_id.clone());
                if let Some(pid) = old_provider {
                    match provider.destroy_node(&pid) {
                        Ok(()) => {
                            self.set_state(
                                &r.old_node_id,
                                LifecycleState::Destroyed,
                                TransitionGuard::None,
                                now_ms,
                                rid,
                            );
                            self.log(
                                now_ms,
                                "orchestrator",
                                Some(rid),
                                &r.old_node_id,
                                "destroyed",
                                None,
                            );
                        }
                        Err(e) => {
                            // Retired nodes are never served; destruction can be retried by an operator.
                            self.log(
                                now_ms,
                                "orchestrator",
                                Some(rid),
                                &r.old_node_id,
                                "destroy_failed",
                                Some(provider_error_kind(&e)),
                            );
                        }
                    }
                } else {
                    self.log(
                        now_ms,
                        "orchestrator",
                        Some(rid),
                        &r.old_node_id,
                        "retired_unmanaged",
                        Some("no provider id; destroy manually".into()),
                    );
                }
                Stage::Completed
            }
            finished => finished.clone(),
        }
    }

    fn roll_back(
        &mut self,
        r: &Replacement,
        provider_id: &str,
        now_ms: TimestampMs,
        provider: &mut dyn ProviderDriver,
        reason: &str,
    ) -> Stage {
        let rid = r.id.as_str();
        if self.policy.quarantine_failed {
            self.set_state(
                &r.new_node_id,
                LifecycleState::Quarantined,
                TransitionGuard::None,
                now_ms,
                rid,
            );
        } else {
            self.set_state(
                &r.new_node_id,
                LifecycleState::Failed,
                TransitionGuard::None,
                now_ms,
                rid,
            );
            if let Err(e) = provider.destroy_node(provider_id) {
                self.log(
                    now_ms,
                    "orchestrator",
                    Some(rid),
                    &r.new_node_id,
                    "destroy_failed",
                    Some(provider_error_kind(&e)),
                );
            }
        }
        // The old node was never drained before publish, so it is still Active.
        self.log(
            now_ms,
            "orchestrator",
            Some(rid),
            &r.old_node_id,
            "replacement_rolled_back",
            Some(reason.into()),
        );
        Stage::RolledBack {
            reason: reason.into(),
        }
    }
}

fn provider_error_kind(e: &ProviderError) -> String {
    match e {
        ProviderError::RateLimited { .. } => "rate_limited",
        ProviderError::QuotaExceeded => "quota_exceeded",
        ProviderError::AuthFailed => "auth_failed",
        ProviderError::NotFound(_) => "not_found",
        ProviderError::Unavailable => "unavailable",
        ProviderError::InvalidRequest(_) => "invalid_request",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;

    struct FakeBoot {
        fail_gates: bool,
        fail_publish: bool,
        published: Vec<(String, String)>,
    }

    impl Bootstrapper for FakeBoot {
        fn bootstrap(&mut self, _node: &str, _pid: &str) -> (Vec<String>, Vec<String>) {
            let passed: Vec<String> = REQUIRED_GATES.iter().map(|s| s.to_string()).collect();
            if self.fail_gates {
                (passed[..2].to_vec(), vec!["doctor-protocol".into()])
            } else {
                (passed, vec![])
            }
        }
        fn publish(&mut self, new: &str, old: &str) -> Result<(), String> {
            if self.fail_publish {
                return Err("control plane unavailable".into());
            }
            self.published.push((new.into(), old.into()));
            Ok(())
        }
    }

    fn boot() -> FakeBoot {
        FakeBoot {
            fail_gates: false,
            fail_publish: false,
            published: vec![],
        }
    }

    fn evidence(
        node: &str,
        n: usize,
        vantages: usize,
        at: u64,
        class: FailureClass,
    ) -> Vec<NodeEvidence> {
        (0..n)
            .map(|i| NodeEvidence {
                node_id: node.into(),
                vantage_id: format!("probe-{}", i % vantages),
                at_ms: at.saturating_sub(i as u64),
                class,
            })
            .collect()
    }

    fn fleet() -> (ReplacementOrchestrator, MockProvider) {
        let mut provider = MockProvider::new();
        let old = provider
            .create_node(&crate::provider::contract::spec("seed", "fi1"), 0)
            .unwrap();
        let mut o = ReplacementOrchestrator::new(ReplacementPolicy::default());
        o.adopt_active("fi1", Some(&old.provider_id), "fi", 0);
        (o, provider)
    }

    const T0: u64 = 100 * 24 * 60 * 60_000;

    #[test]
    fn a_single_transient_failure_never_triggers_replacement() {
        let (o, _) = fleet();
        let ev = evidence("fi1", 1, 1, T0, FailureClass::EndpointDown);
        assert_eq!(
            o.evaluate("fi1", &ev, T0 + 10),
            Decision::Hold {
                reason: HoldReason::InsufficientEvidence
            }
        );
    }

    #[test]
    fn evidence_from_a_single_vantage_is_not_enough() {
        let (o, _) = fleet();
        let ev = evidence("fi1", 10, 1, T0, FailureClass::EndpointDown);
        assert_eq!(
            o.evaluate("fi1", &ev, T0 + 10),
            Decision::Hold {
                reason: HoldReason::InsufficientVantages
            }
        );
    }

    #[test]
    fn non_qualifying_classes_and_old_evidence_are_ignored() {
        let (o, _) = fleet();
        let ev = evidence("fi1", 10, 3, T0, FailureClass::HandshakeFailure);
        assert_eq!(
            o.evaluate("fi1", &ev, T0 + 10),
            Decision::Hold {
                reason: HoldReason::InsufficientEvidence
            }
        );
        let ev = evidence("fi1", 10, 3, T0, FailureClass::EndpointDown);
        let late = T0 + ReplacementPolicy::default().evidence_window_ms + 100;
        assert_eq!(
            o.evaluate("fi1", &ev, late),
            Decision::Hold {
                reason: HoldReason::InsufficientEvidence
            }
        );
    }

    #[test]
    fn full_replacement_publishes_drains_and_retires() {
        let (mut o, mut p) = fleet();
        let mut b = boot();
        let ev = evidence("fi1", 6, 2, T0, FailureClass::CensorshipSuspected);
        let id = o.request("fi1", &ev, "auto", T0 + 10).unwrap();
        let mut now = T0 + 10;
        for _ in 0..3 {
            o.step(now, &mut p, &mut b);
            now += 1_000;
        }
        let r = o.replacements.iter().find(|r| r.id == id).unwrap().clone();
        assert!(matches!(r.stage, Stage::Published { .. }), "{:?}", r.stage);
        assert_eq!(
            b.published,
            vec![(r.new_node_id.clone(), "fi1".to_string())]
        );
        let statuses = o.catalog_statuses();
        assert_eq!(statuses.get("fi1"), Some(&NodeStatus::Draining));
        assert_eq!(statuses.get(&r.new_node_id), Some(&NodeStatus::Active));

        o.step(
            now + ReplacementPolicy::default().drain_period_ms,
            &mut p,
            &mut b,
        );
        let r = o.replacements.iter().find(|r| r.id == id).unwrap();
        assert_eq!(r.stage, Stage::Completed);
        assert_eq!(o.nodes["fi1"].lifecycle.state, LifecycleState::Destroyed);
        assert!(!o.catalog_statuses().contains_key("fi1"));
        assert_eq!(
            p.list_nodes().unwrap().len(),
            1,
            "only the successor remains"
        );
    }

    #[test]
    fn failed_health_gates_roll_back_and_keep_the_old_node_active() {
        let (mut o, mut p) = fleet();
        let mut b = FakeBoot {
            fail_gates: true,
            ..boot()
        };
        o.request(
            "fi1",
            &evidence("fi1", 6, 2, T0, FailureClass::EndpointDown),
            "auto",
            T0,
        )
        .unwrap();
        for i in 0..4 {
            o.step(T0 + i, &mut p, &mut b);
        }
        let r = &o.replacements[0];
        assert!(
            matches!(&r.stage, Stage::RolledBack { reason } if reason == "health_gates_failed")
        );
        assert_eq!(o.nodes["fi1"].lifecycle.state, LifecycleState::Active);
        assert_eq!(
            o.nodes[&r.new_node_id].lifecycle.state,
            LifecycleState::Failed
        );
        assert!(b.published.is_empty());
        assert_eq!(
            p.list_nodes().unwrap().len(),
            1,
            "failed successor destroyed"
        );
    }

    #[test]
    fn publish_failure_rolls_back() {
        let (mut o, mut p) = fleet();
        let mut b = FakeBoot {
            fail_publish: true,
            ..boot()
        };
        o.request(
            "fi1",
            &evidence("fi1", 6, 2, T0, FailureClass::EndpointDown),
            "auto",
            T0,
        )
        .unwrap();
        for i in 0..4 {
            o.step(T0 + i, &mut p, &mut b);
        }
        assert!(
            matches!(&o.replacements[0].stage, Stage::RolledBack { reason } if reason == "publish_failed")
        );
        assert_eq!(o.nodes["fi1"].lifecycle.state, LifecycleState::Active);
    }

    #[test]
    fn provider_outage_retries_with_backoff_then_abandons_without_touching_old_node() {
        let (mut o, mut p) = fleet();
        let mut b = boot();
        p.faults.outage = true;
        o.request(
            "fi1",
            &evidence("fi1", 6, 2, T0, FailureClass::EndpointDown),
            "auto",
            T0,
        )
        .unwrap();
        let mut now = T0;
        for _ in 0..20 {
            o.step(now, &mut p, &mut b);
            now += 60 * 60_000;
        }
        assert!(
            matches!(&o.replacements[0].stage, Stage::Abandoned { reason } if reason == "unavailable")
        );
        assert_eq!(o.nodes["fi1"].lifecycle.state, LifecycleState::Active);
        let retries = o
            .audit
            .iter()
            .filter(|e| e.event == "provider_retry")
            .count();
        assert_eq!(retries, 4);
    }

    #[test]
    fn auth_failure_abandons_immediately() {
        let (mut o, mut p) = fleet();
        let mut b = boot();
        p.faults.create = vec![ProviderError::AuthFailed];
        o.request(
            "fi1",
            &evidence("fi1", 6, 2, T0, FailureClass::EndpointDown),
            "auto",
            T0,
        )
        .unwrap();
        o.step(T0, &mut p, &mut b);
        assert!(
            matches!(&o.replacements[0].stage, Stage::Abandoned { reason } if reason == "auth_failed")
        );
    }

    #[test]
    fn ready_deadline_rolls_back() {
        let (mut o, mut p) = fleet();
        let mut b = boot();
        p.faults.ready_timeouts = 1_000;
        o.request(
            "fi1",
            &evidence("fi1", 6, 2, T0, FailureClass::EndpointDown),
            "auto",
            T0,
        )
        .unwrap();
        o.step(T0, &mut p, &mut b);
        o.step(T0 + 1, &mut p, &mut b);
        assert!(matches!(
            o.replacements[0].stage,
            Stage::WaitingReady { .. }
        ));
        o.step(
            T0 + ReplacementPolicy::default().ready_deadline_ms + 5,
            &mut p,
            &mut b,
        );
        assert!(
            matches!(&o.replacements[0].stage, Stage::RolledBack { reason } if reason == "ready_deadline_exceeded")
        );
    }

    #[test]
    fn cooldown_budget_and_concurrency_guards() {
        let (mut o, mut p) = fleet();
        let mut b = FakeBoot {
            fail_gates: true,
            ..boot()
        };
        let prov = p
            .create_node(&crate::provider::contract::spec("seed2", "nl1"), 0)
            .unwrap();
        o.adopt_active("nl1", Some(&prov.provider_id), "nl", 0);
        let ev_fi = evidence("fi1", 6, 2, T0, FailureClass::EndpointDown);
        let ev_nl = evidence("nl1", 6, 2, T0, FailureClass::EndpointDown);

        o.request("fi1", &ev_fi, "auto", T0).unwrap();
        assert_eq!(
            o.request("fi1", &ev_fi, "auto", T0 + 1),
            Err(HoldReason::AlreadyReplacing)
        );
        assert_eq!(
            o.request("nl1", &ev_nl, "auto", T0 + 1),
            Err(HoldReason::FleetCooldown)
        );

        for i in 0..4 {
            o.step(T0 + i, &mut p, &mut b);
        }
        let policy = ReplacementPolicy::default();
        let after_cooldown = T0 + policy.fleet_cooldown_ms + 1;
        let ev_fi = evidence("fi1", 6, 2, after_cooldown - 10, FailureClass::EndpointDown);
        o.request("fi1", &ev_fi, "operator:alice", after_cooldown)
            .unwrap();
        for i in 0..4 {
            o.step(after_cooldown + i, &mut p, &mut b);
        }
        let later = after_cooldown + policy.fleet_cooldown_ms + 1;
        let ev_nl = evidence("nl1", 6, 2, later - 10, FailureClass::EndpointDown);
        assert_eq!(
            o.request("nl1", &ev_nl, "auto", later),
            Err(HoldReason::BudgetExhausted)
        );
        assert!(o.audit.iter().any(
            |e| e.event == "replacement_held" && e.detail.as_deref() == Some("BudgetExhausted")
        ));
    }

    #[test]
    fn audit_log_is_jsonl_without_secrets() {
        let (mut o, mut p) = fleet();
        let mut b = boot();
        o.request(
            "fi1",
            &evidence("fi1", 6, 2, T0, FailureClass::EndpointDown),
            "auto",
            T0,
        )
        .unwrap();
        for i in 0..3 {
            o.step(T0 + i, &mut p, &mut b);
        }
        let jsonl = o.audit_jsonl();
        for line in jsonl.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            for field in ["at_ms", "actor", "node_id", "event"] {
                assert!(v.get(field).is_some(), "{line}");
            }
            for s in v.as_object().unwrap().values().filter_map(|x| x.as_str()) {
                assert!(
                    crate::telemetry::sensitive_shape(s).is_none() || s.starts_with("mock-"),
                    "{s}"
                );
            }
        }
    }
}
