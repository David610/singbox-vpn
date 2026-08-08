//! Endpoint/transport confidence scoring, quarantine, and non-deterministic
//! fallback selection. See `docs/DECISION_ENGINE.md` for the rationale.

pub mod killswitch;

use common::{EndpointId, TransportId};
use network_state::FailureCategory;
use rand::distributions::{Distribution, WeightedIndex};
use rand::Rng;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const EWMA_ALPHA: f32 = 0.3;
const SCORE_FLOOR: f32 = 0.05;
const QUARANTINE_THRESHOLD: u16 = 3;
const QUARANTINE_MIN: Duration = Duration::from_secs(60);
const QUARANTINE_MAX: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct Score {
    pub ewma_success: f32,
    pub total_attempts: u32,
    pub consecutive_failures: u16,
    pub last_failure_category: Option<FailureCategory>,
    quarantined_until: Option<Instant>,
}

impl Default for Score {
    fn default() -> Self {
        Self {
            // Neutral prior: unknown candidates are tried, not assumed bad.
            ewma_success: 0.5,
            total_attempts: 0,
            consecutive_failures: 0,
            last_failure_category: None,
            quarantined_until: None,
        }
    }
}

impl Score {
    pub fn record_success(&mut self) {
        self.total_attempts += 1;
        self.consecutive_failures = 0;
        self.ewma_success = EWMA_ALPHA * 1.0 + (1.0 - EWMA_ALPHA) * self.ewma_success;
    }

    pub fn record_failure(&mut self, category: FailureCategory, rng: &mut impl Rng) {
        self.total_attempts += 1;
        self.consecutive_failures += 1;
        self.last_failure_category = Some(category);
        self.ewma_success = EWMA_ALPHA * 0.0 + (1.0 - EWMA_ALPHA) * self.ewma_success;
        if self.consecutive_failures >= QUARANTINE_THRESHOLD {
            let jitter_secs = rng.gen_range(QUARANTINE_MIN.as_secs()..=QUARANTINE_MAX.as_secs());
            self.quarantined_until = Some(Instant::now() + Duration::from_secs(jitter_secs));
        }
    }

    pub fn is_quarantined(&self, now: Instant) -> bool {
        matches!(self.quarantined_until, Some(until) if now < until)
    }

    /// Score used for weighted selection: never fully zero, so conditions
    /// that change can be rediscovered (see DECISION_ENGINE.md floor).
    pub fn selection_weight(&self) -> f32 {
        self.ewma_success.max(SCORE_FLOOR)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub transport: TransportId,
    pub endpoint: EndpointId,
}

#[derive(Default)]
pub struct ScoreBoard {
    scores: HashMap<Candidate, Score>,
}

impl ScoreBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn score_of(&self, candidate: &Candidate) -> Score {
        self.scores.get(candidate).cloned().unwrap_or_default()
    }

    pub fn observe_success(&mut self, candidate: &Candidate) {
        self.scores
            .entry(candidate.clone())
            .or_default()
            .record_success();
    }

    /// Applies attribution rules from FAILURE_CLASSIFICATION.md: a failure
    /// only mutates scores for candidates it is attributable to.
    /// `LocalNetworkFailure` mutates nothing (checked by the caller too,
    /// but enforced again here so this function is safe on its own).
    pub fn observe_failure(
        &mut self,
        candidate: &Candidate,
        category: FailureCategory,
        rng: &mut impl Rng,
    ) {
        if !category.is_remote_signal() {
            return;
        }
        self.scores
            .entry(candidate.clone())
            .or_default()
            .record_failure(category, rng);
    }

