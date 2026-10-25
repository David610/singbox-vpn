//! Safe apply of sing-box config changes: the piece that was completely
//! missing before this hardening pass. Rewriting `config.json` alone
//! does nothing to a process that already has its config loaded in
//! memory — `vpn-admin user disable USER` must actually cause the
//! *running* sing-box to reject that user's credentials, not just leave
//! a correct file on disk. See docs/PRODUCTION_HARDENING_PLAN.md #4/#7.
//!
//! `reload-or-restart` (not bare `reload`) is used deliberately: it
//! degrades safely to a full restart if the running sing-box doesn't
//! support in-place reload for a given change, at the cost of a short
//! connection drop that's acceptable for a single-VPS deployment —
//! correctness over a marginally smoother reload.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub struct CompatibilityServiceManager {
    systemctl_binary: PathBuf,
    service_name: String,
    /// How long to wait after reload-or-restart before checking
    /// `is-active`, to let systemd/sing-box settle. Kept short — this is
    /// a CLI tool, not a long-running health monitor.
    settle: Duration,
}

impl Default for CompatibilityServiceManager {
    fn default() -> Self {
        Self::new("sing-box")
    }
}

/// Environment variable naming the exact service-manager executable.
///
/// Unset in production, where `systemctl` is resolved from `PATH`. Set by
/// the CLI test harness and by `update.sh`'s isolated test phase to a
/// guard, so code under test cannot reach the host's real `sing-box` and
/// `vpn-subscription` units (real two-VPS acceptance defect D6). While it
/// is set there is no fallback: a missing or failing guard means "systemd
/// not available", never the real `systemctl`.
pub const SYSTEMCTL_OVERRIDE_ENV: &str = "SINGBOX_VPN_SYSTEMCTL";

/// What unit tests of this binary get when nothing overrides it: a path
/// that cannot exist, so a unit test that forgets `with_systemctl_binary`
/// sees "systemd not available" instead of the host.
#[cfg(test)]
const DEFAULT_SYSTEMCTL: &str = "/nonexistent/singbox-vpn-unit-test-has-no-systemctl";
#[cfg(not(test))]
const DEFAULT_SYSTEMCTL: &str = "systemctl";

fn systemctl_binary_from(override_value: Option<std::ffi::OsString>) -> PathBuf {
    match override_value {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(DEFAULT_SYSTEMCTL),
    }
}

