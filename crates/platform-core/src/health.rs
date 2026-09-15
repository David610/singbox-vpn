//! Multi-layer route health.
//!
//! A socket accepting a connection proves L1 only. A route is *usable*
//! only with L4 (Internet reachable through the tunnel) and *proven* only
//! with L5 (sustained transfer) plus L6 (stability over time).
//!
//! All arithmetic is integer and step-decayed so another implementation
//! (the Tamara port) reproduces exactly the same numbers from the same
//! observation sequence.

use crate::failure::FailureClass;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthLayer {
    #[serde(rename = "L1")]
    Reachability,
    #[serde(rename = "L2")]
    Handshake,
    #[serde(rename = "L3")]
    Tunnel,
    #[serde(rename = "L4")]
    Internet,
    #[serde(rename = "L5")]
    Transfer,
    #[serde(rename = "L6")]
    Stability,
}

impl HealthLayer {
    pub const ALL: [HealthLayer; 6] = [
        HealthLayer::Reachability,
        HealthLayer::Handshake,
        HealthLayer::Tunnel,
        HealthLayer::Internet,
        HealthLayer::Transfer,
        HealthLayer::Stability,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            HealthLayer::Reachability => "L1",
            HealthLayer::Handshake => "L2",
            HealthLayer::Tunnel => "L3",
            HealthLayer::Internet => "L4",
            HealthLayer::Transfer => "L5",
            HealthLayer::Stability => "L6",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerResult {
    pub layer: HealthLayer,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
}

/// One attempt or probe against one route, taken on one local network.
///
/// `network_id` is an opaque, locally derived identifier for the access
/// network (for example a salted hash of the gateway identity). It is
/// never exported by telemetry — see `telemetry.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub route_id: String,
    pub network_id: String,
    pub at_ms: TimestampMs,
    pub layers: Vec<LayerResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u32>,
    /// Packet loss in permille (0..=1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_permille: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_kbps: Option<u32>,
}

impl Observation {
    pub fn layer(&self, layer: HealthLayer) -> Option<bool> {
        self.layers.iter().find(|r| r.layer == layer).map(|r| r.ok)
    }

    /// Highest layer that passed with every layer below it also passing
    /// or absent. A gap in the evidence stops the ladder.
    pub fn highest_passed_layer(&self) -> Option<HealthLayer> {
        let mut highest = None;
        for layer in HealthLayer::ALL {
            match self.layer(layer) {
                Some(true) => highest = Some(layer),
                Some(false) => break,
                None => {}
            }
        }
        highest
    }

    /// Internet reachable through the tunnel, and no lower layer failed.
    pub fn is_usable(&self) -> bool {
        self.highest_passed_layer()
            .is_some_and(|l| l >= HealthLayer::Internet)
            && self
                .layers
                .iter()
                .all(|r| r.ok || r.layer > HealthLayer::Internet)
    }

    pub fn is_failure(&self) -> bool {
        self.layers
            .iter()
            .any(|r| !r.ok && r.layer <= HealthLayer::Internet)
    }
}

/// Parameters for aggregation. Defaults are deliberately conservative
/// and are themselves a documented, versioned part of the route engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthPolicy {
    /// Weight halves every `half_life_ms` of age.
    pub half_life_ms: u64,
    /// Observations older than this contribute nothing and are pruned.
    pub ttl_ms: u64,
    /// Samples kept per metric (median/mean window).
    pub sample_window: usize,
}

