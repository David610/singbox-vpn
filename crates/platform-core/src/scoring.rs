//! Deterministic, explainable route scoring.
//!
//! Every component is a bounded integer with a machine-readable reason,
//! so a decision can be replayed, diffed and explained (chosen because of
//! a recent successful tunnel and low handshake latency despite moderate RTT).
//! There is no learning and no randomness; the same inputs always give
//! the same ranking, which is what makes golden-vector conformance tests
//! across implementations possible.

use crate::failure::FailureClass;
use crate::health::{HealthPolicy, HealthStore, RouteHealth};
use crate::route::{Route, RouteCatalog};
use crate::transport::{CpuCost, L4Protocol, TransportKind};
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoringPolicy {
    /// Engine version tag, bumped whenever weights or rules change so
    /// telemetry and golden vectors can be matched to the rules used.
    pub version: String,
    pub health: HealthPolicy,
    /// A success newer than this earns the freshness bonus.
    pub fresh_success_ms: u64,
    pub fresh_success_bonus: i32,
    /// Points per fully weighted (1000 permille) usable observation.
    pub success_per_unit: i32,
    pub success_cap: i32,
    pub failure_per_unit: i32,
    pub failure_cap: i32,
    /// Extra multiplier (in percent) for authentication failures, which
    /// retries do not fix.
    pub auth_failure_percent: i32,
    pub consecutive_failure_step: i32,
    pub consecutive_failure_cap: i32,
    pub stability_min_streak: u32,
    pub stability_step: i32,
    pub stability_cap: i32,
    pub transfer_ok_bonus: i32,
    pub transfer_failed_penalty: i32,
    pub draining_penalty: i32,
    pub shared_node_failure_penalty: i32,
    /// Decayed weight (permille) of `UdpUnreachable` on this network that
    /// marks the network as UDP-hostile for all UDP routes.
    pub udp_hostile_threshold: u32,
    pub udp_hostile_penalty: i32,
    /// Declared per-transport preference, in points.
    pub transport_preference: BTreeMap<String, i32>,
}