impl CompatibilityServiceManager {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            systemctl_binary: systemctl_binary_from(std::env::var_os(SYSTEMCTL_OVERRIDE_ENV)),
            service_name: service_name.into(),
            settle: Duration::from_millis(300),
        }
    }

    #[cfg(all(test, unix))]
    pub fn with_systemctl_binary(mut self, path: PathBuf) -> Self {
        self.systemctl_binary = path;
        self
    }

    #[cfg(all(test, unix))]
    pub fn with_settle(mut self, d: Duration) -> Self {
        self.settle = d;
        self
    }

    /// Runs `systemctl` with the given args, retrying once on a
    /// transient spawn/exec failure (`Command::output()` erroring, not
    /// the process itself running and failing) before giving up. A
    /// fork/exec under host load can fail transiently for reasons that
    /// have nothing to do with whether `systemctl`/the unit is actually
    /// present — every caller in this file must not mistake that for a
    /// real "not available"/"not installed"/"not active" answer, so this
    /// retry lives in one place instead of being copy-pasted (or, as
    /// happened before, present on some call sites and silently missing
    /// on others).
    fn run(&self, args: &[&str]) -> std::io::Result<std::process::Output> {
        match Command::new(&self.systemctl_binary).args(args).output() {
            Ok(o) => Ok(o),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(50));
                Command::new(&self.systemctl_binary).args(args).output()
            }
        }
    }

    /// True if a usable `systemctl` is on PATH — false in local dev /
    /// unit-test environments, where reload is skipped with a warning
    /// rather than treated as a hard failure.
    pub fn is_available(&self) -> bool {
        self.run(&["--version"])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// The full systemd unit name for `self.service_name`: used as-is if
    /// it already names a unit type (contains a `.`, e.g. constructing
    /// this manager with `"vpn-expiry-reconcile.timer"` to inspect a
    /// `.timer` unit instead of a `.service`), otherwise `.service` is
    /// appended — every existing call site constructs this with a bare
    /// name (`"sing-box"`, `"vpn-subscription"`, ...) and has always
    /// meant `.service`, so that behavior is preserved unchanged.
    fn unit_name(&self) -> String {
        if self.service_name.contains('.') {
            self.service_name.clone()
        } else {
            format!("{}.service", self.service_name)
        }
    }

    /// True if the unit is actually installed (`systemctl` itself can be
    /// present — e.g. a GitHub Actions VM — without the `sing-box.service`
    /// unit ever having been installed by `install.sh`). Reload is
    /// skipped, not attempted-and-failed, when the unit isn't installed:
    /// this is what lets `vpn-admin render-config`/`user create` work in
    /// CI and local dev without a real systemd deployment.
    /// `systemctl daemon-reload`: required after a drop-in is added or removed, before the next restart.
    pub fn daemon_reload(&self) -> Result<(), String> {
        match self.run(&["daemon-reload"]) {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(format!(
                "systemctl daemon-reload failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => Err(format!("systemctl daemon-reload could not run: {e}")),
        }
    }

    pub fn is_unit_installed(&self) -> bool {
        let unit = self.unit_name();
        self.run(&["show", "-p", "LoadState", "--value", &unit])
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "loaded")
            .unwrap_or(false)
    }

    /// Reads one `systemctl show -p <property> --value` string for this
    /// unit. Shared by every `--value`-style property lookup below so
    /// there is exactly one place that decides what "unavailable" means
    /// (spawn/exec failure, non-zero exit, or an empty value are all
    /// treated identically — `None`, never a fabricated default that
    /// could be misread as a real answer).
    fn show_value(&self, property: &str) -> Option<String> {
        self.run(&["show", "-p", property, "--value", &self.service_name])
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// How many times systemd has automatically restarted this unit
    /// (`systemctl show -p NRestarts`) since it was last (re)started —
    /// resets to 0 on every explicit `systemctl start`/`restart`, so a
    /// nonzero count means "has crashed and been auto-restarted since
    /// the last deliberate (re)start," a real signal `vpn doctor`/`vpn
    /// status` can surface even when the unit is currently healthy.
    /// Only meaningful for `Restart=`-driven units (sing-box,
    /// vpn-subscription); a `Type=oneshot` unit with no `Restart=`
    /// (vpn-expiry-reconcile) always reports 0 here — `is_failed`/
    /// `last_result` are the right questions for that kind of unit
    /// instead. `None` if unavailable (old systemd, dev/CI host, or the
    /// unit isn't installed) — never fabricated as `Some(0)`, which
    /// would be indistinguishable from a genuinely healthy unit.
    pub fn restart_count(&self) -> Option<u64> {
        self.show_value("NRestarts").and_then(|s| s.parse().ok())
    }

    /// The systemd-assigned `Result=` of this unit's most recent
    /// completed run — `"success"` normally, or a short systemd keyword
    /// (`"exit-code"`, `"signal"`, `"timeout"`, `"start-limit-hit"`, ...)
    /// describing how it last failed. This is systemd's own record, not
    /// re-derived from `is_failed`/`is_active`: it is the one place
    /// "how did the last run actually end" is exposed, including for a
    /// unit that is currently active/healthy again after a past failure
    /// (`is_failed` alone cannot tell that story once the unit is
    /// running fine — `Result=` still can, until the next run overwrites
    /// it). `None` if unavailable.
    pub fn last_result(&self) -> Option<String> {
        self.show_value("Result")
    }

    /// Human-readable next scheduled elapse time for a `.timer` unit,
    /// exactly as systemd formats it (`systemctl show -p
    /// NextElapseUSecRealtime`) — e.g. `"Mon 2024-01-01 12:00:00 UTC"`.
    /// Only meaningful when this manager was constructed with a `.timer`
    /// unit name (see `unit_name`'s doc comment); calling it against a
    /// `.service` name will simply report unavailable, since services
    /// don't have this property. `None` if the timer has no pending
    /// elapse (disabled/inactive) or is otherwise unavailable.
    pub fn timer_next_elapse(&self) -> Option<String> {
        self.show_value("NextElapseUSecRealtime")
    }

    fn reload_or_restart(&self) -> Result<(), String> {
        // sing-box has no in-place reload, so every authorization change is a
        // deliberate restart. systemd counts those against the unit's
        // StartLimitBurst, which exists to stop a crash loop, not an operator:
        // once a burst of changes trips it the unit is parked `failed` and
        // every further restart is refused until a watchdog runs minutes
        // later. Clearing the counter first keeps administrative restarts out
        // of the crash-loop budget; automatic `Restart=on-failure` restarts
        // still count, so real crash loops stay bounded. Best effort: on a
        // unit that is not failed this is a no-op.
        let _ = self.run(&["reset-failed", &self.service_name]);
        let output = self
            .run(&["reload-or-restart", &self.service_name])
            .map_err(|e| format!("failed to invoke systemctl: {e}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl reload-or-restart {} failed: {}",
                self.service_name,
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    pub fn is_active(&self) -> bool {
        self.run(&["is-active", "--quiet", &self.service_name])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// True only if systemd is actually holding this unit in `failed`
    /// state right now (persists until the next successful run or an
    /// explicit `systemctl reset-failed`, so a caller polling this
    /// occasionally — e.g. `vpn doctor` — will still see a failure that
    /// happened between polls). Meant for oneshot units like
    /// `vpn-expiry-reconcile.service`, where `is_active` is not the
    /// right question (a successful oneshot run is briefly "active" and
    /// then normally "inactive (dead)", never "active" at rest — only
    /// "failed" is the durable, checkable signal). `false` on any
    /// systemctl/spawn problem — this must never itself manufacture a
    /// failure report out of "couldn't check."
    pub fn is_failed(&self) -> bool {
        self.run(&["is-failed", "--quiet", &self.service_name])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Reload/restart the service, then verify it actually came back up
    /// healthy. Returns Err (without panicking or leaving the caller
    /// guessing) if either step fails — the caller is responsible for
    /// restoring the previous config and retrying on failure (see
    /// `apps/admin/src/main.rs::regenerate_singbox_config`).
    pub fn reload_and_verify(&self) -> Result<(), String> {
        self.reload_or_restart()?;
        // A single `is-active` immediately after reload can catch the unit
        // during its brief active window before ExecStart/health failure
        // makes it exit. Require three consecutive healthy observations
        // within a bounded number of observations.
        //
        // The bound is a COUNT, not a wall-clock deadline: each observation is
        // a real subprocess (`systemctl is-active`) whose own latency grows on
        // a loaded host, and a deadline that subprocess time consumes turns
        // "slow to ask" into "not healthy" (I-75: 3 spawns could not fit in
        // settle*10, so a healthy unit was reported failed and a legitimate
        // change was rolled back). With a count, a healthy unit is confirmed
        // after exactly three polls however slow they are, and an unhealthy
        // one is abandoned after MAX_POLLS polls.
        const MAX_POLLS: u32 = 10;
        let mut consecutive_active = 0u8;
        for _ in 0..MAX_POLLS {
            std::thread::sleep(self.settle);
            if self.is_active() {
                consecutive_active += 1;
                if consecutive_active == 3 {
                    return Ok(());
                }
            } else {
                consecutive_active = 0;
            }
        }
        Err(format!(
            "{} did not remain active after reload",
            self.service_name
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// Writes a fake `systemctl` shell script to a temp dir that logs
    /// its invocation and exits with the given status, so reload/verify
    /// behavior can be tested without a real systemd host.
    fn fake_systemctl(dir: &std::path::Path, is_active_exit: i32, reload_exit: i32) -> PathBuf {
        let path = dir.join("systemctl");
        let script = format!(
            r#"#!/usr/bin/env bash
case "$1" in
  --version) exit 0 ;;
  reload-or-restart) exit {reload_exit} ;;
  is-active) exit {is_active_exit} ;;
  show) echo "loaded"; exit 0 ;;
esac
exit 1
"#
        );
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        drop(f); // close the write handle before chmod/exec — see `run()`'s ETXTBSY retry.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn reload_and_verify_succeeds_when_service_comes_up_healthy() {
        let dir = tempfile::tempdir().unwrap();
        let systemctl = fake_systemctl(dir.path(), 0, 0);
        // `reload_and_verify` requires 3 consecutive successful
        // `is-active` subprocess spawns within `settle * 10`. A 1ms
        // settle (10ms total budget) was observed flaky on a loaded CI
        // runner — 3 real `fork`/`exec`/bash-script/`exit` round trips
        // don't reliably fit in 10ms under contention. 20ms (200ms
        // total) keeps this fast for a unit test while giving real
        // subprocess spawns enough headroom.
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(20));
        assert!(mgr.is_available());
        assert!(mgr.reload_and_verify().is_ok());
    }

    #[test]
    fn reload_and_verify_fails_when_reload_command_fails() {
        let dir = tempfile::tempdir().unwrap();
        let systemctl = fake_systemctl(dir.path(), 0, 1);
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(1));
        assert!(mgr.reload_and_verify().is_err());
    }

    #[test]
    fn reload_and_verify_fails_when_service_not_active_after_reload() {
        let dir = tempfile::tempdir().unwrap();
        let systemctl = fake_systemctl(dir.path(), 3, 0);
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(1));
        let err = mgr.reload_and_verify().unwrap_err();
        assert!(err.contains("did not remain active"));
    }

    /// A fake `systemctl` that models systemd's start-rate limiter
    /// (`StartLimitBurst`): after `limit` starts without an intervening
    /// `reset-failed`, `reload-or-restart` is refused ("start request repeated
    /// too quickly") and the unit is parked failed. `reset-failed` clears it.
    fn fake_systemctl_with_start_limit(dir: &std::path::Path, limit: u32) -> PathBuf {
        let path = dir.join("systemctl");
        let counter = dir.join("starts");
        let script = format!(
            r#"#!/usr/bin/env bash
COUNTER="{counter}"
[ -f "$COUNTER" ] || echo 0 > "$COUNTER"
case "$1" in
  --version) exit 0 ;;
  show) echo "loaded"; exit 0 ;;
  reset-failed) echo 0 > "$COUNTER"; exit 0 ;;
  reload-or-restart)
    n=$(cat "$COUNTER")
    if [ "$n" -ge {limit} ]; then
      echo "Failed to restart $2: Start request repeated too quickly." >&2
      exit 1
    fi
    echo $((n + 1)) > "$COUNTER"
    exit 0 ;;
  is-active)
    n=$(cat "$COUNTER")
    [ "$n" -gt 0 ] && [ "$n" -le {limit} ] && exit 0
    exit 3 ;;
esac
exit 1
"#,
            counter = counter.display(),
        );
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Every user mutation is a deliberate administrative restart. A burst of
    /// them (an agent reconciling several leases, an operator rotating a few
    /// credentials) must not be refused by the crash-loop limiter and leave
    /// the whole data plane down until a watchdog notices: reproduced on a
    /// real v1.3.0 host, where the 9th restart within 300 s parked sing-box
    /// `failed (start-limit-hit)` for 3 min 35 s.
    #[test]
    fn a_burst_of_administrative_reloads_is_never_refused_by_the_start_limit() {
        let dir = tempfile::tempdir().unwrap();
        let systemctl = fake_systemctl_with_start_limit(dir.path(), 8);
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(20));
        for i in 1..=20 {
            // Report WHY, not just that: "Start request repeated too quickly" is the limiter (a product defect);
            // "did not remain active" is the verify window (3 consecutive is-active spawns within settle*10) being
            // missed on a loaded host (test timing).
            let r = mgr.reload_and_verify();
            assert!(
                r.is_ok(),
                "reload {i} of a rapid burst was refused by the start-rate limiter (error: {:?})",
                r.err()
            );
        }
    }

    /// A fake `systemctl` whose `is-active` takes `delay_ms` (a loaded host's fork/exec latency, made deterministic) and answers
    /// per `answers` (one char per call: `1` active, `0` not; the last char repeats). Every `is-active` call is counted.
    fn fake_systemctl_slow_is_active(
        dir: &std::path::Path,
        delay_ms: u32,
        answers: &str,
    ) -> (PathBuf, PathBuf) {
        let path = dir.join("systemctl");
        let calls = dir.join("is-active-calls");
        let delay = format!("{}.{:03}", delay_ms / 1000, delay_ms % 1000);
        let script = format!(
            r#"#!/usr/bin/env bash
CALLS="{calls}"
case "$1" in
  --version) exit 0 ;;
  show) echo "loaded"; exit 0 ;;
  reset-failed|reload-or-restart) exit 0 ;;
  is-active)
    n=$(cat "$CALLS" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$CALLS"
    sleep {delay}
    a="{answers}"; i=$((n - 1)); [ "$i" -ge "${{#a}}" ] && i=$((${{#a}} - 1))
    [ "${{a:$i:1}}" = 1 ] && exit 0
    exit 3 ;;
esac
exit 1
"#,
            calls = calls.display(),
            delay = delay,
            answers = answers,
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (path, calls)
    }

    fn polls(calls: &std::path::Path) -> u32 {
        std::fs::read_to_string(calls)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0)
    }

    /// I-75, made deterministic: the verify budget must be a number of OBSERVATIONS, not a wall-clock deadline that the
    /// subprocess latency itself consumes. A healthy unit whose `is-active` takes 150 ms (a loaded host) used to be reported
    /// "did not remain active" because 3 spawns could not fit in settle*10 (200 ms at a 20 ms settle).
    #[test]
    fn slow_is_active_spawns_do_not_make_a_healthy_service_fail_verification() {
        let dir = tempfile::tempdir().unwrap();
        let (systemctl, calls) = fake_systemctl_slow_is_active(dir.path(), 150, "1");
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(20));
        assert_eq!(mgr.reload_and_verify(), Ok(()));
        assert_eq!(
            polls(&calls),
            3,
            "a healthy unit is confirmed by exactly three consecutive observations"
        );
    }

    /// The bound still holds: a unit that never comes up is given up on after a fixed number of observations (10),
    /// however slow or fast each observation is.
    #[test]
    fn a_unit_that_never_comes_up_is_abandoned_after_a_fixed_number_of_observations() {
        let dir = tempfile::tempdir().unwrap();
        let (systemctl, calls) = fake_systemctl_slow_is_active(dir.path(), 30, "0");
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(5));
        let err = mgr.reload_and_verify().unwrap_err();
        assert!(err.contains("did not remain active"), "{err}");
        assert_eq!(polls(&calls), 10);
    }

    /// Flapping is not health: active/inactive alternation never produces three CONSECUTIVE healthy observations.
    #[test]
    fn a_flapping_unit_never_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let (systemctl, calls) = fake_systemctl_slow_is_active(dir.path(), 0, "11010");
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(5));
        assert!(mgr.reload_and_verify().is_err());
        assert_eq!(polls(&calls), 10);
    }

    /// A unit that needs a few observations to start is still accepted inside the bound.
    #[test]
    fn a_slow_starter_is_accepted_within_the_bound() {
        let dir = tempfile::tempdir().unwrap();
        let (systemctl, calls) = fake_systemctl_slow_is_active(dir.path(), 10, "0001");
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(5));
        assert_eq!(mgr.reload_and_verify(), Ok(()));
        assert_eq!(polls(&calls), 6);
    }

    /// `systemctl is-failed` exits 0 when the unit IS in `failed` state
    /// and non-zero otherwise — mirrors real systemd's own contract.
    #[test]
    fn is_failed_true_when_systemctl_reports_failed() {
        // `systemctl is-failed` exits 0 when the unit IS failed. Use the
        // platform's immutable success binary so this status-mapping test
        // cannot intermittently hit ETXTBSY while executing a just-written
        // temporary shell script on a loaded CI filesystem.
        let mgr = CompatibilityServiceManager::new("vpn-expiry-reconcile")
            .with_systemctl_binary(PathBuf::from("/bin/true"));
        assert!(mgr.is_failed());
    }

    #[test]
    fn is_failed_false_when_systemctl_reports_not_failed() {
        // Any non-zero `systemctl is-failed` status means the unit is not in
        // failed state; /bin/false deterministically supplies that status.
        let mgr = CompatibilityServiceManager::new("vpn-expiry-reconcile")
            .with_systemctl_binary(PathBuf::from("/bin/false"));
        assert!(!mgr.is_failed());
    }

    #[test]
    fn is_available_is_false_for_nonexistent_binary() {
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(PathBuf::from("/nonexistent/systemctl"));
        assert!(!mgr.is_available());
    }

    /// A fake `systemctl` that answers `show -p <PROPERTY> --value <unit>`
    /// with a caller-supplied value for exactly one property (empty
    /// string simulates "unavailable"/no such property) and fails
    /// everything else. Also records every invocation's raw argv (one
    /// call per line) to `log_path`, so a test can assert exactly which
    /// unit name was actually queried — the whole point of the
    /// `is_unit_installed`/`unit_name` fix is that a `.timer`-suffixed
    /// name must reach `systemctl` unchanged, never re-suffixed to
    /// `.timer.service`.
    fn fake_systemctl_show_property(
        dir: &std::path::Path,
        log_path: &std::path::Path,
        property: &str,
        value: &str,
    ) -> PathBuf {
        let path = dir.join("systemctl");
        let script = format!(
            r#"#!/usr/bin/env bash
echo "$*" >> "{log}"
case "$1" in
  --version) exit 0 ;;
  show)
    if [ "$3" = "{property}" ]; then
      echo "{value}"
      exit 0
    fi
    exit 1
    ;;