impl Default for HealthPolicy {
    fn default() -> Self {
        HealthPolicy {
            half_life_ms: 10 * 60 * 1000,
            ttl_ms: 24 * 60 * 60 * 1000,
            sample_window: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Outcome {
    at_ms: TimestampMs,
    usable: bool,
    failure: Option<FailureClass>,
}

/// Aggregated health of one route on one network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteHealth {
    pub route_id: String,
    pub network_id: String,
    outcomes: VecDeque<Outcome>,
    handshake_ms: VecDeque<u32>,
    rtt_ms: VecDeque<u32>,
    jitter_ms: VecDeque<u32>,
    loss_permille: VecDeque<u16>,
    pub last_success_ms: Option<TimestampMs>,
    pub last_failure_ms: Option<TimestampMs>,
    pub last_failure: Option<FailureClass>,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub last_transfer_ok_ms: Option<TimestampMs>,
    pub last_transfer_failed_ms: Option<TimestampMs>,
}

/// Bounded history kept per route for decayed rates.
const OUTCOME_WINDOW: usize = 64;

impl RouteHealth {
    pub fn new(route_id: &str, network_id: &str) -> Self {
        RouteHealth {
            route_id: route_id.to_string(),
            network_id: network_id.to_string(),
            outcomes: VecDeque::new(),
            handshake_ms: VecDeque::new(),
            rtt_ms: VecDeque::new(),
            jitter_ms: VecDeque::new(),
            loss_permille: VecDeque::new(),
            last_success_ms: None,
            last_failure_ms: None,
            last_failure: None,
            consecutive_failures: 0,
            consecutive_successes: 0,
            last_transfer_ok_ms: None,
            last_transfer_failed_ms: None,
        }
    }

    pub fn record(&mut self, obs: &Observation, policy: &HealthPolicy) {
        let usable = obs.is_usable();
        let failed = obs.is_failure();
        // A probe that proved neither success nor failure (e.g. only L1
        // passed and nothing higher was attempted) updates metrics but not
        // the success/failure ledger.
        if usable || failed {
            push_bounded(
                &mut self.outcomes,
                Outcome {
                    at_ms: obs.at_ms,
                    usable,
                    failure: if failed { obs.failure } else { None },
                },
                OUTCOME_WINDOW,
            );
        }
        if usable {
            self.last_success_ms = Some(obs.at_ms);
            self.consecutive_successes = self.consecutive_successes.saturating_add(1);
            self.consecutive_failures = 0;
        } else if failed {
            self.last_failure_ms = Some(obs.at_ms);
            self.last_failure = obs.failure.or(Some(FailureClass::Unknown));
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            self.consecutive_successes = 0;
        }
        match obs.layer(HealthLayer::Transfer) {
            Some(true) => self.last_transfer_ok_ms = Some(obs.at_ms),
            Some(false) => self.last_transfer_failed_ms = Some(obs.at_ms),
            None => {}
        }
        let w = policy.sample_window.max(1);
        if let Some(v) = obs.handshake_ms {
            push_bounded(&mut self.handshake_ms, v, w);
        }
        if let Some(v) = obs.rtt_ms {
            push_bounded(&mut self.rtt_ms, v, w);
        }
        if let Some(v) = obs.jitter_ms {
            push_bounded(&mut self.jitter_ms, v, w);
        }
        if let Some(v) = obs.loss_permille {
            push_bounded(&mut self.loss_permille, v.min(1000), w);
        }
    }

    /// Decayed success and failure weights in permille units.
    pub fn decayed_counts(&self, now_ms: TimestampMs, policy: &HealthPolicy) -> (u32, u32) {
        let mut ok = 0u32;
        let mut bad = 0u32;
        for o in &self.outcomes {
            let w = decay_weight(now_ms.saturating_sub(o.at_ms), policy);
            if o.usable {
                ok += w;
            } else {
                bad += w;
            }
        }
        (ok, bad)
    }

    /// Decayed weight of failures of a specific class.
    pub fn decayed_failures_of(
        &self,
        class: FailureClass,
        now_ms: TimestampMs,
        policy: &HealthPolicy,
    ) -> u32 {
        self.outcomes
            .iter()
            .filter(|o| o.failure == Some(class))
            .map(|o| decay_weight(now_ms.saturating_sub(o.at_ms), policy))
            .sum()
    }

    pub fn median_handshake_ms(&self) -> Option<u32> {
        median(&self.handshake_ms)
    }
    pub fn median_rtt_ms(&self) -> Option<u32> {
        median(&self.rtt_ms)
    }
    pub fn mean_jitter_ms(&self) -> Option<u32> {
        mean(self.jitter_ms.iter().map(|&v| v as u64))
    }
    pub fn mean_loss_permille(&self) -> Option<u32> {
        mean(self.loss_permille.iter().map(|&v| v as u64))
    }

    /// Newest evidence of any kind.
    pub fn last_seen_ms(&self) -> Option<TimestampMs> {
        match (self.last_success_ms, self.last_failure_ms) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn is_stale(&self, now_ms: TimestampMs, policy: &HealthPolicy) -> bool {
        self.last_seen_ms()
            .is_none_or(|t| now_ms.saturating_sub(t) > policy.ttl_ms)
    }
}

/// Step decay: 1000 permille at age 0, halved per elapsed half-life,
/// zero after ten half-lives.
pub fn decay_weight(age_ms: u64, policy: &HealthPolicy) -> u32 {
    let half = policy.half_life_ms.max(1);
    let steps = age_ms / half;
    if steps >= 10 || age_ms > policy.ttl_ms {
        0
    } else {
        1000 >> steps
    }
}

fn push_bounded<T>(q: &mut VecDeque<T>, v: T, cap: usize) {
    q.push_back(v);
    while q.len() > cap {
        q.pop_front();
    }
}

fn median(q: &VecDeque<u32>) -> Option<u32> {
    if q.is_empty() {
        return None;
    }
    let mut v: Vec<u32> = q.iter().copied().collect();
    v.sort_unstable();
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        // Lower-biased integer midpoint keeps results reproducible.
        ((v[n / 2 - 1] as u64 + v[n / 2] as u64) / 2) as u32
    })
}

fn mean(iter: impl Iterator<Item = u64>) -> Option<u32> {
    let (sum, n) = iter.fold((0u64, 0u64), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| (sum / n) as u32)
}

/// Health for every (network, route) pair seen, bounded by TTL pruning.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStore {
    entries: BTreeMap<String, BTreeMap<String, RouteHealth>>,
}

