//! Per-connection state machine plus a "possible shutdown" guard that stops
//! the engine from cycling through transports forever when almost nothing
//! is reaching the outside world. See `docs/FAILURE_CLASSIFICATION.md`.

use network_state::FailureCategory;
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Healthy,
    Degraded,
    Failed(FailureCategory),
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    StartConnecting,
    ConnectSucceeded,
    ConnectFailed(FailureCategory),
    DataFlowing,
    StallDetected,
    UserClose,
    Reset,
}

impl ConnectionState {
    /// Total transition function: every `(State, Event)` pair is handled,
    /// so this never panics on an "unexpected" event — invariant #5 in
    /// FAILURE_CLASSIFICATION.md.
    pub fn transition(self, event: Event) -> ConnectionState {
        use ConnectionState::*;
        use Event::*;
        match (self, event) {
            (_, UserClose) => Closed,
            (_, Reset) => Idle,
            (Idle, StartConnecting) => Connecting,
            (Connecting, ConnectSucceeded) => Connected,
            (Connecting, ConnectFailed(cat)) => Failed(cat),
            (Connected, DataFlowing) => Healthy,
            (Connected, StallDetected) => Degraded,
            (Healthy, StallDetected) => Degraded,
            (Healthy, DataFlowing) => Healthy,
            (Degraded, DataFlowing) => Healthy,
            (Failed(_), StartConnecting) => Connecting,
            // No-op for any other combination: state is unchanged rather
            // than panicking or transitioning somewhere undefined.
            (s, _) => s,
        }
    }
}

/// Tracks a sliding window of recent connection outcomes across *all*
/// transports/endpoints to detect "most external routes unavailable"
/// (`PossibleShutdown`) and, critically, to suppress further automatic
/// transport switching while that condition holds — otherwise the engine
/// would cycle through every transport/endpoint combination forever
/// (invariant: shutdown detection must not cause infinite transport
/// cycling).
pub struct ShutdownGuard {
    window: VecDeque<(u64, bool)>, // (tick, success)
    window_size: usize,
    failure_fraction_threshold: f32,
    cooldown_ticks: u64,
    cooldown_until: Option<u64>,
}

impl ShutdownGuard {
    pub fn new(window_size: usize, failure_fraction_threshold: f32, cooldown_ticks: u64) -> Self {
        Self {
            window: VecDeque::with_capacity(window_size),
            window_size,
            failure_fraction_threshold,
            cooldown_ticks,
            cooldown_until: None,
        }
    }

    /// Record one connection outcome at logical time `tick`.
    pub fn record(&mut self, tick: u64, success: bool) {
        if self.window.len() >= self.window_size {
            self.window.pop_front();
        }
        self.window.push_back((tick, success));
    }

    fn failure_fraction(&self) -> f32 {
        if self.window.is_empty() {
            return 0.0;
        }
        let failures = self.window.iter().filter(|(_, ok)| !ok).count();
        failures as f32 / self.window.len() as f32
    }

