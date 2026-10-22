//! Event-driven wake-ups for the main loop.
//!
//! One background task holds a long poll open against `GET /api/agent/events`.
//! The server answers within about a second of a new job, an authorization or
//! entitlement change, or a lease revocation/renewal, naming WHICH channel
//! moved. The task only sets flags and wakes the main loop; the authoritative
//! state is still fetched through the normal (authenticated, ETag-aware)
//! endpoints, and a periodic reconcile remains as the safety net. When the
//! channel is missing or unhealthy, [`Schedule`] returns to plain polling at
//! `poll_interval_secs`, so response times never get worse than before.

use crate::pacing::{random_unit, ApiError};
use crate::worker_client::{EventsOutcome, WorkerClient};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[derive(Default)]
pub struct Wakeups {
    jobs: AtomicBool,
    authorizations: AtomicBool,
    leases: AtomicBool,
    healthy: AtomicBool,
    pub notify: Notify,
}

impl Wakeups {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn mark<S: AsRef<str>>(&self, channels: &[S]) {
        for channel in channels {
            match channel.as_ref() {
                "jobs" => self.jobs.store(true, Ordering::SeqCst),
                "authorizations" => self.authorizations.store(true, Ordering::SeqCst),
                "leases" => self.leases.store(true, Ordering::SeqCst),
                _ => {}
            }
        }
        self.notify.notify_one();
    }

    pub fn take_jobs(&self) -> bool {
        self.jobs.swap(false, Ordering::SeqCst)
    }
    pub fn take_authorizations(&self) -> bool {
        self.authorizations.swap(false, Ordering::SeqCst)
    }
    pub fn take_leases(&self) -> bool {
        self.leases.swap(false, Ordering::SeqCst)
    }

    /// True while the long poll is answering normally.
    pub fn healthy(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }

    pub fn set_healthy(&self, healthy: bool) {
        let was = self.healthy.swap(healthy, Ordering::SeqCst);
        if was && !healthy {
            // Wake the main loop so it resumes polling at once instead of
            // finishing a long reconcile sleep.
            self.notify.notify_one();
        }
    }
}

/// Which network steps the main loop should run on this iteration.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Due {
    pub claim: bool,
    pub authorizations: bool,
    pub leases: bool,
}

/// Decides, per channel, between "an event or the reconcile timer says go"
/// (channel healthy) and "poll every iteration" (no healthy channel).
pub struct Schedule {
    reconcile: Duration,
    next_claim: Instant,
    next_authorizations: Instant,
    next_leases: Instant,
}

impl Schedule {
    pub fn new(reconcile: Duration, now: Instant) -> Self {
        Self {
            reconcile,
            next_claim: now,
            next_authorizations: now,
            next_leases: now,
        }
    }

    pub fn due(&mut self, now: Instant, wake: &Wakeups) -> Due {
        // Always consume the flags, even when polling anyway, so a stale flag
        // cannot trigger a redundant call after the channel recovers.
        let (jobs, auth, leases) = (
            wake.take_jobs(),
            wake.take_authorizations(),
            wake.take_leases(),
        );
        let primary = wake.healthy();
        let claim = !primary || jobs || now >= self.next_claim;
        let authorizations = !primary || auth || now >= self.next_authorizations;
        let leases = !primary || leases || now >= self.next_leases;
        if claim {
            self.next_claim = now + self.reconcile;
        }
        if authorizations {
            self.next_authorizations = now + self.reconcile;
        }
        if leases {
            self.next_leases = now + self.reconcile;
        }
        Due {
            claim,
            authorizations,
            leases,
        }
    }

    /// A job was just processed: look for the next one immediately.
    pub fn claim_again_now(&mut self, now: Instant) {
        self.next_claim = now;
    }

    /// A control-plane step failed before completing: retry on the next
    /// iteration (the shared gate decides whether anything is actually sent).
    pub fn retry_authorizations(&mut self, now: Instant) {
        self.next_authorizations = now;
    }
    pub fn retry_leases(&mut self, now: Instant) {
        self.next_leases = now;
    }
    pub fn retry_claim(&mut self, now: Instant) {
        self.next_claim = now;
    }
}

