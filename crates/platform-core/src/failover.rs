//! Automatic failover controller.
//!
//! A pure state machine: the host (Tamara, the probe agent, a simulator)
//! feeds it [`Event`]s and executes the [`Action`]s it returns. It never
//! sleeps, spawns or dials anything itself, which is what makes every
//! anti-flapping rule below testable to the millisecond.
//!
//! Guarantees enforced here:
//!
//! * **bounded retries** — per route per episode, and per episode;
//!   exhaustion stops automatic retries until a backoff expires, the
//!   network changes, or the user acts;
//! * **cooldowns** — a failed route is ignored for an exponentially
//!   growing, capped period;
//! * **hysteresis** — a *working* route is only abandoned for a candidate
//!   that is better by a margin for a minimum dwell time;
//! * **anti-flapping** — a cap on switches and recoveries per window;
//! * **probation** — a route that failed and later worked must succeed
//!   repeatedly before it is preferred automatically again;
//! * **generation fencing** — results of superseded attempts are ignored;
//! * **mode invariant** — a Privacy+ session only ever uses relayed routes
//!   and a Fast session only direct routes; there is no silent downgrade;
//! * **safety first** — a detected leak blocks traffic instead of
//!   switching routes silently.
//!
//! Switching nodes changes the public egress address and breaks existing
//! TCP flows. The controller measures recovery time; it does not promise
//! zero interruption.

use crate::failure::FailureClass;
use crate::health::{HealthStore, Observation};
use crate::route::{RouteCatalog, RouteKind};
use crate::scoring::{rank, RouteDecision, ScoringPolicy};
use crate::transport::TransportKind;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailoverPolicy {
    pub max_attempts_per_route: u32,
    pub max_routes_per_episode: u32,
    pub cooldown_base_ms: u64,
    pub cooldown_max_ms: u64,
    pub switch_margin: i32,
    pub min_dwell_ms: u64,
    pub flap_window_ms: u64,
    pub max_switches_in_window: usize,
    pub max_recoveries_in_window: usize,
    pub probation_successes: u32,
    pub exhausted_backoff_ms: u64,
    pub exhausted_backoff_max_ms: u64,
    pub connect_timeout_ms: u64,
    /// Consecutive failed health checks on the current route before a
    /// recovery episode starts. One lost probe is not an outage.
    pub degraded_grace_failures: u32,
}