esac
exit 1
"#,
            log = log_path.display(),
        );
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn is_unit_installed_uses_a_dotted_unit_name_verbatim_for_a_timer() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("calls.log");
        let systemctl = fake_systemctl_show_property(dir.path(), &log_path, "LoadState", "loaded");
        let mgr = CompatibilityServiceManager::new("vpn-expiry-reconcile.timer")
            .with_systemctl_binary(systemctl);
        assert!(
            mgr.is_unit_installed(),
            "expected a .timer unit reporting LoadState=loaded to be seen as installed"
        );
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("vpn-expiry-reconcile.timer") && !log.contains("vpn-expiry-reconcile.timer.service"),
            "the .timer unit name must reach systemctl verbatim, never re-suffixed with .service: {log}"
        );
    }

    #[test]
    fn is_unit_installed_still_appends_service_for_a_bare_name() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("calls.log");
        let systemctl = fake_systemctl_show_property(dir.path(), &log_path, "LoadState", "loaded");
        let mgr = CompatibilityServiceManager::new("sing-box").with_systemctl_binary(systemctl);
        assert!(mgr.is_unit_installed());
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("sing-box.service"),
            "a bare unit name must still be queried as <name>.service, unchanged from before this fix: {log}"
        );
    }

    #[test]
    fn restart_count_parses_nrestarts() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("calls.log");
        let systemctl = fake_systemctl_show_property(dir.path(), &log_path, "NRestarts", "3");
        let mgr = CompatibilityServiceManager::new("sing-box").with_systemctl_binary(systemctl);
        assert_eq!(mgr.restart_count(), Some(3));
    }

    #[test]
    fn restart_count_is_none_when_systemctl_is_unavailable() {
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(PathBuf::from("/nonexistent/systemctl"));
        assert_eq!(mgr.restart_count(), None);
    }

    #[test]
    fn last_result_returns_the_raw_systemd_result_keyword() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("calls.log");
        let systemctl =
            fake_systemctl_show_property(dir.path(), &log_path, "Result", "start-limit-hit");
        let mgr = CompatibilityServiceManager::new("sing-box").with_systemctl_binary(systemctl);
        assert_eq!(mgr.last_result(), Some("start-limit-hit".to_string()));
    }

    #[test]
    fn last_result_is_none_when_the_value_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("calls.log");
        let systemctl = fake_systemctl_show_property(dir.path(), &log_path, "Result", "");
        let mgr = CompatibilityServiceManager::new("sing-box").with_systemctl_binary(systemctl);
        assert_eq!(mgr.last_result(), None);
    }

    #[test]
    fn timer_next_elapse_returns_systemds_own_formatted_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("calls.log");
        let systemctl = fake_systemctl_show_property(
            dir.path(),
            &log_path,
            "NextElapseUSecRealtime",
            "Mon 2024-01-01 12:00:00 UTC",
        );
        let mgr = CompatibilityServiceManager::new("vpn-expiry-reconcile.timer")
            .with_systemctl_binary(systemctl);
        assert_eq!(
            mgr.timer_next_elapse(),
            Some("Mon 2024-01-01 12:00:00 UTC".to_string())
        );
    }

    #[test]
    fn is_unit_installed_true_when_systemctl_reports_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let systemctl = fake_systemctl(dir.path(), 0, 0);
        let mgr = CompatibilityServiceManager::new("sing-box").with_systemctl_binary(systemctl);
        assert!(mgr.is_unit_installed());
    }

    #[test]
    fn is_unit_installed_false_when_systemctl_binary_missing() {
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(PathBuf::from("/nonexistent/systemctl"));
        assert!(!mgr.is_unit_installed());
    }

    #[test]
    fn override_names_the_exact_service_manager_and_never_falls_back_to_path() {
        let missing = std::ffi::OsString::from("/nonexistent/guard/systemctl");
        let binary = systemctl_binary_from(Some(missing.clone()));
        assert_eq!(binary, PathBuf::from(&missing));
        let mgr = CompatibilityServiceManager::new("sing-box").with_systemctl_binary(binary);
        assert!(
            !mgr.is_available() && !mgr.is_unit_installed(),
            "a missing guard means unavailable, not the host's systemctl"
        );
    }

    #[test]
    fn unit_tests_never_default_to_the_host_systemctl() {
        assert_eq!(
            systemctl_binary_from(None),
            PathBuf::from(DEFAULT_SYSTEMCTL)
        );
        assert!(!std::path::Path::new(DEFAULT_SYSTEMCTL).exists());
        assert_ne!(DEFAULT_SYSTEMCTL, "systemctl");
    }
}
