//! Kill-switch policy, decoupled from any OS firewall implementation
//! (spec §21). `KillSwitchBackend` is the seam a real Linux nftables
//! backend (or Windows WFP / macOS pf backend) would implement; only a
//! `MockBackend` exists in this session — see `docs/DEPLOYMENT.md` Phase 9
//! for why the real backend is deferred rather than half-built.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoutePolicy {
    /// A secure (tunneled) route is active: protected traffic is allowed.
    SecureRouteActive,
    /// No secure route: protected traffic must be blocked, never silently
    /// fall back to direct.
    SecureRouteUnavailable,
}

pub trait KillSwitchBackend {
    fn apply(&mut self, policy: RoutePolicy);
    fn current_policy(&self) -> RoutePolicy;
}

pub struct MockBackend {
    policy: RoutePolicy,
    apply_log: Vec<RoutePolicy>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            policy: RoutePolicy::SecureRouteUnavailable,
            apply_log: Vec::new(),
        }
    }

    pub fn apply_log(&self) -> &[RoutePolicy] {
        &self.apply_log
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl KillSwitchBackend for MockBackend {
    fn apply(&mut self, policy: RoutePolicy) {
        self.policy = policy;
        self.apply_log.push(policy);
    }

    fn current_policy(&self) -> RoutePolicy {
        self.policy
    }
}

/// Drives a `KillSwitchBackend` from tunnel-liveness signals. Never allows
/// a silent transition from "secure" to "protected traffic flows direct":
/// the only two states are block-or-allow, and the default (before any
/// signal arrives) is block.
pub struct KillSwitch<B: KillSwitchBackend> {
    backend: B,
}

impl<B: KillSwitchBackend> KillSwitch<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn on_secure_route_established(&mut self) {
        self.backend.apply(RoutePolicy::SecureRouteActive);
    }

    pub fn on_secure_route_lost(&mut self) {
        self.backend.apply(RoutePolicy::SecureRouteUnavailable);
    }

    pub fn policy(&self) -> RoutePolicy {
        self.backend.current_policy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_blocks_before_any_route_established() {
        let ks = KillSwitch::new(MockBackend::new());
        assert_eq!(ks.policy(), RoutePolicy::SecureRouteUnavailable);
    }

    #[test]
    fn route_loss_after_establishment_blocks_again_never_falls_back_direct() {
        let mut ks = KillSwitch::new(MockBackend::new());
        ks.on_secure_route_established();
        assert_eq!(ks.policy(), RoutePolicy::SecureRouteActive);
        ks.on_secure_route_lost();
        assert_eq!(ks.policy(), RoutePolicy::SecureRouteUnavailable);
    }

    #[test]
    fn every_transition_is_recorded_no_silent_state_changes() {
        let mut ks = KillSwitch::new(MockBackend::new());
        ks.on_secure_route_established();
        ks.on_secure_route_lost();
        ks.on_secure_route_established();
        assert_eq!(
            ks.backend.apply_log(),
            &[
                RoutePolicy::SecureRouteActive,
                RoutePolicy::SecureRouteUnavailable,
                RoutePolicy::SecureRouteActive,
            ]
        );
    }
}
