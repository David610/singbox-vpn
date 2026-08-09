//! Process-level locking for state-changing `vpn-admin` operations
//! (docs/FINAL_PRODUCTION_AUDIT.md P0-4). Every mutating command
//! (`user create/disable/enable/remove/rotate-*`, `init --rotate`,
//! `restore`, `render-config`) must hold this lock for the ENTIRE
//! load -> mutate -> persist -> apply -> reload -> verify sequence, not
//! just the file write — otherwise two concurrent `vpn-admin` invocations
//! can each read the same starting `users.json`, mutate independently,
//! and have the second writer silently discard the first writer's
//! change (each individual file write is already atomic — see
//! `compat_config::store`/`server` — but that does not make the
//! load-then-write sequence as a whole atomic).
//!
//! Uses a plain advisory `flock(2)` on a well-known path
//! (`/run/lock/vpn1.lock` in production), held for the lifetime of the
//! returned guard. `flock` blocks until the lock is available rather
//! than failing immediately — a concurrent admin invocation should wait
//! its turn, not lose its change or race — with `flock`'s well-defined
//! at-most-one-holder semantics doing the actual mutual exclusion.
use anyhow::{Context, Result};
use std::fs::File;
use std::path::PathBuf;

/// Overridable for tests (each test uses an isolated temp directory and
/// must not contend with other tests' or the host's real
/// `/run/lock/vpn1.lock`). Production installs never set this — the
/// installer/systemd units run `vpn-admin` with a normal environment, so
/// the default applies.
pub fn lock_path() -> PathBuf {
    std::env::var_os("VPN1_LOCK_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/lock/vpn1.lock"))
}

/// Held for as long as this value is alive; the OS releases the flock
/// automatically when the underlying file descriptor is closed (i.e. on
/// `Drop`), so there is no separate explicit unlock step to forget.
pub struct StateLock {
    _file: File,
}

#[cfg(unix)]
pub fn acquire_state_lock() -> Result<StateLock> {
    use std::os::unix::io::AsRawFd;
    let path = lock_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating lock directory {parent:?}"))?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening lock file {path:?}"))?;
    // SAFETY: `file`'s fd is valid for the duration of this call, and
    // `flock` does not take ownership of it — it only registers an
    // advisory lock associated with the open file description, which is
    // released when the fd is closed (i.e. when `file`, held inside the
    // returned `StateLock`, is dropped).
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("acquiring exclusive lock on {path:?}"));
    }
    Ok(StateLock { _file: file })
}

#[cfg(not(unix))]
pub fn acquire_state_lock() -> Result<StateLock> {
    // No advisory-locking equivalent wired up for non-unix targets; vpn1
    // only ships for systemd Linux hosts (see deploy/lib/os.sh), so this
    // is intentionally a no-op rather than a hard error on platforms
    // this project doesn't deploy to (e.g. `cargo check` on a dev
    // laptop).
    Ok(StateLock {
        _file: tempfile::tempfile()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_blocks_until_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");
        std::env::set_var("VPN1_LOCK_PATH", &lock_path);

        let first = acquire_state_lock().unwrap();

        // A second, non-blocking attempt from a helper thread must not
        // succeed while `first` is held — proven here via a raw
        // try-lock (LOCK_EX | LOCK_NB) rather than blocking the test.
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let f = std::fs::File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&lock_path)
                .unwrap();
            let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            assert_ne!(rc, 0, "second exclusive lock attempt must fail while held");
        }

        drop(first);

        // Once released, a fresh acquire must succeed immediately.
        let second = acquire_state_lock().unwrap();
        drop(second);
        std::env::remove_var("VPN1_LOCK_PATH");
    }
}