impl Default for ScoringPolicy {
    fn default() -> Self {
        ScoringPolicy {
            version: "route-engine/1".into(),
            health: HealthPolicy::default(),
            fresh_success_ms: 5 * 60 * 1000,
            fresh_success_bonus: 30,
            success_per_unit: 20,
            success_cap: 80,
            failure_per_unit: 30,
            failure_cap: 150,
            auth_failure_percent: 300,
            consecutive_failure_step: 15,
            consecutive_failure_cap: 90,
            stability_min_streak: 3,
            stability_step: 2,
            stability_cap: 20,
            transfer_ok_bonus: 20,
            transfer_failed_penalty: 40,
            draining_penalty: 60,
            shared_node_failure_penalty: 40,
            udp_hostile_threshold: 500,
            udp_hostile_penalty: 60,
            transport_preference: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    OperatorPriority,
    Unmeasured,
    RecentSuccess,
    SuccessHistory,
    FailureHistory,
    AuthFailures,
    ConsecutiveFailures,
    HandshakeLatency,
    Rtt,
    Jitter,
    Loss,
    SustainedTransferOk,
    SustainedTransferFailed,
    Stability,
    NodeDraining,
    SharedNodeFailure,
    UdpHostileNetwork,
    CpuCost,
    TransportPreference,
}

impl ReasonKind {
    fn text(self) -> &'static str {
        match self {
            ReasonKind::OperatorPriority => "operator priority",
            ReasonKind::Unmeasured => "no measurements on this network yet",
            ReasonKind::RecentSuccess => "recent successful tunnel",
            ReasonKind::SuccessHistory => "success history",
            ReasonKind::FailureHistory => "failure history",
            ReasonKind::AuthFailures => "authentication failures",
            ReasonKind::ConsecutiveFailures => "consecutive failures",
            ReasonKind::HandshakeLatency => "handshake latency",
            ReasonKind::Rtt => "RTT",
            ReasonKind::Jitter => "jitter",
            ReasonKind::Loss => "packet loss",
            ReasonKind::SustainedTransferOk => "stable transfer test",
            ReasonKind::SustainedTransferFailed => "failed transfer test",
            ReasonKind::Stability => "stability streak",
            ReasonKind::NodeDraining => "node draining",
            ReasonKind::SharedNodeFailure => "same node failing on another transport",
            ReasonKind::UdpHostileNetwork => "UDP unreachable on this network",
            ReasonKind::CpuCost => "declared CPU cost",
            ReasonKind::TransportPreference => "transport preference",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    pub kind: ReasonKind,
    pub points: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteScore {
    pub route_id: String,
    pub total: i32,
    pub reasons: Vec<Reason>,
}

impl RouteScore {
    /// Operator-facing explanation. Not shown to end users by default.
    pub fn explain(&self) -> String {
        let mut out = format!("{} = {}", self.route_id, self.total);
        for r in &self.reasons {
            if r.points == 0 && r.kind != ReasonKind::Unmeasured {
                continue;
            }
            let sign = if r.points >= 0 { '+' } else { '-' };
            out.push_str(&format!("\n  {sign} {} ({})", r.kind.text(), r.points));
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub policy_version: String,
    pub network_id: String,
    pub at_ms: TimestampMs,
    /// Best first. Ties break by operator priority, then route id.
    pub ranked: Vec<RouteScore>,
}

impl RouteDecision {
    pub fn best(&self) -> Option<&RouteScore> {
        self.ranked.first()
    }

    pub fn score_of(&self, route_id: &str) -> Option<i32> {
        self.ranked
            .iter()
            .find(|s| s.route_id == route_id)
            .map(|s| s.total)
    }
}

/// Rank `candidates` (already filtered for servability, client support and
/// session mode) on `network_id`.
pub fn rank(
    catalog: &RouteCatalog,
    candidates: &[&Route],
    store: &HealthStore,
    network_id: &str,
    now_ms: TimestampMs,
    policy: &ScoringPolicy,
) -> RouteDecision {
    let udp_hostile = network_is_udp_hostile(store, network_id, now_ms, policy);
    let mut ranked: Vec<(u8, RouteScore)> = candidates
        .iter()
        .map(|route| {
            let score = score_route(
                catalog,
                route,
                store,
                network_id,
                now_ms,
                udp_hostile,
                policy,
            );
            (route.priority, score)
        })
        .collect();
    ranked.sort_by(|(pa, a), (pb, b)| {
        b.total
            .cmp(&a.total)
            .then(pb.cmp(pa))
            .then_with(|| a.route_id.cmp(&b.route_id))
    });
    RouteDecision {
        policy_version: policy.version.clone(),
        network_id: network_id.to_string(),
        at_ms: now_ms,
        ranked: ranked.into_iter().map(|(_, s)| s).collect(),
    }
}

fn network_is_udp_hostile(
    store: &HealthStore,
    network_id: &str,
    now_ms: TimestampMs,
    policy: &ScoringPolicy,
) -> bool {
    let weight: u32 = store
        .routes_on(network_id)
        .map(|h| h.decayed_failures_of(FailureClass::UdpUnreachable, now_ms, &policy.health))
        .sum();
    weight >= policy.udp_hostile_threshold
}

fn score_route(
    catalog: &RouteCatalog,
    route: &Route,
    store: &HealthStore,
    network_id: &str,
    now_ms: TimestampMs,
    udp_hostile: bool,
    policy: &ScoringPolicy,
) -> RouteScore {
    let mut reasons = vec![Reason {
        kind: ReasonKind::OperatorPriority,
        points: route.priority as i32,
    }];
    let entry = catalog.entry_transport(route);
    let entry_caps = entry.and_then(TransportKind::capabilities);

    match store.get(network_id, &route.route_id) {
        Some(h) if !h.is_stale(now_ms, &policy.health) => {
            health_reasons(h, now_ms, policy, &mut reasons);
        }
        _ => reasons.push(Reason {
            kind: ReasonKind::Unmeasured,
            points: 0,
        }),
    }

    if catalog.is_draining(route) {
        reasons.push(Reason {
            kind: ReasonKind::NodeDraining,
            points: -policy.draining_penalty,
        });
    }

    let shared_failure = catalog.routes_sharing_node(route).any(|other| {
        store.get(network_id, &other.route_id).is_some_and(|h| {
            [FailureClass::EndpointDown, FailureClass::ServerOverloaded]
                .into_iter()
                .any(|c| h.decayed_failures_of(c, now_ms, &policy.health) >= 500)
        })
    });
    if shared_failure {
        reasons.push(Reason {
            kind: ReasonKind::SharedNodeFailure,
            points: -policy.shared_node_failure_penalty,
        });
    }

    if udp_hostile && entry_caps.as_ref().is_some_and(|c| c.l4 == L4Protocol::Udp) {
        reasons.push(Reason {
            kind: ReasonKind::UdpHostileNetwork,
            points: -policy.udp_hostile_penalty,
        });
    }

    if let Some(caps) = &entry_caps {
        let points = match caps.relative_cpu_cost {
            CpuCost::Low => 3,
            CpuCost::Medium => 0,
            CpuCost::High => -3,
        };
        reasons.push(Reason {
            kind: ReasonKind::CpuCost,
            points,
        });
    }
    if let Some(points) = entry.and_then(|t| policy.transport_preference.get(t.as_str())) {
        reasons.push(Reason {
            kind: ReasonKind::TransportPreference,
            points: *points,
        });
    }

    let total = reasons.iter().map(|r| r.points).sum();
    RouteScore {
        route_id: route.route_id.clone(),
        total,
        reasons,
    }
}

fn health_reasons(
    h: &RouteHealth,
    now_ms: TimestampMs,
    policy: &ScoringPolicy,
    out: &mut Vec<Reason>,
) {
    let hp = &policy.health;
    let (ok, bad) = h.decayed_counts(now_ms, hp);
    let auth = h.decayed_failures_of(FailureClass::AuthFailure, now_ms, hp);

    if h.last_success_ms
        .is_some_and(|t| now_ms.saturating_sub(t) <= policy.fresh_success_ms)
    {
        out.push(Reason {
            kind: ReasonKind::RecentSuccess,
            points: policy.fresh_success_bonus,
        });
    }
    let success =
        (ok as i64 * policy.success_per_unit as i64 / 1000).min(policy.success_cap as i64) as i32;
    if success != 0 {
        out.push(Reason {
            kind: ReasonKind::SuccessHistory,
            points: success,
        });
    }
    let non_auth_bad = bad.saturating_sub(auth);
    let failure = (non_auth_bad as i64 * policy.failure_per_unit as i64 / 1000)
        .min(policy.failure_cap as i64) as i32;
    if failure != 0 {
        out.push(Reason {
            kind: ReasonKind::FailureHistory,
            points: -failure,
        });
    }
    let auth_points =
        (auth as i64 * policy.failure_per_unit as i64 * policy.auth_failure_percent as i64
            / 100_000)
            .min(policy.failure_cap as i64) as i32;
    if auth_points != 0 {
        out.push(Reason {
            kind: ReasonKind::AuthFailures,
            points: -auth_points,
        });
    }
    let consecutive = (h.consecutive_failures as i32 * policy.consecutive_failure_step)
        .min(policy.consecutive_failure_cap);
    if consecutive != 0 {
        out.push(Reason {
            kind: ReasonKind::ConsecutiveFailures,
            points: -consecutive,
        });
    }

    if let Some(ms) = h.median_handshake_ms() {
        out.push(Reason {
            kind: ReasonKind::HandshakeLatency,
            points: bucket(ms, &[(150, 0), (400, -5), (1000, -15), (3000, -30)], -50),
        });
    }
    if let Some(ms) = h.median_rtt_ms() {
        out.push(Reason {
            kind: ReasonKind::Rtt,
            points: bucket(ms, &[(50, 0), (120, -5), (250, -15), (500, -30)], -50),
        });
    }
    if let Some(ms) = h.mean_jitter_ms() {
        out.push(Reason {
            kind: ReasonKind::Jitter,
            points: bucket(ms, &[(10, 0), (30, -5), (80, -15)], -25),
        });
    }
    if let Some(pm) = h.mean_loss_permille() {
        out.push(Reason {
            kind: ReasonKind::Loss,
            points: bucket(pm, &[(5, 0), (20, -10), (50, -25)], -50),
        });
    }

    match (h.last_transfer_ok_ms, h.last_transfer_failed_ms) {
        (Some(ok_at), failed) if failed.is_none_or(|f| ok_at >= f) => {
            out.push(Reason {
                kind: ReasonKind::SustainedTransferOk,
                points: policy.transfer_ok_bonus,
            });
        }
        (_, Some(_)) => {
            out.push(Reason {
                kind: ReasonKind::SustainedTransferFailed,
                points: -policy.transfer_failed_penalty,
            });
        }
        _ => {}
    }

    if h.consecutive_successes >= policy.stability_min_streak {
        let points =
            (h.consecutive_successes as i32 * policy.stability_step).min(policy.stability_cap);
        out.push(Reason {
            kind: ReasonKind::Stability,
            points,
        });
    }
}

fn bucket(value: u32, steps: &[(u32, i32)], beyond: i32) -> i32 {
    steps
        .iter()
        .find(|(limit, _)| value <= *limit)
        .map(|(_, points)| *points)
        .unwrap_or(beyond)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::test_support::{failure, success};
    use crate::health::{HealthLayer, LayerResult};
    use crate::route::test_support::two_node_catalog;
    use crate::route::NodeStatus;

    fn all_routes(c: &RouteCatalog) -> Vec<&Route> {
        c.routes.iter().filter(|r| r.hops.len() == 1).collect()
    }

    #[test]
    fn with_no_measurements_operator_priority_and_cpu_cost_decide() {
        let c = two_node_catalog();
        let d = rank(
            &c,
            &all_routes(&c),
            &HealthStore::default(),
            "net-a",
            0,
            &ScoringPolicy::default(),
        );
        assert_eq!(d.best().unwrap().route_id, "fi1/awg");
        assert!(d
            .ranked
            .iter()
            .all(|s| s.reasons.iter().any(|r| r.kind == ReasonKind::Unmeasured)));
    }

    #[test]
    fn measured_success_beats_higher_priority_failure() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        for t in 0..3 {
            store.record(
                &failure(
                    "fi1/awg",
                    t,
                    HealthLayer::Handshake,
                    FailureClass::UdpUnreachable,
                ),
                &p.health,
            );
            store.record(&success("nl1/reality", t, 60), &p.health);
        }
        let d = rank(&c, &all_routes(&c), &store, "net-a", 10, &p);
        assert_eq!(d.best().unwrap().route_id, "nl1/reality");
        let awg = d.ranked.iter().find(|s| s.route_id == "fi1/awg").unwrap();
        assert!(awg
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::FailureHistory && r.points < 0));
    }

    #[test]
    fn udp_hostile_network_penalises_every_udp_route_not_just_the_failed_one() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        store.record(
            &failure(
                "fi1/awg",
                0,
                HealthLayer::Handshake,
                FailureClass::UdpUnreachable,
            ),
            &p.health,
        );
        let d = rank(&c, &all_routes(&c), &store, "net-a", 1, &p);
        for id in ["nl1/awg", "nl1/hy2", "fi1/hy2"] {
            let s = d.ranked.iter().find(|s| s.route_id == id).unwrap();
            assert!(
                s.reasons
                    .iter()
                    .any(|r| r.kind == ReasonKind::UdpHostileNetwork),
                "{id}"
            );
        }
        let tcp = d
            .ranked
            .iter()
            .find(|s| s.route_id == "nl1/reality")
            .unwrap();
        assert!(!tcp
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::UdpHostileNetwork));
    }

    #[test]
    fn udp_penalty_is_per_network() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        store.record(
            &failure(
                "fi1/awg",
                0,
                HealthLayer::Handshake,
                FailureClass::UdpUnreachable,
            ),
            &p.health,
        );
        let d = rank(&c, &all_routes(&c), &store, "net-b", 1, &p);
        assert!(d.ranked.iter().all(|s| !s
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::UdpHostileNetwork)));
    }

