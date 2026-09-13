//! Host isolation for every test that runs the compiled `vpn-admin`.
//!
//! These tests are also run by `update.sh --repair`/`--dev-rebuild` on a
//! live server, as root, where the real `systemctl` would act on the real
//! `sing-box`/`vpn-subscription` units. That happened during the real
//! two-VPS acceptance (defect D6): tests that did not install a fake
//! `systemctl` restarted the live relay until systemd's start limit
//! tripped. So no test may start the binary without this module:
//!
//! * `SINGBOX_VPN_SYSTEMCTL` points at a guard that refuses every call and
//!   records mutating ones — the binary never resolves `systemctl` from
//!   `PATH` while it is set, so a test that forgets to install a fake gets
//!   "systemd not available", never the host's service manager;
//! * `SINGBOX_VPN_REPAIR_UPDATE_SH` points at a guard, so `vpn repair`
//!   can never run the real updater;
//! * `PATH` starts with guards for the host-control tools a helper script
//!   could reach (`systemctl`, `service`, reboot/shutdown, account tools).
//!
//! A test that needs service control installs its own fake by overriding
//! `SINGBOX_VPN_SYSTEMCTL` explicitly. `every_binary_invocation_is_guarded`
//! in `cli.rs` enforces that nothing bypasses these constructors.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

/// Tools a test must never run against the host. Each guard exits 97.
const GUARDED_TOOLS: &[&str] = &[
    "systemctl",
    "service",
    "reboot",
    "shutdown",
    "poweroff",
    "halt",
    "useradd",
    "userdel",
    "usermod",
    "groupadd",
    "groupdel",
];

pub struct HostGuard {
    pub bin_dir: PathBuf,
    /// One line per refused mutating call, from any test in this process.
    pub attempts_log: PathBuf,
}

/// The per-process guard directory, created on first use. Panics (fails the
/// test) if it cannot be created: there is no unguarded fallback.
pub fn host_guard() -> &'static HostGuard {
    static GUARD: OnceLock<HostGuard> = OnceLock::new();
    GUARD.get_or_init(|| {
        let root = std::env::temp_dir().join(format!(
            "singbox-vpn-test-host-guard-{}",
            std::process::id()
        ));
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).expect("create test host-guard directory");
        let attempts_log = root.join("refused-host-control.log");
        std::fs::write(&attempts_log, b"").expect("create host-guard log");
        #[cfg(unix)]
        for tool in GUARDED_TOOLS {
            use std::os::unix::fs::PermissionsExt;
            let path = bin_dir.join(tool);
            let script = format!(
                r#"#!/bin/sh
case "$1" in
  --version|show|status|is-active|is-enabled|is-failed|cat|list-units|list-timers) ;;
  *) echo "{tool} $*" >> "{log}" ;;
esac
echo "test host guard: refused '{tool} $*' (tests must never control the host)" >&2
exit 97
"#,
                log = attempts_log.display(),
            );
            std::fs::write(&path, script).expect("write host-guard tool");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make host-guard tool executable");
        }
        HostGuard {
            bin_dir,
            attempts_log,
        }
    })
}

fn guarded_path() -> std::ffi::OsString {
    let guard = host_guard();
    std::env::join_paths(
        std::iter::once(guard.bin_dir.clone()).chain(
            std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
                .unwrap_or_default(),
        ),
    )
    .expect("join guarded PATH")
}

fn guard_env() -> Vec<(&'static str, std::ffi::OsString)> {
    let guard = host_guard();
    vec![
        (
            "SINGBOX_VPN_SYSTEMCTL",
            guard.bin_dir.join("systemctl").into_os_string(),
        ),
        (
            "SINGBOX_VPN_REPAIR_UPDATE_SH",
            guard.bin_dir.join("service").into_os_string(),
        ),
        ("PATH", guarded_path()),
    ]
}

/// `assert_cmd` handle for the compiled `vpn-admin`, isolated from the host.
pub fn vpn_admin() -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("vpn-admin").expect("locate vpn-admin");
    for (key, value) in guard_env() {
        cmd.env(key, value);
    }
    cmd
}

/// `std::process::Command` for the compiled `vpn-admin` (for tests that need
/// a real child process), isolated from the host.
pub fn vpn_admin_process() -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_vpn-admin"));
    for (key, value) in guard_env() {
        cmd.env(key, value);
    }
    cmd
}

/// `std::process::Command` for another workspace binary (the subscription
/// service), isolated from the host.
pub fn guarded_process(program: impl Into<PathBuf>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program.into());
    for (key, value) in guard_env() {
        cmd.env(key, value);
    }
    cmd
}
