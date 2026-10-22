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

/// Creates the system groups a root-run test needs and removes exactly the
/// ones this process created when it exits (I-50). A group is "ours" only if
/// our own `groupadd` succeeded; a group that already existed, or that another
/// process created first (our `groupadd` loses the race), is never removed.
/// A group that has gained members since is left in place too. A SIGKILL
/// cannot run the exit handler; that residue is what I-41 tolerates.
#[cfg(unix)]
pub mod root_groups {
    use std::sync::Mutex;

    #[derive(Clone)]
    pub struct Tools {
        pub groupadd: String,
        pub groupdel: String,
    }

    impl Tools {
        pub fn real() -> Self {
            Self {
                groupadd: "groupadd".into(),
                groupdel: "groupdel".into(),
            }
        }
    }

    struct State {
        tools: Option<Tools>,
        created: Vec<String>,
    }

    /// Groups created by one owner. The process-wide `REAL` registry is the
    /// only one wired to the exit handler; tests use their own instances.
    pub struct Registry(Mutex<State>);

    impl Registry {
        pub const fn new() -> Self {
            Self(Mutex::new(State {
                tools: None,
                created: Vec::new(),
            }))
        }
    }

    pub static REAL: Registry = Registry::new();

    pub fn real_group_exists(group: &str) -> bool {
        let name = std::ffi::CString::new(group).unwrap();
        !unsafe { libc::getgrnam(name.as_ptr()) }.is_null()
    }

    fn real_group_has_members(group: &str) -> bool {
        let name = std::ffi::CString::new(group).unwrap();
        let entry = unsafe { libc::getgrnam(name.as_ptr()) };
        !entry.is_null() && unsafe { !(*(*entry).gr_mem).is_null() }
    }

    pub fn ensure(reg: &Registry, tools: &Tools, exists: &dyn Fn(&str) -> bool, groups: &[&str]) {
        for group in groups {
            if exists(group) {
                continue;
            }
            let status = std::process::Command::new(&tools.groupadd)
                .args(["--system", group])
                .status();
            if matches!(status, Ok(s) if s.success()) {
                let mut state = reg.0.lock().unwrap();
                state.tools = Some(tools.clone());
                state.created.push((*group).to_string());
                drop(state);
                if std::ptr::eq(reg, &REAL) {
                    register_exit_cleanup();
                }
            } else if exists(group) {
                // Another process created it between our check and groupadd:
                // it is not ours to remove.
                continue;
            } else {
                panic!(
                    "test running as root could not create the {group:?} system group needed \
                     to exercise the real restore ownership path: {status:?}"
                );
            }
        }
    }

    pub fn register_exit_cleanup() {
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        extern "C" fn at_exit() {
            remove_created(&REAL, &real_group_has_members);
        }
        REGISTERED.call_once(|| unsafe {
            libc::atexit(at_exit);
        });
    }

    pub fn remove_created(reg: &Registry, has_members: &dyn Fn(&str) -> bool) {
        let Ok(mut state) = reg.0.lock() else { return };
        let Some(tools) = state.tools.clone() else {
            return;
        };
        for group in std::mem::take(&mut state.created) {
            if has_members(&group) {
                continue;
            }
            let _ = std::process::Command::new(&tools.groupdel)
                .arg(&group)
                .status();
        }
    }
}

/// Root-run suites need the `vpn-subscription` and `sing-box` system groups
/// for the real restore ownership path. Every test binary that restores
/// calls this itself: relying on another binary to have left the groups
/// behind (the I-50 leak) made `relay_cli` pass only by accident.
#[cfg(unix)]
pub fn ensure_service_groups_for_root_tests() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let tools = root_groups::Tools::real();
    root_groups::ensure(
        &root_groups::REAL,
        &tools,
        &root_groups::real_group_exists,
        &["vpn-subscription", "sing-box"],
    );
}
#[cfg(not(unix))]
pub fn ensure_service_groups_for_root_tests() {}
