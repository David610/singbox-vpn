//! Node lifecycle state machine.
//!
//! The one invariant that matters most: **a node is never served to
//! clients until its health gates have passed**, and a node that is being
//! replaced keeps serving until its successor is verified and published.

use crate::route::NodeStatus;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Requested,
    Creating,
    WaitingReady,
    Bootstrapping,
    Verifying,
    Registered,
    Active,
    Draining,
    Retired,
    Destroyed,
    /// Terminal for this attempt; the node is never served.
    Failed,
    /// Kept for investigation, never served, not yet destroyed.
    Quarantined,
}

impl LifecycleState {
    /// The route-catalog status clients see, or `None` when the node must
    /// not appear in any catalog.
    pub fn catalog_status(self) -> Option<NodeStatus> {
        match self {
            LifecycleState::Active => Some(NodeStatus::Active),
            LifecycleState::Draining => Some(NodeStatus::Draining),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, LifecycleState::Destroyed | LifecycleState::Failed)
    }

    fn allowed_next(self) -> &'static [LifecycleState] {
        use LifecycleState::*;
        match self {
            Requested => &[Creating, Failed],
            Creating => &[WaitingReady, Failed],
            WaitingReady => &[Bootstrapping, Failed, Quarantined],
            Bootstrapping => &[Verifying, Failed, Quarantined],
            Verifying => &[Registered, Failed, Quarantined],
            Registered => &[Active, Failed, Quarantined],
            Active => &[Draining],
            Draining => &[Retired, Active],
            Retired => &[Destroyed],
            Quarantined => &[Destroyed],
            Failed => &[Destroyed],
            Destroyed => &[],
        }
    }
}

/// What a transition must be justified by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransitionGuard {
    None,
    /// `Verifying -> Registered` requires every health gate to pass.
    HealthGates {
        passed: Vec<String>,
        failed: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub from: LifecycleState,
    pub to: LifecycleState,
    pub at_ms: TimestampMs,
    pub reason: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("illegal transition {from:?} -> {to:?}")]
    Illegal {
        from: LifecycleState,
        to: LifecycleState,
    },
    #[error("health gates not passed: failed {0:?}")]
    GatesFailed(Vec<String>),
    #[error("no health gates were run")]
    NoGates,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLifecycle {
    pub node_id: String,
    pub state: LifecycleState,
    pub history: Vec<Transition>,
}

/// Health gates every node must pass before registration.
pub const REQUIRED_GATES: &[&str] = &[
    "release-verified",
    "firewall-configured",
    "services-active",
    "doctor-protocol",
];

impl NodeLifecycle {
    pub fn new(node_id: impl Into<String>, at_ms: TimestampMs) -> Self {
        NodeLifecycle {
            node_id: node_id.into(),
            state: LifecycleState::Requested,
            history: vec![Transition {
                from: LifecycleState::Requested,
                to: LifecycleState::Requested,
                at_ms,
                reason: "requested".into(),
            }],
        }
    }

    pub fn transition(
        &mut self,
        to: LifecycleState,
        guard: TransitionGuard,
        at_ms: TimestampMs,
        reason: impl Into<String>,
    ) -> Result<(), LifecycleError> {
        if !self.state.allowed_next().contains(&to) {
            return Err(LifecycleError::Illegal {
                from: self.state,
                to,
            });
        }
        if self.state == LifecycleState::Verifying && to == LifecycleState::Registered {
            match &guard {
                TransitionGuard::HealthGates { passed, failed } => {
                    if !failed.is_empty() {
                        return Err(LifecycleError::GatesFailed(failed.clone()));
                    }
                    let missing: Vec<String> = REQUIRED_GATES
                        .iter()
                        .filter(|g| !passed.iter().any(|p| p == *g))
                        .map(|g| g.to_string())
                        .collect();
                    if !missing.is_empty() {
                        return Err(LifecycleError::GatesFailed(missing));
                    }
                }
                TransitionGuard::None => return Err(LifecycleError::NoGates),
            }
        }
        self.history.push(Transition {
            from: self.state,
            to,
            at_ms,
            reason: reason.into(),
        });
        self.state = to;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LifecycleState::*;

    fn gates(passed: &[&str], failed: &[&str]) -> TransitionGuard {
        TransitionGuard::HealthGates {
            passed: passed.iter().map(|s| s.to_string()).collect(),
            failed: failed.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn to_verifying() -> NodeLifecycle {
        let mut n = NodeLifecycle::new("n1", 0);
        for s in [Creating, WaitingReady, Bootstrapping, Verifying] {
            n.transition(s, TransitionGuard::None, 1, "step").unwrap();
        }
        n
    }

    #[test]
    fn happy_path_requires_all_gates() {
        let mut n = to_verifying();
        assert_eq!(
            n.transition(Registered, TransitionGuard::None, 2, "x"),
            Err(LifecycleError::NoGates)
        );
        assert!(matches!(
            n.transition(Registered, gates(&["release-verified"], &[]), 2, "x"),
            Err(LifecycleError::GatesFailed(_))
        ));
        assert!(matches!(
            n.transition(
                Registered,
                gates(REQUIRED_GATES, &["doctor-protocol"]),
                2,
                "x"
            ),
            Err(LifecycleError::GatesFailed(_))
        ));
        n.transition(Registered, gates(REQUIRED_GATES, &[]), 2, "gates passed")
            .unwrap();
        n.transition(Active, TransitionGuard::None, 3, "published")
            .unwrap();
        assert_eq!(n.state.catalog_status(), Some(NodeStatus::Active));
    }

    #[test]
    fn nodes_cannot_skip_verification() {
        let mut n = NodeLifecycle::new("n1", 0);
        for bad in [Active, Registered, Draining] {
            assert!(matches!(
                n.transition(bad, TransitionGuard::None, 1, "x"),
                Err(LifecycleError::Illegal { .. })
            ));
        }
        n.transition(Creating, TransitionGuard::None, 1, "x")
            .unwrap();
        assert!(n.transition(Active, TransitionGuard::None, 1, "x").is_err());
    }

    #[test]
    fn only_active_and_draining_appear_in_catalogs() {
        for s in [
            Requested,
            Creating,
            WaitingReady,
            Bootstrapping,
            Verifying,
            Registered,
            Retired,
            Destroyed,
            Failed,
            Quarantined,
        ] {
            assert_eq!(s.catalog_status(), None, "{s:?}");
        }
        assert_eq!(Draining.catalog_status(), Some(NodeStatus::Draining));
    }

    #[test]
    fn draining_can_be_rolled_back_to_active() {
        let mut n = to_verifying();
        n.transition(Registered, gates(REQUIRED_GATES, &[]), 2, "ok")
            .unwrap();
        n.transition(Active, TransitionGuard::None, 3, "ok")
            .unwrap();
        n.transition(Draining, TransitionGuard::None, 4, "replacement")
            .unwrap();
        n.transition(Active, TransitionGuard::None, 5, "replacement rolled back")
            .unwrap();
        assert_eq!(n.history.last().unwrap().reason, "replacement rolled back");
    }
}