#[derive(Clone, Copy)]
pub struct Timing {
    pub wait_secs: u64,
    pub unavailable_sleep: Duration,
    pub superseded_floor: Duration,
    pub error_base: Duration,
    pub error_cap: Duration,
}

impl Timing {
    pub fn production(wait_secs: u64) -> Self {
        Self {
            wait_secs,
            unavailable_sleep: Duration::from_secs(600),
            superseded_floor: Duration::from_secs(5),
            error_base: Duration::from_secs(2),
            error_cap: Duration::from_secs(60),
        }
    }
}

fn jittered(base: Duration) -> Duration {
    base.mul_f64(0.75 + 0.5 * random_unit())
}

/// Delay after the event channel was superseded. The server's `Retry-After`
/// and our own floor are minimums: jitter may only add to them (I-87).
fn superseded_delay(retry_after: Option<Duration>, floor: Duration, unit: f64) -> Duration {
    let minimum = retry_after.unwrap_or_default().max(floor);
    minimum + minimum.mul_f64(0.25 * unit.clamp(0.0, 1.0))
}

/// Runs forever (until the process exits).
pub async fn run_loop(client: WorkerClient, wake: Arc<Wakeups>, timing: Timing) {
    let mut cursor: Option<String> = None;
    let mut backoff = crate::pacing::Backoff::new(timing.error_base, timing.error_cap);
    loop {
        match client
            .wait_events(cursor.as_deref(), timing.wait_secs)
            .await
        {
            Ok(EventsOutcome::Event {
                reason,
                changed,
                cursor: next,
            }) => {
                backoff.reset();
                if !next.is_empty() {
                    cursor = Some(next);
                }
                wake.set_healthy(true);
                if reason != "timeout" && !changed.is_empty() {
                    wake.mark(&changed);
                }
            }
            Ok(EventsOutcome::Unavailable) => {
                tracing::info!("control plane has no event channel; polling instead");
                wake.set_healthy(false);
                cursor = None;
                tokio::time::sleep(jittered(timing.unavailable_sleep)).await;
            }
            Ok(EventsOutcome::Superseded { retry_after }) => {
                tracing::warn!("another connection from this node replaced the event channel");
                wake.set_healthy(false);
                tokio::time::sleep(superseded_delay(
                    retry_after,
                    timing.superseded_floor,
                    random_unit(),
                ))
                .await;
            }
            Err(err) => {
                wake.set_healthy(false);
                if let Some(ApiError::Gated { remaining }) = err.downcast_ref::<ApiError>() {
                    tokio::time::sleep(*remaining + Duration::from_millis(50)).await;
                } else {
                    tracing::debug!(error = %err, "event channel request failed");
                    tokio::time::sleep(backoff.next_delay(random_unit())).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn superseded_delay_never_undercuts_the_server_minimum() {
        for unit in [0.0, 0.25, 0.5, 0.999_999] {
            let d = superseded_delay(Some(S(30)), S(5), unit);
            assert!(d >= S(30), "retry-after 30s shortened to {d:?}");
            let d = superseded_delay(None, S(5), unit);
            assert!(d >= S(5), "floor 5s shortened to {d:?}");
            let d = superseded_delay(Some(S(1)), S(5), unit);
            assert!(d >= S(5), "floor must still apply: {d:?}");
        }
    }

    #[test]
    fn without_a_healthy_channel_everything_is_polled_every_iteration() {
        let wake = Wakeups::new();
        let t0 = Instant::now();
        let mut schedule = Schedule::new(S(60), t0);
        for i in 0..5 {
            let due = schedule.due(t0 + S(i * 3), &wake);
            assert_eq!(
                due,
                Due {
                    claim: true,
                    authorizations: true,
                    leases: true
                }
            );
        }
    }

    #[test]
    fn with_a_healthy_channel_only_events_and_the_reconcile_timer_trigger_calls() {
        let wake = Wakeups::new();
        wake.set_healthy(true);
        let t0 = Instant::now();
        let mut schedule = Schedule::new(S(60), t0);
        assert!(
            schedule.due(t0, &wake).claim,
            "first iteration always syncs"
        );
        let quiet = Due {
            claim: false,
            authorizations: false,
            leases: false,
        };
        for i in 1..20 {
            assert_eq!(schedule.due(t0 + S(i * 3), &wake), quiet);
        }
        wake.mark(&["jobs"]);
        assert_eq!(
            schedule.due(t0 + S(58), &wake),
            Due {
                claim: true,
                authorizations: false,
                leases: false
            }
        );
        assert_eq!(
            schedule.due(t0 + S(59), &wake),
            quiet,
            "the flag is consumed once"
        );
        let due = schedule.due(t0 + S(60), &wake);
        assert!(
            due.authorizations && due.leases && !due.claim,
            "reconcile timer fires: {due:?}"
        );
    }

    #[test]
    fn each_channel_wakes_only_its_own_step() {
        let wake = Wakeups::new();
        wake.set_healthy(true);
        let t0 = Instant::now();
        let mut schedule = Schedule::new(S(60), t0);
        schedule.due(t0, &wake);
        wake.mark(&["leases"]);
        assert_eq!(
            schedule.due(t0 + S(3), &wake),
            Due {
                claim: false,
                authorizations: false,
                leases: true
            }
        );
        wake.mark(&["authorizations", "unknown-channel"]);
        assert_eq!(
            schedule.due(t0 + S(6), &wake),
            Due {
                claim: false,
                authorizations: true,
                leases: false
            }
        );
    }

    #[test]
    fn a_processed_job_or_a_failed_step_is_retried_on_the_next_iteration() {
        let wake = Wakeups::new();
        wake.set_healthy(true);
        let t0 = Instant::now();
        let mut schedule = Schedule::new(S(60), t0);
        schedule.due(t0, &wake);
        schedule.claim_again_now(t0 + S(1));
        schedule.retry_leases(t0 + S(1));
        let due = schedule.due(t0 + S(3), &wake);
        assert!(due.claim && due.leases && !due.authorizations);
    }

    #[test]
    fn going_unhealthy_resumes_polling_immediately() {
        let wake = Wakeups::new();
        wake.set_healthy(true);
        let t0 = Instant::now();
        let mut schedule = Schedule::new(S(60), t0);
        schedule.due(t0, &wake);
        assert!(!schedule.due(t0 + S(3), &wake).claim);
        wake.set_healthy(false);
        assert!(schedule.due(t0 + S(6), &wake).claim);
    }

    fn client(base_url: &str) -> WorkerClient {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("agent.toml");
        std::fs::write(
            &config,
            format!(
                "worker_url = {base_url:?}\nnode_id = \"n\"\nagent_api_key = \"k\"\nvpn_admin_binary = \"v\"\nvpn_admin_config = \"c\"\n"
            ),
        )
        .unwrap();
        WorkerClient::new(&AgentConfig::load(&config).unwrap())
    }

    fn fast() -> Timing {
        Timing {
            wait_secs: 5,
            unavailable_sleep: S(3600),
            superseded_floor: S(3600),
            error_base: S(3600),
            error_cap: S(3600),
        }
    }

    async fn eventually(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn a_change_event_sets_the_flag_and_marks_the_channel_healthy() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "reason": "resync", "changed": ["jobs", "leases"], "cursor": "c1"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(200).set_delay(S(30)).set_body_json(
                serde_json::json!({ "reason": "timeout", "changed": [], "cursor": "c1" }),
            ))
            .mount(&server)
            .await;
        let wake = Wakeups::new();
        let task = tokio::spawn(run_loop(client(&server.uri()), wake.clone(), fast()));
        assert!(eventually(|| wake.healthy()).await);
        assert!(wake.take_jobs() && wake.take_leases());
        assert!(!wake.take_authorizations());
        task.abort();
    }

    #[tokio::test]
    async fn a_501_leaves_the_channel_unhealthy_so_polling_continues() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(501))
            .expect(1)
            .mount(&server)
            .await;
        let wake = Wakeups::new();
        let task = tokio::spawn(run_loop(client(&server.uri()), wake.clone(), fast()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!wake.healthy());
        task.abort(); // `expect(1)`: it asked once, then slept instead of hammering
    }

    #[tokio::test]
    async fn a_failing_channel_backs_off_instead_of_retrying_in_a_loop() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/events"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let wake = Wakeups::new();
        let task = tokio::spawn(run_loop(client(&server.uri()), wake.clone(), fast()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!wake.healthy());
        task.abort();
    }
}