    #[test]
    fn a_single_handshake_failure_does_not_spread_to_the_same_node() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        store.record(
            &failure(
                "fi1/reality",
                0,
                HealthLayer::Handshake,
                FailureClass::HandshakeFailure,
            ),
            &p.health,
        );
        let d = rank(&c, &all_routes(&c), &store, "net-a", 1, &p);
        let awg = d.ranked.iter().find(|s| s.route_id == "fi1/awg").unwrap();
        assert!(!awg
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::SharedNodeFailure));
    }

    #[test]
    fn endpoint_down_spreads_to_routes_on_the_same_node() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        store.record(
            &failure(
                "fi1/reality",
                0,
                HealthLayer::Reachability,
                FailureClass::EndpointDown,
            ),
            &p.health,
        );
        let d = rank(&c, &all_routes(&c), &store, "net-a", 1, &p);
        let awg = d.ranked.iter().find(|s| s.route_id == "fi1/awg").unwrap();
        assert!(awg
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::SharedNodeFailure));
        let nl = d.ranked.iter().find(|s| s.route_id == "nl1/awg").unwrap();
        assert!(!nl
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::SharedNodeFailure));
    }

    #[test]
    fn auth_failures_weigh_more_than_ordinary_failures() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut a = HealthStore::default();
        let mut b = HealthStore::default();
        a.record(
            &failure(
                "fi1/reality",
                0,
                HealthLayer::Handshake,
                FailureClass::AuthFailure,
            ),
            &p.health,
        );
        b.record(
            &failure(
                "fi1/reality",
                0,
                HealthLayer::Handshake,
                FailureClass::HandshakeFailure,
            ),
            &p.health,
        );
        let routes = all_routes(&c);
        let sa = rank(&c, &routes, &a, "net-a", 1, &p)
            .score_of("fi1/reality")
            .unwrap();
        let sb = rank(&c, &routes, &b, "net-a", 1, &p)
            .score_of("fi1/reality")
            .unwrap();
        assert!(sa < sb);
    }

    #[test]
    fn draining_nodes_are_penalised() {
        let mut c = two_node_catalog();
        c.nodes[0].status = NodeStatus::Draining;
        let d = rank(
            &c,
            &all_routes(&c),
            &HealthStore::default(),
            "net-a",
            0,
            &ScoringPolicy::default(),
        );
        assert!(d.best().unwrap().route_id.starts_with("nl1"));
    }

    #[test]
    fn latency_and_transfer_are_explained() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        let mut obs = success("nl1/reality", 0, 180);
        obs.layers.push(LayerResult {
            layer: HealthLayer::Transfer,
            ok: true,
            duration_ms: None,
        });
        store.record(&obs, &p.health);
        let d = rank(&c, &all_routes(&c), &store, "net-a", 1, &p);
        let s = d
            .ranked
            .iter()
            .find(|s| s.route_id == "nl1/reality")
            .unwrap();
        let text = s.explain();
        assert!(text.contains("+ recent successful tunnel"), "{text}");
        assert!(text.contains("+ stable transfer test"), "{text}");
        assert!(text.contains("- RTT"), "{text}");
    }

    #[test]
    fn ranking_is_deterministic_and_ties_break_by_priority_then_id() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let routes = all_routes(&c);
        let a = rank(&c, &routes, &HealthStore::default(), "n", 0, &p);
        let mut reversed = routes.clone();
        reversed.reverse();
        let b = rank(&c, &reversed, &HealthStore::default(), "n", 0, &p);
        assert_eq!(a, b);
    }

    #[test]
    fn stale_health_is_ignored() {
        let c = two_node_catalog();
        let p = ScoringPolicy::default();
        let mut store = HealthStore::default();
        store.record(
            &failure(
                "fi1/awg",
                0,
                HealthLayer::Handshake,
                FailureClass::HandshakeFailure,
            ),
            &p.health,
        );
        let later = p.health.ttl_ms + 1;
        let d = rank(&c, &all_routes(&c), &store, "net-a", later, &p);
        assert_eq!(d.best().unwrap().route_id, "fi1/awg");
    }
}