    /// Select the next candidate to try via weighted random sampling,
    /// excluding quarantined and (by the caller's pre-filter)
    /// capability-incompatible candidates. Deliberately non-deterministic:
    /// no fixed `A -> B -> C` order (spec §8).
    pub fn select_next(&self, candidates: &[Candidate], rng: &mut impl Rng) -> Option<Candidate> {
        let now = Instant::now();
        let eligible: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| !self.score_of(c).is_quarantined(now))
            .collect();
        if eligible.is_empty() {
            return None;
        }
        let weights: Vec<f32> = eligible
            .iter()
            .map(|c| self.score_of(c).selection_weight())
            .collect();
        let dist = WeightedIndex::new(&weights).ok()?;
        Some(eligible[dist.sample(rng)].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn cand(t: &'static str, e: &str) -> Candidate {
        Candidate {
            transport: TransportId::new(t),
            endpoint: EndpointId(e.to_string()),
        }
    }

    #[test]
    fn score_stays_in_unit_interval() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let mut s = Score::default();
        for _ in 0..50 {
            s.record_success();
            assert!((0.0..=1.0).contains(&s.ewma_success));
        }
        for _ in 0..50 {
            s.record_failure(FailureCategory::TcpReset, &mut rng);
            assert!((0.0..=1.0).contains(&s.ewma_success));
        }
    }

    #[test]
    fn quarantine_engages_after_threshold_consecutive_failures() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(2);
        let mut s = Score::default();
        for _ in 0..(QUARANTINE_THRESHOLD - 1) {
            s.record_failure(FailureCategory::TcpReset, &mut rng);
            assert!(!s.is_quarantined(Instant::now()));
        }
        s.record_failure(FailureCategory::TcpReset, &mut rng);
        assert!(s.is_quarantined(Instant::now()));
    }

    #[test]
    fn success_resets_consecutive_failure_counter() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let mut s = Score::default();
        s.record_failure(FailureCategory::TcpReset, &mut rng);
        s.record_failure(FailureCategory::TcpReset, &mut rng);
        s.record_success();
        assert_eq!(s.consecutive_failures, 0);
        assert!(!s.is_quarantined(Instant::now()));
    }

    #[test]
    fn local_network_failure_does_not_mutate_any_score() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(4);
        let mut board = ScoreBoard::new();
        let c = cand("direct-tls", "relay-1");
        board.observe_failure(&c, FailureCategory::LocalNetworkFailure, &mut rng);
        let s = board.score_of(&c);
        assert_eq!(s.total_attempts, 0);
        assert_eq!(s.ewma_success, 0.5); // untouched default
    }

    #[test]
    fn endpoint_failure_does_not_affect_a_different_transports_score() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(5);
        let mut board = ScoreBoard::new();
        let a = cand("direct-tls", "relay-1");
        let b = cand("noise-quic", "relay-1");
        for _ in 0..5 {
            board.observe_failure(&a, FailureCategory::TcpReset, &mut rng);
        }
        let score_b = board.score_of(&b);
        assert_eq!(score_b.total_attempts, 0);
    }

    #[test]
    fn select_next_never_returns_a_quarantined_candidate() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(6);
        let mut board = ScoreBoard::new();
        let a = cand("direct-tls", "relay-1");
        let b = cand("noise-quic", "relay-2");
        for _ in 0..QUARANTINE_THRESHOLD {
            board.observe_failure(&a, FailureCategory::TcpReset, &mut rng);
        }
        assert!(board.score_of(&a).is_quarantined(Instant::now()));
        for _ in 0..200 {
            let chosen = board.select_next(&[a.clone(), b.clone()], &mut rng);
            assert_eq!(chosen, Some(b.clone()));
        }
    }

    #[test]
    fn select_next_returns_none_when_all_quarantined() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let mut board = ScoreBoard::new();
        let a = cand("direct-tls", "relay-1");
        for _ in 0..QUARANTINE_THRESHOLD {
            board.observe_failure(&a, FailureCategory::TcpReset, &mut rng);
        }
        assert_eq!(board.select_next(&[a], &mut rng), None);
    }

    #[test]
    fn weighted_selection_favors_higher_scoring_candidate_over_many_draws() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(8);
        let mut board = ScoreBoard::new();
        let good = cand("direct-tls", "relay-good");
        let bad = cand("noise-quic", "relay-bad");
        for _ in 0..10 {
            board.observe_success(&good);
        }
        for _ in 0..10 {
            board.observe_failure(&bad, FailureCategory::TcpReset, &mut rng);
        }
        let mut good_count = 0;
        for _ in 0..1000 {
            if board.select_next(&[good.clone(), bad.clone()], &mut rng) == Some(good.clone()) {
                good_count += 1;
            }
        }
        // Not deterministic (bad is never fully excluded), but should be
        // strongly favored.
        assert!(good_count > 900, "good_count={good_count}");
    }
}