impl Default for FailoverPolicy {
    fn default() -> Self {
        FailoverPolicy {
            max_attempts_per_route: 1,
            max_routes_per_episode: 4,
            cooldown_base_ms: 10_000,
            cooldown_max_ms: 10 * 60_000,
            switch_margin: 40,
            min_dwell_ms: 60_000,
            flap_window_ms: 10 * 60_000,
            max_switches_in_window: 4,
            max_recoveries_in_window: 3,
            probation_successes: 3,
            exhausted_backoff_ms: 30_000,
            exhausted_backoff_max_ms: 10 * 60_000,
            connect_timeout_ms: 20_000,
            degraded_grace_failures: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Direct routes only.
    Fast,
    /// Relayed routes only; never falls back to direct.
    PrivacyPlus,
}

impl SessionMode {
    fn allows(self, kind: RouteKind) -> bool {
        matches!(
            (self, kind),
            (SessionMode::Fast, RouteKind::Direct) | (SessionMode::PrivacyPlus, RouteKind::Relayed)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    Idle,
    Connecting {
        route_id: String,
        attempt: u64,
        started_ms: TimestampMs,
    },
    Connected {
        route_id: String,
        since_ms: TimestampMs,
    },
    Degraded {
        route_id: String,
        failed_checks: u32,
        since_ms: TimestampMs,
    },
    Exhausted {
        until_ms: TimestampMs,
    },
    /// A safety violation was detected; traffic stays blocked until the
    /// user or host explicitly restarts the session.
    Blocked {
        class: FailureClass,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectReason {
    Initial,
    Failover,
    BetterRoute,
    NetworkChanged,
    CatalogChanged,
    RetryAfterBackoff,
    UserSelected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Connect {
        route_id: String,
        attempt: u64,
        reason: ConnectReason,
    },
    Disconnect,
    /// Keep the kill switch engaged and stop all automatic activity.
    BlockTraffic {
        class: FailureClass,
    },
    Recovered {
        route_id: String,
        recovery_ms: u64,
    },
    Exhausted {
        retry_at_ms: TimestampMs,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Start {
        now_ms: TimestampMs,
    },
    Stop {
        now_ms: TimestampMs,
    },
    /// Result of a `Connect` action. `observation` carries the layers the
    /// engine proved (L4 required for success).
    ConnectResult {
        attempt: u64,
        observation: Observation,
    },
    /// A periodic health probe, of the current route or a background one.
    HealthCheck {
        observation: Observation,
    },
    NetworkChanged {
        network_id: String,
        now_ms: TimestampMs,
    },
    CatalogChanged {
        catalog: RouteCatalog,
        now_ms: TimestampMs,
    },
    UserSelected {
        route_id: Option<String>,
        now_ms: TimestampMs,
    },
    Tick {
        now_ms: TimestampMs,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Cooldown {
    until_ms: TimestampMs,
    strikes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Episode {
    started_ms: TimestampMs,
    /// Whether the episode began by losing a working route (recovery time
    /// is reported) rather than at session start.
    from_outage: bool,
    attempts: BTreeMap<String, u32>,
    excluded: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailoverController {
    policy: FailoverPolicy,
    scoring: ScoringPolicy,
    mode: SessionMode,
    supported: BTreeSet<TransportKind>,
    catalog: RouteCatalog,
    health: HealthStore,
    network_id: String,
    state: State,
    attempt_counter: u64,
    cooldowns: BTreeMap<String, Cooldown>,
    probation: BTreeMap<String, u32>,
    episode: Option<Episode>,
    switches: VecDeque<TimestampMs>,
    recoveries: VecDeque<TimestampMs>,
    exhausted_strikes: u32,
    better_since: Option<(String, TimestampMs)>,
    manual_route: Option<String>,
    last_connected_route: Option<String>,
    last_decision: Option<RouteDecision>,
}

impl FailoverController {
    pub fn new(
        policy: FailoverPolicy,
        scoring: ScoringPolicy,
        mode: SessionMode,
        supported: BTreeSet<TransportKind>,
        catalog: RouteCatalog,
        network_id: impl Into<String>,
    ) -> Self {
        FailoverController {
            policy,
            scoring,
            mode,
            supported,
            catalog,
            health: HealthStore::default(),
            network_id: network_id.into(),
            state: State::Idle,
            attempt_counter: 0,
            cooldowns: BTreeMap::new(),
            probation: BTreeMap::new(),
            episode: None,
            switches: VecDeque::new(),
            recoveries: VecDeque::new(),
            exhausted_strikes: 0,
            better_since: None,
            manual_route: None,
            last_connected_route: None,
            last_decision: None,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn health(&self) -> &HealthStore {
        &self.health
    }

    pub fn last_decision(&self) -> Option<&RouteDecision> {
        self.last_decision.as_ref()
    }

    pub fn is_in_probation(&self, route_id: &str) -> bool {
        self.probation.contains_key(route_id)
    }

    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        match event {
            Event::Start { now_ms } => {
                if !matches!(self.state, State::Idle | State::Blocked { .. }) {
                    return Vec::new();
                }
                self.exhausted_strikes = 0;
                self.begin_episode(now_ms, false);
                self.connect_next(now_ms, ConnectReason::Initial)
            }
            Event::Stop { .. } => {
                self.state = State::Idle;
                self.episode = None;
                self.bump_generation();
                vec![Action::Disconnect]
            }
            Event::ConnectResult {
                attempt,
                observation,
            } => self.on_connect_result(attempt, observation),
            Event::HealthCheck { observation } => self.on_health_check(observation),
            Event::NetworkChanged { network_id, now_ms } => {
                self.on_network_changed(network_id, now_ms)
            }
            Event::CatalogChanged { catalog, now_ms } => self.on_catalog_changed(catalog, now_ms),
            Event::UserSelected { route_id, now_ms } => self.on_user_selected(route_id, now_ms),
            Event::Tick { now_ms } => self.on_tick(now_ms),
        }
    }

    fn bump_generation(&mut self) -> u64 {
        self.attempt_counter += 1;
        self.attempt_counter
    }

    fn begin_episode(&mut self, now_ms: TimestampMs, from_outage: bool) {
        self.episode = Some(Episode {
            started_ms: now_ms,
            from_outage,
            attempts: BTreeMap::new(),
            excluded: BTreeSet::new(),
        });
    }

    fn eligible_routes(&self, now_ms: TimestampMs, ignore_cooldown: bool) -> Vec<String> {
        let supported = |t: &TransportKind| self.supported.contains(t);
        let episode = self.episode.as_ref();
        self.catalog
            .selectable_routes(&supported)
            .filter(|r| self.mode.allows(r.kind))
            .filter(|r| self.manual_route.as_ref().is_none_or(|m| *m == r.route_id))
            .filter(|r| {
                ignore_cooldown
                    || self
                        .cooldowns
                        .get(&r.route_id)
                        .is_none_or(|c| now_ms >= c.until_ms)
            })
            .filter(|r| {
                episode.is_none_or(|e| {
                    !e.excluded.contains(&r.route_id)
                        && e.attempts.get(&r.route_id).copied().unwrap_or(0)
                            < self.policy.max_attempts_per_route
                })
            })
            .map(|r| r.route_id.clone())
            .collect()
    }

    fn decide(&mut self, candidates: &[String], now_ms: TimestampMs) -> Option<String> {
        if candidates.is_empty() {
            return None;
        }
        // Routes on probation are only chosen when nothing else qualifies.
        let (normal, on_probation): (Vec<&String>, Vec<&String>) = candidates
            .iter()
            .partition(|id| !self.probation.contains_key(*id));
        let pool = if normal.is_empty() {
            on_probation
        } else {
            normal
        };
        let routes: Vec<_> = pool
            .iter()
            .filter_map(|id| self.catalog.route(id))
            .collect();
        let decision = rank(
            &self.catalog,
            &routes,
            &self.health,
            &self.network_id,
            now_ms,
            &self.scoring,
        );
        let best = decision.best().map(|s| s.route_id.clone());
        self.last_decision = Some(decision);
        best
    }

    fn connect_next(&mut self, now_ms: TimestampMs, reason: ConnectReason) -> Vec<Action> {
        let tried = self
            .episode
            .as_ref()
            .map(|e| e.attempts.len() as u32)
            .unwrap_or(0);
        let candidates = if tried >= self.policy.max_routes_per_episode {
            Vec::new()
        } else {
            self.eligible_routes(now_ms, false)
        };
        match self.decide(&candidates, now_ms) {
            Some(route_id) => self.connect(route_id, now_ms, reason),
            None => self.exhaust(now_ms),
        }
    }

    fn connect(
        &mut self,
        route_id: String,
        now_ms: TimestampMs,
        reason: ConnectReason,
    ) -> Vec<Action> {
        let attempt = self.bump_generation();
        if let Some(e) = self.episode.as_mut() {
            *e.attempts.entry(route_id.clone()).or_insert(0) += 1;
        }
        self.state = State::Connecting {
            route_id: route_id.clone(),
            attempt,
            started_ms: now_ms,
        };
        vec![Action::Connect {
            route_id,
            attempt,
            reason,
        }]
    }

    fn exhaust(&mut self, now_ms: TimestampMs) -> Vec<Action> {
        self.exhausted_strikes = self.exhausted_strikes.saturating_add(1);
        let backoff = backoff(
            self.policy.exhausted_backoff_ms,
            self.policy.exhausted_backoff_max_ms,
            self.exhausted_strikes,
        );
        let until_ms = now_ms + backoff;
        self.state = State::Exhausted { until_ms };
        self.episode = None;
        self.bump_generation();
        vec![Action::Exhausted {
            retry_at_ms: until_ms,
        }]
    }

    fn strike(&mut self, route_id: &str, now_ms: TimestampMs) {
        let entry = self
            .cooldowns
            .entry(route_id.to_string())
            .or_insert(Cooldown {
                until_ms: 0,
                strikes: 0,
            });
        entry.strikes = entry.strikes.saturating_add(1);
        entry.until_ms = now_ms
            + backoff(
                self.policy.cooldown_base_ms,
                self.policy.cooldown_max_ms,
                entry.strikes,
            );
        self.probation.insert(route_id.to_string(), 0);
    }

    fn on_connect_result(&mut self, attempt: u64, obs: Observation) -> Vec<Action> {
        let State::Connecting {
            route_id,
            attempt: current,
            ..
        } = &self.state
        else {
            return Vec::new();
        };
        if *current != attempt || *route_id != obs.route_id || obs.network_id != self.network_id {
            return Vec::new(); // superseded attempt: fenced
        }
        let route_id = route_id.clone();
        let now_ms = obs.at_ms;
        self.health.record(&obs, &self.scoring.health);

        if let Some(class) = obs.failure.filter(|c| c.is_safety_violation()) {
            return self.block(class);
        }
        if obs.is_usable() {
            return self.on_connected(route_id, now_ms);
        }

        let class = obs.failure.unwrap_or(FailureClass::Unknown);
        match class {
            // Local device problems: trying other routes cannot help.
            FailureClass::LocalTunFailure => self.exhaust(now_ms),
            // Not a route failure; re-plan on the (new) network.
            FailureClass::NetworkTransition => {
                self.connect_next(now_ms, ConnectReason::NetworkChanged)
            }
            _ => {
                if class.implicates_route() {
                    self.strike(&route_id, now_ms);
                }
                if let Some(e) = self.episode.as_mut() {
                    e.excluded.insert(route_id);
                }
                if self.manual_route.is_some() {
                    // Manual selection: never switch silently.
                    return self.exhaust(now_ms);
                }
                self.connect_next(now_ms, ConnectReason::Failover)
            }
        }
    }

    fn on_connected(&mut self, route_id: String, now_ms: TimestampMs) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(e) = self.episode.take() {
            if e.from_outage {
                actions.push(Action::Recovered {
                    route_id: route_id.clone(),
                    recovery_ms: now_ms.saturating_sub(e.started_ms),
                });
            }
        }
        if self
            .last_connected_route
            .as_ref()
            .is_some_and(|r| *r != route_id)
        {
            push_window(&mut self.switches, now_ms, self.policy.flap_window_ms);
        }
        self.last_connected_route = Some(route_id.clone());
        self.exhausted_strikes = 0;
        self.better_since = None;
        self.state = State::Connected {
            route_id,
            since_ms: now_ms,
        };
        actions
    }

    fn block(&mut self, class: FailureClass) -> Vec<Action> {
        self.state = State::Blocked { class };
        self.episode = None;
        self.bump_generation();
        vec![Action::BlockTraffic { class }]
    }

    fn current_route(&self) -> Option<&str> {
        match &self.state {
            State::Connected { route_id, .. } | State::Degraded { route_id, .. } => Some(route_id),
            _ => None,
        }
    }

    fn on_health_check(&mut self, obs: Observation) -> Vec<Action> {
        if obs.network_id != self.network_id {
            return Vec::new(); // stale probe from a previous network
        }
        let now_ms = obs.at_ms;
        self.health.record(&obs, &self.scoring.health);

        if let Some(class) = obs.failure.filter(|c| c.is_safety_violation()) {
            if self.current_route() == Some(obs.route_id.as_str()) {
                return self.block(class);
            }
        }

        let usable = obs.is_usable();
        if usable {
            if let Some(count) = self.probation.get_mut(&obs.route_id) {
                *count += 1;
                if *count >= self.policy.probation_successes {
                    self.probation.remove(&obs.route_id);
                    self.cooldowns.remove(&obs.route_id);
                }
            }
        }

        let Some(current) = self.current_route().map(str::to_string) else {
            return Vec::new();
        };

        if obs.route_id == current {
            if usable {
                if let State::Degraded { since_ms, .. } = self.state {
                    let _ = since_ms;
                    self.state = State::Connected {
                        route_id: current,
                        since_ms: now_ms,
                    };
                }
                return Vec::new();
            }
            if !obs.is_failure() {
                return Vec::new();
            }
            let failed_checks = match &self.state {
                State::Degraded { failed_checks, .. } => failed_checks + 1,
                _ => 1,
            };
            let since_ms = match &self.state {
                State::Degraded { since_ms, .. } => *since_ms,
                _ => now_ms,
            };
            if failed_checks < self.policy.degraded_grace_failures {
                self.state = State::Degraded {
                    route_id: current,
                    failed_checks,
                    since_ms,
                };
                return Vec::new();
            }
            return self.start_recovery(current, since_ms, now_ms, obs.failure);
        }

        self.consider_better_route(&current, now_ms)
    }

    fn start_recovery(
        &mut self,
        current: String,
        outage_started_ms: TimestampMs,
        now_ms: TimestampMs,
        failure: Option<FailureClass>,
    ) -> Vec<Action> {
        if failure.unwrap_or(FailureClass::Unknown).implicates_route() {
            self.strike(&current, now_ms);
        }
        push_window(&mut self.recoveries, now_ms, self.policy.flap_window_ms);
        if self.recoveries.len() > self.policy.max_recoveries_in_window {
            let mut actions = vec![Action::Disconnect];
            actions.extend(self.exhaust(now_ms));
            return actions;
        }
        if self.manual_route.is_some() {
            let mut actions = vec![Action::Disconnect];
            actions.extend(self.exhaust(now_ms));
            return actions;
        }
        self.begin_episode(outage_started_ms, true);
        if let Some(e) = self.episode.as_mut() {
            e.excluded.insert(current);
        }
        self.connect_next(now_ms, ConnectReason::Failover)
    }

    fn consider_better_route(&mut self, current: &str, now_ms: TimestampMs) -> Vec<Action> {
        if self.manual_route.is_some() || !matches!(self.state, State::Connected { .. }) {
            return Vec::new();
        }
        let mut candidates = self.eligible_routes(now_ms, false);
        if !candidates.iter().any(|c| c == current) {
            candidates.push(current.to_string());
        }
        let Some(best) = self.decide(&candidates, now_ms) else {
            return Vec::new();
        };
        let decision = self.last_decision.as_ref().expect("decide sets it");
        let (Some(best_score), Some(current_score)) =
            (decision.score_of(&best), decision.score_of(current))
        else {
            self.better_since = None;
            return Vec::new();
        };
        if best == current || best_score - current_score < self.policy.switch_margin {
            self.better_since = None;
            return Vec::new();
        }
        let since = match &self.better_since {
            Some((route, since)) if *route == best => *since,
            _ => {
                self.better_since = Some((best.clone(), now_ms));
                now_ms
            }
        };
        if now_ms.saturating_sub(since) < self.policy.min_dwell_ms {
            return Vec::new();
        }
        prune_window(&mut self.switches, now_ms, self.policy.flap_window_ms);
        if self.switches.len() >= self.policy.max_switches_in_window {
            return Vec::new(); // flap limit: keep the working route
        }
        self.begin_episode(now_ms, false);
        self.connect(best, now_ms, ConnectReason::BetterRoute)
    }

    fn on_network_changed(&mut self, network_id: String, now_ms: TimestampMs) -> Vec<Action> {
        self.network_id = network_id;
        self.cooldowns.clear();
        self.better_since = None;
        self.exhausted_strikes = 0;
        if matches!(self.state, State::Idle | State::Blocked { .. }) {
            return Vec::new();
        }
        self.begin_episode(now_ms, false);
        self.connect_next(now_ms, ConnectReason::NetworkChanged)
    }

    fn on_catalog_changed(&mut self, catalog: RouteCatalog, now_ms: TimestampMs) -> Vec<Action> {
        self.catalog = catalog;
        let catalog = &self.catalog;
        self.health.retain_routes(&|id| catalog.route(id).is_some());
        self.cooldowns.retain(|id, _| catalog.route(id).is_some());
        self.probation.retain(|id, _| catalog.route(id).is_some());
        if self
            .manual_route
            .as_ref()
            .is_some_and(|m| catalog.route(m).is_none())
        {
            self.manual_route = None;
        }
        let current_gone = match &self.state {
            State::Connected { route_id, .. }
            | State::Degraded { route_id, .. }
            | State::Connecting { route_id, .. } => {
                let supported = |t: &TransportKind| self.supported.contains(t);
                let still_selectable = self
                    .catalog
                    .selectable_routes(&supported)
                    .any(|r| r.route_id == *route_id && self.mode.allows(r.kind));
                !still_selectable
            }
            _ => false,
        };
        if current_gone {
            self.begin_episode(now_ms, false);
            return self.connect_next(now_ms, ConnectReason::CatalogChanged);
        }
        Vec::new()
    }

    fn on_user_selected(&mut self, route_id: Option<String>, now_ms: TimestampMs) -> Vec<Action> {
        self.manual_route = route_id;
        if matches!(self.state, State::Idle) {
            return Vec::new();
        }
        self.cooldowns.clear();
        self.exhausted_strikes = 0;
        self.begin_episode(now_ms, false);
        self.connect_next(now_ms, ConnectReason::UserSelected)
    }

    fn on_tick(&mut self, now_ms: TimestampMs) -> Vec<Action> {
        match self.state.clone() {
            State::Exhausted { until_ms } if now_ms >= until_ms => {
                self.begin_episode(now_ms, false);
                self.connect_next(now_ms, ConnectReason::RetryAfterBackoff)
            }
            State::Connecting {
                route_id,
                attempt,
                started_ms,
            } if now_ms.saturating_sub(started_ms) >= self.policy.connect_timeout_ms => {
                let obs = Observation {
                    route_id,
                    network_id: self.network_id.clone(),
                    at_ms: now_ms,
                    layers: vec![crate::health::LayerResult {
                        layer: crate::health::HealthLayer::Tunnel,
                        ok: false,
                        duration_ms: None,
                    }],
                    failure: Some(FailureClass::Unknown),
                    handshake_ms: None,
                    rtt_ms: None,
                    jitter_ms: None,
                    loss_permille: None,
                    transfer_kbps: None,
                };
                self.on_connect_result(attempt, obs)
            }
            _ => Vec::new(),
        }
    }
}

fn backoff(base: u64, max: u64, strikes: u32) -> u64 {
    let shift = strikes.saturating_sub(1).min(20);
    base.saturating_mul(1u64 << shift).min(max)
}

fn prune_window(q: &mut VecDeque<TimestampMs>, now_ms: TimestampMs, window: u64) {
    while q
        .front()
        .is_some_and(|t| now_ms.saturating_sub(*t) > window)
    {
        q.pop_front();
    }
}

fn push_window(q: &mut VecDeque<TimestampMs>, now_ms: TimestampMs, window: u64) {
    prune_window(q, now_ms, window);
    q.push_back(now_ms);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::test_support::{failure, success};
    use crate::health::HealthLayer;
    use crate::route::test_support::two_node_catalog;
    use crate::route::NodeStatus;

    fn all_transports() -> BTreeSet<TransportKind> {
        [
            TransportKind::VlessReality,
            TransportKind::Hysteria2,
            TransportKind::AmneziaWg,
        ]
        .into_iter()
        .collect()
    }

    fn controller(mode: SessionMode) -> FailoverController {
        FailoverController::new(
            FailoverPolicy::default(),
            ScoringPolicy::default(),
            mode,
            all_transports(),
            two_node_catalog(),
            "net-a",
        )
    }

    fn connect_action(actions: &[Action]) -> (String, u64, ConnectReason) {
        actions
            .iter()
            .find_map(|a| match a {
                Action::Connect {
                    route_id,
                    attempt,
                    reason,
                } => Some((route_id.clone(), *attempt, *reason)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no connect in {actions:?}"))
    }

    fn fail_connect(
        c: &mut FailoverController,
        route: &str,
        attempt: u64,
        at: u64,
        class: FailureClass,
    ) -> Vec<Action> {
        c.handle(Event::ConnectResult {
            attempt,
            observation: failure(route, at, HealthLayer::Handshake, class),
        })
    }

    #[test]
    fn start_connects_to_best_route_and_success_connects() {
        let mut c = controller(SessionMode::Fast);
        let (route, attempt, reason) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        assert_eq!(
            (route.as_str(), reason),
            ("fi1/awg", ConnectReason::Initial)
        );
        let actions = c.handle(Event::ConnectResult {
            attempt,
            observation: success(&route, 100, 40),
        });
        assert!(
            actions.is_empty(),
            "initial connect reports no recovery: {actions:?}"
        );
        assert!(matches!(c.state(), State::Connected { route_id, .. } if route_id == "fi1/awg"));
    }

    #[test]
    fn failed_route_falls_over_to_next_and_is_cooled_down() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        let (r2, _, reason) = connect_action(&fail_connect(
            &mut c,
            &r1,
            a1,
            10,
            FailureClass::UdpUnreachable,
        ));
        assert_ne!(r1, r2);
        assert_eq!(reason, ConnectReason::Failover);
        // UDP-hostile network: the next choice is a TCP route.
        assert!(r2.ends_with("/reality"), "{r2}");
        assert!(c.is_in_probation(&r1));
    }

    #[test]
    fn retries_are_bounded_then_exhausted_then_retried_after_backoff() {
        let mut c = controller(SessionMode::Fast);
        let mut actions = c.handle(Event::Start { now_ms: 0 });
        let mut attempts = 0;
        loop {
            match actions
                .iter()
                .find(|a| matches!(a, Action::Connect { .. } | Action::Exhausted { .. }))
            {
                Some(Action::Connect {
                    route_id, attempt, ..
                }) => {
                    attempts += 1;
                    let (r, a) = (route_id.clone(), *attempt);
                    actions = fail_connect(&mut c, &r, a, attempts, FailureClass::HandshakeFailure);
                }
                Some(Action::Exhausted { retry_at_ms }) => {
                    assert_eq!(*retry_at_ms, attempts + 30_000);
                    break;
                }
                _ => panic!("unexpected {actions:?}"),
            }
            assert!(attempts <= 10, "reconnect loop");
        }
        assert_eq!(attempts, 4, "max_routes_per_episode");
        assert!(c.handle(Event::Tick { now_ms: 1000 }).is_empty());
        let retry = c.handle(Event::Tick {
            now_ms: attempts + 30_000,
        });
        let (_, _, reason) = connect_action(&retry);
        assert_eq!(reason, ConnectReason::RetryAfterBackoff);
    }

    #[test]
    fn exhaustion_backoff_grows_and_is_capped() {
        assert_eq!(backoff(30_000, 600_000, 1), 30_000);
        assert_eq!(backoff(30_000, 600_000, 2), 60_000);
        assert_eq!(backoff(30_000, 600_000, 10), 600_000);
    }

    #[test]
    fn superseded_connect_results_are_ignored() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        let (r2, _, _) = connect_action(&c.handle(Event::NetworkChanged {
            network_id: "net-b".into(),
            now_ms: 5,
        }));
        let mut late = success(&r1, 6, 40);
        late.network_id = "net-b".into();
        assert!(c
            .handle(Event::ConnectResult {
                attempt: a1,
                observation: late
            })
            .is_empty());
        assert!(matches!(c.state(), State::Connecting { route_id, .. } if *route_id == r2));
    }

    #[test]
    fn network_change_is_not_a_route_failure() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: success(&r1, 10, 40),
        });
        let actions = c.handle(Event::NetworkChanged {
            network_id: "net-b".into(),
            now_ms: 20,
        });
        let (r2, _, reason) = connect_action(&actions);
        assert_eq!(reason, ConnectReason::NetworkChanged);
        assert_eq!(r1, r2, "same best route on a fresh network");
        assert!(!c.is_in_probation(&r1));
    }

    #[test]
    fn one_failed_health_check_is_grace_two_start_recovery_with_measured_time() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: success(&r1, 10, 40),
        });

        let obs = failure(
            &r1,
            1_000,
            HealthLayer::Internet,
            FailureClass::PostHandshakeStall,
        );
        assert!(c.handle(Event::HealthCheck { observation: obs }).is_empty());
        assert!(matches!(
            c.state(),
            State::Degraded {
                failed_checks: 1,
                ..
            }
        ));

        let obs = failure(
            &r1,
            2_000,
            HealthLayer::Internet,
            FailureClass::PostHandshakeStall,
        );
        let (r2, a2, reason) = connect_action(&c.handle(Event::HealthCheck { observation: obs }));
        assert_ne!(r1, r2);
        assert_eq!(reason, ConnectReason::Failover);
        let actions = c.handle(Event::ConnectResult {
            attempt: a2,
            observation: success(&r2, 3_500, 50),
        });
        assert_eq!(
            actions,
            vec![Action::Recovered {
                route_id: r2,
                recovery_ms: 2_500
            }]
        );
    }

    #[test]
    fn a_recovered_health_check_clears_degraded() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: success(&r1, 10, 40),
        });
        c.handle(Event::HealthCheck {
            observation: failure(&r1, 1_000, HealthLayer::Internet, FailureClass::Unknown),
        });
        c.handle(Event::HealthCheck {
            observation: success(&r1, 2_000, 40),
        });
        assert!(matches!(c.state(), State::Connected { .. }));
    }

    #[test]
    fn hysteresis_requires_margin_and_dwell() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        assert_eq!(r1, "fi1/awg");
        // Slow but working current route.
        let mut slow = success(&r1, 10, 600);
        slow.handshake_ms = Some(4_000);
        slow.loss_permille = Some(80);
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: slow,
        });

        let good = |at| {
            let mut o = success("nl1/reality", at, 30);
            o.layers.push(crate::health::LayerResult {
                layer: HealthLayer::Transfer,
                ok: true,
                duration_ms: None,
            });
            o
        };
        assert!(
            c.handle(Event::HealthCheck {
                observation: good(1_000)
            })
            .is_empty(),
            "no switch before dwell"
        );
        assert!(c
            .handle(Event::HealthCheck {
                observation: good(30_000)
            })
            .is_empty());
        let (r2, _, reason) = connect_action(&c.handle(Event::HealthCheck {
            observation: good(61_000),
        }));
        assert_eq!(
            (r2.as_str(), reason),
            ("nl1/reality", ConnectReason::BetterRoute)
        );
    }

