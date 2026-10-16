//! Control-plane pacing: bounded exponential backoff with jitter, `Retry-After`
//! support and one shared circuit breaker for every polling call.
//!
//! Why this exists: on 2026-10-06 the agents polled three control-plane
//! endpoints every few seconds, forever, with a fixed sleep. When the
//! control plane started refusing requests (daily request quota, CPU limit)
//! they kept the same rate, so a temporary refusal became a sustained request
//! storm. Everything that talks to the control plane now goes through one
//! [`ControlPlaneGate`]: after a failure NO polling call leaves the node
//! until the gate reopens, and each consecutive failure lengthens the wait
//! (jittered, capped). A success closes it again immediately.
//!
//! The gate never affects local safety work (credential expiry, lease
//! rotation, sing-box supervision); it only gates network calls.

use reqwest::header::{HeaderMap, RETRY_AFTER};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Longest `Retry-After` the agent will honour. A hostile or broken server
/// must not be able to silence a node for hours.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// A uniformly distributed value in `[0, 1)` from the OS-seeded std hasher;
/// no extra dependency, and independent between processes/nodes, which is
/// all jitter needs.
pub fn random_unit() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(0x9e37_79b9_7f4a_7c15);
    (hasher.finish() >> 11) as f64 / (1_u64 << 53) as f64
}

/// Parses `Retry-After` as delta-seconds or an HTTP-date. Anything else, a
/// date in the past, or a value above [`MAX_RETRY_AFTER`] is clamped/ignored.
pub fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let raw = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER));
    }
    let when = time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc2822).ok()?;
    let delta = when - time::OffsetDateTime::now_utc();
    if delta.is_negative() {
        return None;
    }
    Some(Duration::from_secs(delta.whole_seconds() as u64).min(MAX_RETRY_AFTER))
}

/// "Equal jitter" exponential backoff: half the exponential step is fixed and
/// half is random, so the delay never collapses to ~0 (which would defeat the
/// breaker) yet a fleet of agents does not retry in lock-step.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
    failures: u32,
}

impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Self { base, cap, failures: 0 }
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }

    /// Delay for the next retry; `jitter` is in `[0, 1)`.
    pub fn next_delay(&mut self, jitter: f64) -> Duration {
        let exponent = self.failures.min(20);
        let step = self
            .base
            .checked_mul(1_u32 << exponent)
            .unwrap_or(self.cap)
            .min(self.cap);
        self.failures = self.failures.saturating_add(1);
        let jitter = jitter.clamp(0.0, 1.0);
        step / 2 + step.mul_f64(0.5 * jitter)
    }
}