    /// Returns true if the engine should stop rotating transports/endpoints
    /// right now and instead report "no external path detected".
    pub fn should_suppress_switching(&mut self, tick: u64) -> bool {
        if let Some(until) = self.cooldown_until {
            if tick < until {
                return true;
            }
            self.cooldown_until = None;
            // Discard the observations that armed this cooldown. They are
            // now provably stale: the engine does NOT dial while it is
            // suppressing (it returns `PossibleShutdown` before touching a
            // transport), so no `record()` can have happened during the
            // cooldown and the window still holds exactly the failures that
            // armed it. Re-evaluating it here would find the same
            // all-failure contents, arm another cooldown, and do so forever
            // — a terminal absorbing state that only a process restart
            // escapes, which is the opposite of the intended "pause, then
            // re-check" behaviour.
            //
            // Clearing lets the engine issue a bounded re-probe. If the
            // network is still down those probes fail, refill the window,
            // and re-arm the cooldown — so the anti-cycling invariant
            // (docs/FAILURE_CLASSIFICATION.md) still holds, at a low duty
            // cycle instead of at 100%.
            self.window.clear();
        }
        if self.window.len() >= self.window_size.max(4)
            && self.failure_fraction() >= self.failure_fraction_threshold
        {
            self.cooldown_until = Some(tick + self.cooldown_ticks);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn shutdown_guard_suppresses_switching_after_mass_failure() {
        let mut guard = ShutdownGuard::new(6, 0.8, 100);
        for tick in 0..6 {
            guard.record(tick, false);
        }
        assert!(guard.should_suppress_switching(6));
    }

    #[test]
    fn shutdown_guard_does_not_trigger_on_healthy_traffic() {
        let mut guard = ShutdownGuard::new(6, 0.8, 100);
        for tick in 0..6 {
            guard.record(tick, true);
        }
        assert!(!guard.should_suppress_switching(6));
    }

    #[test]
    fn shutdown_cooldown_holds_for_its_full_duration_no_infinite_cycling() {
        let mut guard = ShutdownGuard::new(6, 0.8, 50);
        for tick in 0..6 {
            guard.record(tick, false);
        }
        assert!(guard.should_suppress_switching(6));
        // Still suppressed throughout the cooldown window, even if new
        // (still-failing) observations keep arriving.
        for tick in 7..56 {
            guard.record(tick, false);
            assert!(guard.should_suppress_switching(tick), "tick {tick}");
        }
    }

    #[test]
    fn shutdown_cooldown_releases_for_a_re_probe_once_it_expires() {
        let mut guard = ShutdownGuard::new(6, 0.8, 10);
        for tick in 0..6 {
            guard.record(tick, false);
        }
        assert!(guard.should_suppress_switching(6));
        // Once the cooldown expires the guard MUST release, so the engine
        // gets to dial again and find out whether the network came back.
        //
        // This is the regression guard for the terminal-absorbing-state
        // bug: the previous implementation re-evaluated the (necessarily
        // unchanged) window here and armed a fresh cooldown, forever. The
        // engine never dials while suppressed, so "wait for successes to
        // appear in the window before releasing" is unsatisfiable by
        // construction — the window can only refill *after* a release.
        assert!(!guard.should_suppress_switching(20));
    }

    #[test]
    fn shutdown_guard_re_arms_when_the_re_probe_also_fails() {
        let mut guard = ShutdownGuard::new(6, 0.8, 10);
        for tick in 0..6 {
            guard.record(tick, false);
        }
        assert!(guard.should_suppress_switching(6));
        assert!(!guard.should_suppress_switching(20)); // released for a re-probe
                                                       // The re-probe fails too: the guard must suppress again rather
                                                       // than let the engine cycle transports at full rate.
        for tick in 20..26 {
            guard.record(tick, false);
        }
        assert!(guard.should_suppress_switching(26));
        // ...and stays suppressed for the whole fresh cooldown.
        assert!(guard.should_suppress_switching(30));
    }

    #[test]
    fn shutdown_guard_stays_released_when_the_re_probe_succeeds() {
        let mut guard = ShutdownGuard::new(6, 0.8, 10);
        for tick in 0..6 {
            guard.record(tick, false);
        }
        assert!(guard.should_suppress_switching(6));
        assert!(!guard.should_suppress_switching(20));
        for tick in 20..26 {
            guard.record(tick, true);
        }
        assert!(!guard.should_suppress_switching(26));
    }

    /// The property the absorbing-state bug violated: however long the
    /// outage lasted and whatever the parameters, a guard that is left
    /// alone (no further observations, exactly as the engine behaves while
    /// suppressed) must eventually release. Otherwise the only recovery
    /// from a transient outage is a process restart.
    #[test]
    fn shutdown_guard_is_never_a_terminal_state() {
        for window_size in [4usize, 6, 8, 16] {
            for cooldown in [1u64, 10, 90] {
                let mut guard = ShutdownGuard::new(window_size, 0.75, cooldown);
                for tick in 0..window_size as u64 {
                    guard.record(tick, false);
                }
                assert!(guard.should_suppress_switching(window_size as u64));
                let released = (0..1000).any(|i| {
                    !guard.should_suppress_switching(window_size as u64 + 1 + i * cooldown.max(1))
                });
                assert!(
                    released,
                    "guard never released (window_size={window_size}, cooldown={cooldown})"
                );
            }
        }
    }

    proptest! {
        #[test]
        fn state_transitions_never_panic(
            start in 0u8..7,
            event_kind in 0u8..7,
        ) {
            let state = match start {
                0 => ConnectionState::Idle,
                1 => ConnectionState::Connecting,
                2 => ConnectionState::Connected,
                3 => ConnectionState::Healthy,
                4 => ConnectionState::Degraded,
                5 => ConnectionState::Failed(FailureCategory::TcpReset),
                _ => ConnectionState::Closed,
            };
            let event = match event_kind {
                0 => Event::StartConnecting,
                1 => Event::ConnectSucceeded,
                2 => Event::ConnectFailed(FailureCategory::TlsFailure),
                3 => Event::DataFlowing,
                4 => Event::StallDetected,
                5 => Event::UserClose,
                _ => Event::Reset,
            };
            // Must return *some* defined state, never panic.
            let _ = state.transition(event);
        }
    }
}