    #[test]
    fn small_improvements_never_switch() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: success(&r1, 10, 40),
        });
        // Both routes are probed equally; the alternative is marginally
        // faster, which must never be worth an egress-IP change.
        for t in 1..20u64 {
            c.handle(Event::HealthCheck {
                observation: success(&r1, t * 60_000, 40),
            });
            let actions = c.handle(Event::HealthCheck {
                observation: success("nl1/awg", t * 60_000 + 1, 20),
            });
            assert!(actions.is_empty(), "switched at {t}: {actions:?}");
        }
    }

    #[test]
    fn repeated_outages_hit_the_recovery_flap_limit() {
        let mut c = controller(SessionMode::Fast);
        let (mut route, attempt, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt,
            observation: success(&route, 1, 40),
        });
        let mut t = 10;
        let mut exhausted = false;
        for _ in 0..10 {
            c.handle(Event::HealthCheck {
                observation: failure(
                    &route,
                    t,
                    HealthLayer::Internet,
                    FailureClass::PostHandshakeStall,
                ),
            });
            let actions = c.handle(Event::HealthCheck {
                observation: failure(
                    &route,
                    t + 1,
                    HealthLayer::Internet,
                    FailureClass::PostHandshakeStall,
                ),
            });
            if actions
                .iter()
                .any(|a| matches!(a, Action::Exhausted { .. }))
            {
                exhausted = true;
                break;
            }
            let (next, attempt, _) = connect_action(&actions);
            c.handle(Event::ConnectResult {
                attempt,
                observation: success(&next, t + 2, 40),
            });
            route = next;
            t += 1_000;
        }
        assert!(
            exhausted,
            "recoveries must be capped within the flap window"
        );
    }

    #[test]
    fn probation_requires_repeated_successes() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        let (r2, a2, _) = connect_action(&fail_connect(
            &mut c,
            &r1,
            a1,
            1,
            FailureClass::HandshakeFailure,
        ));
        c.handle(Event::ConnectResult {
            attempt: a2,
            observation: success(&r2, 2, 40),
        });
        assert!(c.is_in_probation(&r1));
        for i in 0..3 {
            c.handle(Event::HealthCheck {
                observation: success(&r1, 100_000 + i, 40),
            });
        }
        assert!(!c.is_in_probation(&r1));
    }

    #[test]
    fn privacy_plus_never_uses_direct_routes() {
        let mut c = controller(SessionMode::PrivacyPlus);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        assert_eq!(r1, "nl1/reality-via-ru1");
        let actions = fail_connect(&mut c, &r1, a1, 1, FailureClass::HandshakeFailure);
        assert!(
            actions.iter().all(|a| !matches!(a, Action::Connect { .. })),
            "must not downgrade: {actions:?}"
        );
        assert!(matches!(c.state(), State::Exhausted { .. }));
    }

    #[test]
    fn manual_selection_never_switches_silently() {
        let mut c = controller(SessionMode::Fast);
        c.handle(Event::Start { now_ms: 0 });
        let (r, a, reason) = connect_action(&c.handle(Event::UserSelected {
            route_id: Some("nl1/hy2".into()),
            now_ms: 1,
        }));
        assert_eq!(
            (r.as_str(), reason),
            ("nl1/hy2", ConnectReason::UserSelected)
        );
        let actions = fail_connect(&mut c, &r, a, 2, FailureClass::UdpUnreachable);
        assert!(
            actions.iter().all(|a| !matches!(a, Action::Connect { .. })),
            "{actions:?}"
        );
    }

    #[test]
    fn leak_detection_blocks_instead_of_switching() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: success(&r1, 1, 40),
        });
        let actions = c.handle(Event::HealthCheck {
            observation: failure(
                &r1,
                2,
                HealthLayer::Internet,
                FailureClass::Ipv6LeakDetected,
            ),
        });
        assert_eq!(
            actions,
            vec![Action::BlockTraffic {
                class: FailureClass::Ipv6LeakDetected
            }]
        );
        assert!(c.handle(Event::Tick { now_ms: 10_000_000 }).is_empty());
    }

    #[test]
    fn local_tun_failure_does_not_cycle_routes() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        let actions = c.handle(Event::ConnectResult {
            attempt: a1,
            observation: failure(&r1, 1, HealthLayer::Tunnel, FailureClass::LocalTunFailure),
        });
        assert!(matches!(actions.as_slice(), [Action::Exhausted { .. }]));
        assert!(!c.is_in_probation(&r1));
    }

    #[test]
    fn removed_or_retired_current_route_triggers_reconnect() {
        let mut c = controller(SessionMode::Fast);
        let (r1, a1, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        c.handle(Event::ConnectResult {
            attempt: a1,
            observation: success(&r1, 1, 40),
        });
        let mut catalog = two_node_catalog();
        catalog.nodes[0].status = NodeStatus::Retired;
        let (r2, _, reason) =
            connect_action(&c.handle(Event::CatalogChanged { catalog, now_ms: 2 }));
        assert!(r2.starts_with("nl1"));
        assert_eq!(reason, ConnectReason::CatalogChanged);
        assert!(
            c.health().get("net-a", &r1).is_some(),
            "retired, not removed: history kept"
        );
    }

    #[test]
    fn connect_timeout_counts_as_failure() {
        let mut c = controller(SessionMode::Fast);
        let (r1, _, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        let (r2, _, reason) = connect_action(&c.handle(Event::Tick { now_ms: 20_000 }));
        assert_ne!(r1, r2);
        assert_eq!(reason, ConnectReason::Failover);
    }

    #[test]
    fn clients_without_awg_never_connect_to_awg() {
        let mut c = FailoverController::new(
            FailoverPolicy::default(),
            ScoringPolicy::default(),
            SessionMode::Fast,
            [TransportKind::VlessReality, TransportKind::Hysteria2]
                .into_iter()
                .collect(),
            two_node_catalog(),
            "net-a",
        );
        let (r, _, _) = connect_action(&c.handle(Event::Start { now_ms: 0 }));
        assert!(!r.ends_with("/awg"));
    }
}
