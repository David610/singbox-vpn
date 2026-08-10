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

impl CompatibilityServiceManager {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            systemctl_binary: PathBuf::from("systemctl"),
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

    /// True if the unit is actually installed (`systemctl` itself can be
    /// present — e.g. a GitHub Actions VM — without the `sing-box.service`
    /// unit ever having been installed by `install.sh`). Reload is
    /// skipped, not attempted-and-failed, when the unit isn't installed:
    /// this is what lets `vpn-admin render-config`/`user create` work in
    /// CI and local dev without a real systemd deployment.
    pub fn is_unit_installed(&self) -> bool {
        let unit = format!("{}.service", self.service_name);
        self.run(&["show", "-p", "LoadState", "--value", &unit])
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "loaded")
            .unwrap_or(false)
    }

    fn reload_or_restart(&self) -> Result<(), String> {
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

    /// Reload/restart the service, then verify it actually came back up
    /// healthy. Returns Err (without panicking or leaving the caller
    /// guessing) if either step fails — the caller is responsible for
    /// restoring the previous config and retrying on failure (see
    /// `apps/admin/src/main.rs::regenerate_singbox_config`).
    pub fn reload_and_verify(&self) -> Result<(), String> {
        self.reload_or_restart()?;
        // A single `is-active` immediately after reload can catch the unit
        // during its brief active window before ExecStart/health failure
        // makes it exit. Require three consecutive healthy observations and
        // allow a bounded startup window.
        let deadline = std::time::Instant::now() + self.settle * 10;
        let mut consecutive_active = 0u8;
        while std::time::Instant::now() < deadline {
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
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(systemctl)
            .with_settle(Duration::from_millis(1));
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
        assert!(err.contains("not active"));
    }

    #[test]
    fn is_available_is_false_for_nonexistent_binary() {
        let mgr = CompatibilityServiceManager::new("sing-box")
            .with_systemctl_binary(PathBuf::from("/nonexistent/systemctl"));
        assert!(!mgr.is_available());
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
}