impl HealthStore {
    pub fn record(&mut self, obs: &Observation, policy: &HealthPolicy) {
        self.entries
            .entry(obs.network_id.clone())
            .or_default()
            .entry(obs.route_id.clone())
            .or_insert_with(|| RouteHealth::new(&obs.route_id, &obs.network_id))
            .record(obs, policy);
    }

    pub fn get(&self, network_id: &str, route_id: &str) -> Option<&RouteHealth> {
        self.entries.get(network_id)?.get(route_id)
    }

    pub fn routes_on(&self, network_id: &str) -> impl Iterator<Item = &RouteHealth> {
        self.entries
            .get(network_id)
            .into_iter()
            .flat_map(|m| m.values())
    }

    pub fn prune(&mut self, now_ms: TimestampMs, policy: &HealthPolicy) {
        for routes in self.entries.values_mut() {
            routes.retain(|_, h| !h.is_stale(now_ms, policy));
        }
        self.entries.retain(|_, routes| !routes.is_empty());
    }

    /// Forget everything about routes no longer in the catalog, so
    /// credential rotation or node removal cannot leave phantom health.
    pub fn retain_routes(&mut self, keep: &dyn Fn(&str) -> bool) {
        for routes in self.entries.values_mut() {
            routes.retain(|id, _| keep(id));
        }
        self.entries.retain(|_, routes| !routes.is_empty());
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn layers_up_to(
        highest_ok: Option<HealthLayer>,
        failed: Option<HealthLayer>,
    ) -> Vec<LayerResult> {
        let mut out = Vec::new();
        for layer in HealthLayer::ALL {
            if Some(layer) == failed {
                out.push(LayerResult {
                    layer,
                    ok: false,
                    duration_ms: None,
                });
                break;
            }
            if highest_ok.is_some_and(|h| layer <= h) {
                out.push(LayerResult {
                    layer,
                    ok: true,
                    duration_ms: None,
                });
            }
        }
        out
    }

    pub fn success(route: &str, at: TimestampMs, rtt: u32) -> Observation {
        Observation {
            route_id: route.into(),
            network_id: "net-a".into(),
            at_ms: at,
            layers: layers_up_to(Some(HealthLayer::Internet), None),
            failure: None,
            handshake_ms: Some(rtt * 2),
            rtt_ms: Some(rtt),
            jitter_ms: Some(rtt / 10),
            loss_permille: Some(0),
            transfer_kbps: None,
        }
    }

    pub fn failure(
        route: &str,
        at: TimestampMs,
        layer: HealthLayer,
        class: FailureClass,
    ) -> Observation {
        let below = HealthLayer::ALL
            .iter()
            .copied()
            .take_while(|l| *l < layer)
            .last();
        Observation {
            route_id: route.into(),
            network_id: "net-a".into(),
            at_ms: at,
            layers: layers_up_to(below, Some(layer)),
            failure: Some(class),
            handshake_ms: None,
            rtt_ms: None,
            jitter_ms: None,
            loss_permille: None,
            transfer_kbps: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn l1_alone_is_not_usable() {
        let obs = Observation {
            layers: layers_up_to(Some(HealthLayer::Reachability), None),
            ..success("r", 0, 10)
        };
        assert!(!obs.is_usable());
        assert!(!obs.is_failure());
    }

    #[test]
    fn l4_success_is_usable_even_if_l5_fails() {
        let mut obs = success("r", 0, 10);
        obs.layers.push(LayerResult {
            layer: HealthLayer::Transfer,
            ok: false,
            duration_ms: None,
        });
        assert!(obs.is_usable());
        assert!(!obs.is_failure());
    }

    #[test]
    fn a_failed_lower_layer_stops_the_ladder() {
        let obs = Observation {
            layers: vec![
                LayerResult {
                    layer: HealthLayer::Reachability,
                    ok: true,
                    duration_ms: None,
                },
                LayerResult {
                    layer: HealthLayer::Handshake,
                    ok: false,
                    duration_ms: None,
                },
                LayerResult {
                    layer: HealthLayer::Internet,
                    ok: true,
                    duration_ms: None,
                },
            ],
            ..success("r", 0, 10)
        };
        assert_eq!(obs.highest_passed_layer(), Some(HealthLayer::Reachability));
        assert!(!obs.is_usable());
        assert!(obs.is_failure());
    }

    #[test]
    fn decay_halves_per_half_life_and_hits_zero() {
        let p = HealthPolicy {
            half_life_ms: 100,
            ttl_ms: 10_000,
            sample_window: 8,
        };
        assert_eq!(decay_weight(0, &p), 1000);
        assert_eq!(decay_weight(99, &p), 1000);
        assert_eq!(decay_weight(100, &p), 500);
        assert_eq!(decay_weight(250, &p), 250);
        assert_eq!(decay_weight(1000, &p), 0);
    }

    #[test]
    fn consecutive_counters_and_metrics() {
        let p = HealthPolicy::default();
        let mut h = RouteHealth::new("r", "net-a");
        h.record(&success("r", 0, 40), &p);
        h.record(&success("r", 1, 60), &p);
        h.record(
            &failure(
                "r",
                2,
                HealthLayer::Handshake,
                FailureClass::HandshakeFailure,
            ),
            &p,
        );
        assert_eq!(h.consecutive_failures, 1);
        assert_eq!(h.consecutive_successes, 0);
        assert_eq!(h.median_rtt_ms(), Some(50));
        assert_eq!(h.last_failure, Some(FailureClass::HandshakeFailure));
        let (ok, bad) = h.decayed_counts(2, &p);
        assert_eq!((ok, bad), (2000, 1000));
    }

    #[test]
    fn store_prunes_stale_entries_and_forgets_removed_routes() {
        let p = HealthPolicy {
            half_life_ms: 100,
            ttl_ms: 1000,
            sample_window: 4,
        };
        let mut store = HealthStore::default();
        store.record(&success("a", 0, 10), &p);
        store.record(&success("b", 900, 10), &p);
        store.prune(1500, &p);
        assert!(store.get("net-a", "a").is_none());
        assert!(store.get("net-a", "b").is_some());
        store.retain_routes(&|id| id != "b");
        assert!(store.get("net-a", "b").is_none());
    }

    #[test]
    fn sample_windows_are_bounded() {
        let p = HealthPolicy {
            sample_window: 3,
            ..HealthPolicy::default()
        };
        let mut h = RouteHealth::new("r", "n");
        for (i, rtt) in [10, 20, 30, 1000].into_iter().enumerate() {
            h.record(&success("r", i as u64, rtt), &p);
        }
        assert_eq!(h.median_rtt_ms(), Some(30));
    }
}