/// Why a polling call did not go out or did not succeed. Carried inside
/// `anyhow::Error` so callers can `downcast_ref` and pace accordingly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The shared gate is closed; no request was sent.
    Gated { remaining: Duration },
    /// The control plane answered with a non-success status.
    Status {
        what: &'static str,
        status: u16,
        retry_after: Option<Duration>,
    },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Gated { remaining } => {
                write!(f, "control plane paused by backoff for another {}s", remaining.as_secs())
            }
            ApiError::Status { what, status, .. } => write!(f, "{what} returned {status}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// True when `err` only means "the gate is closed, nothing was attempted"
/// (expected during an outage; must not be logged at warn level every tick).
pub fn is_gated(err: &anyhow::Error) -> bool {
    matches!(err.downcast_ref::<ApiError>(), Some(ApiError::Gated { .. }))
}

#[derive(Debug)]
struct GateState {
    backoff: Backoff,
    blocked_until: Option<Instant>,
    consecutive_failures: u32,
}

/// Shared circuit breaker. Cheap to clone; all clones see the same state.
#[derive(Debug, Clone)]
pub struct ControlPlaneGate {
    state: Arc<Mutex<GateState>>,
}

impl ControlPlaneGate {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(GateState {
                backoff: Backoff::new(base, cap),
                blocked_until: None,
                consecutive_failures: 0,
            })),
        }
    }

    pub fn check(&self) -> Result<(), ApiError> {
        self.check_at(Instant::now())
    }

    pub fn check_at(&self, now: Instant) -> Result<(), ApiError> {
        let state = self.state.lock().expect("gate mutex poisoned");
        match state.blocked_until {
            Some(until) if until > now => Err(ApiError::Gated { remaining: until - now }),
            _ => Ok(()),
        }
    }

    pub fn record_success(&self) {
        let mut state = self.state.lock().expect("gate mutex poisoned");
        state.backoff.reset();
        state.blocked_until = None;
        state.consecutive_failures = 0;
    }

    pub fn record_failure(&self, retry_after: Option<Duration>) {
        self.record_failure_at(Instant::now(), retry_after, random_unit());
    }

    /// The server's `Retry-After` is a floor: we wait at least that long, and
    /// at least our own jittered backoff, whichever is longer.
    pub fn record_failure_at(&self, now: Instant, retry_after: Option<Duration>, jitter: f64) {
        let mut state = self.state.lock().expect("gate mutex poisoned");
        let own = state.backoff.next_delay(jitter);
        let delay = retry_after.map_or(own, |server| server.max(own));
        let until = now + delay;
        // Never shorten a wait that is already running.
        state.blocked_until = Some(state.blocked_until.map_or(until, |current| current.max(until)));
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.state.lock().expect("gate mutex poisoned").consecutive_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn headers(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn retry_after_seconds_and_garbage() {
        assert_eq!(parse_retry_after(&headers("7")), Some(S(7)));
        assert_eq!(parse_retry_after(&headers(" 30 ")), Some(S(30)));
        assert_eq!(parse_retry_after(&headers("soon")), None);
        assert_eq!(parse_retry_after(&HeaderMap::new()), None);
    }

    #[test]
    fn retry_after_is_capped() {
        assert_eq!(parse_retry_after(&headers("999999")), Some(MAX_RETRY_AFTER));
    }

    #[test]
    fn retry_after_http_date_future_and_past() {
        let future = time::OffsetDateTime::now_utc() + time::Duration::seconds(120);
        let text = future.format(&time::format_description::well_known::Rfc2822).unwrap();
        let parsed = parse_retry_after(&headers(&text)).expect("future date parses");
        assert!(parsed > S(100) && parsed <= S(120), "{parsed:?}");
        let past = time::OffsetDateTime::now_utc() - time::Duration::seconds(120);
        let text = past.format(&time::format_description::well_known::Rfc2822).unwrap();
        assert_eq!(parse_retry_after(&headers(&text)), None);
    }

    #[test]
    fn backoff_grows_geometrically_to_the_cap_and_never_collapses() {
        let mut b = Backoff::new(S(1), S(60));
        let lows: Vec<_> = (0..9).map(|_| b.next_delay(0.0)).collect();
        // equal jitter at jitter=0 -> half the step: 0.5,1,2,4,8,16,30,30,30
        assert_eq!(lows[0], Duration::from_millis(500));
        assert_eq!(lows[3], S(4));
        assert_eq!(lows[6], S(30));
        assert_eq!(lows[8], S(30));
        let mut b = Backoff::new(S(1), S(60));
        for _ in 0..30 {
            let high = b.next_delay(0.999_999);
            assert!(high <= S(60), "{high:?}");
        }
        assert!(b.next_delay(0.0) >= S(30), "stays at the cap, not near zero");
    }

    #[test]
    fn backoff_resets_after_success() {
        let mut b = Backoff::new(S(1), S(60));
        for _ in 0..6 {
            b.next_delay(0.5);
        }
        b.reset();
        assert!(b.next_delay(0.0) < S(1));
    }

    #[test]
    fn backoff_does_not_overflow() {
        let mut b = Backoff::new(S(1), S(60));
        for _ in 0..200 {
            b.next_delay(1.0);
        }
        assert!(b.failures() >= 100);
    }

    #[test]
    fn gate_is_open_until_a_failure_then_blocks_every_caller() {
        let gate = ControlPlaneGate::new(S(2), S(60));
        let t0 = Instant::now();
        assert!(gate.check_at(t0).is_ok());
        gate.record_failure_at(t0, None, 0.0);
        let clone = gate.clone();
        assert!(matches!(clone.check_at(t0), Err(ApiError::Gated { .. })));
        assert!(gate.check_at(t0 + S(1) + Duration::from_millis(1)).is_ok(), "1 s step at jitter 0");
    }

    #[test]
    fn retry_after_is_a_floor_over_our_own_backoff() {
        let gate = ControlPlaneGate::new(S(1), S(60));
        let t0 = Instant::now();
        gate.record_failure_at(t0, Some(S(45)), 0.0);
        assert!(gate.check_at(t0 + S(44)).is_err());
        assert!(gate.check_at(t0 + S(46)).is_ok());
    }

    #[test]
    fn a_short_retry_after_cannot_shorten_our_backoff() {
        let gate = ControlPlaneGate::new(S(8), S(60));
        let t0 = Instant::now();
        gate.record_failure_at(t0, Some(S(1)), 0.0); // own = 4 s
        assert!(gate.check_at(t0 + S(3)).is_err());
    }

    #[test]
    fn repeated_failures_back_off_and_a_later_short_one_never_shortens_the_wait() {
        let gate = ControlPlaneGate::new(S(1), S(60));
        let t0 = Instant::now();
        for _ in 0..8 {
            gate.record_failure_at(t0, None, 0.0);
        }
        assert!(gate.check_at(t0 + S(29)).is_err(), "capped step of 30 s at jitter 0");
        assert!(gate.check_at(t0 + S(31)).is_ok());
        assert_eq!(gate.consecutive_failures(), 8);
        gate.record_failure_at(t0, Some(S(1)), 0.0);
        assert!(gate.check_at(t0 + S(29)).is_err());
    }

    #[test]
    fn success_reopens_immediately_and_restarts_the_ladder() {
        let gate = ControlPlaneGate::new(S(1), S(60));
        let t0 = Instant::now();
        for _ in 0..5 {
            gate.record_failure_at(t0, None, 0.0);
        }
        gate.record_success();
        assert!(gate.check_at(t0).is_ok());
        assert_eq!(gate.consecutive_failures(), 0);
        gate.record_failure_at(t0, None, 0.0);
        assert!(gate.check_at(t0 + Duration::from_millis(501)).is_ok(), "ladder restarted at the first step");
    }

    #[test]
    fn a_storm_of_failures_produces_a_bounded_attempt_rate() {
        // Simulate 1 hour of an agent that would like to poll every 3 s while
        // every attempt fails: the gate must cut the attempts to the capped
        // cadence instead of ~1200.
        let gate = ControlPlaneGate::new(S(1), S(60));
        let start = Instant::now();
        let mut attempts = 0;
        let mut t = 0;
        while t < 3600 {
            let now = start + S(t);
            if gate.check_at(now).is_ok() {
                attempts += 1;
                gate.record_failure_at(now, None, 0.5);
            }
            t += 3;
        }
        assert!(attempts <= 140, "attempts in an hour of outage: {attempts}");
        assert!(attempts >= 40, "still probes regularly enough to notice recovery: {attempts}");
    }

    #[test]
    fn gated_errors_are_recognisable_and_status_errors_keep_the_old_message() {
        let gated: anyhow::Error = ApiError::Gated { remaining: S(5) }.into();
        assert!(is_gated(&gated));
        let status: anyhow::Error = ApiError::Status { what: "POST /api/agent/claim", status: 429, retry_after: None }.into();
        assert!(!is_gated(&status));
        assert_eq!(status.to_string(), "POST /api/agent/claim returned 429");
    }

    #[test]
    fn jitter_source_stays_in_range_and_varies() {
        let samples: Vec<f64> = (0..64).map(|_| random_unit()).collect();
        assert!(samples.iter().all(|v| (0.0..1.0).contains(v)));
        assert!(samples.windows(2).any(|w| w[0] != w[1]));
    }
}
