//! Shared backup + atomic-write primitives for the two persistent state
//! formats whose on-disk shape can evolve (`deployment.toml`,
//! `users.json`) — see `deployment::migrate_deployment_toml` and
//! `store::migrate_users`, and `vpn-admin config migrate`.
//!
//! Every migration follows the same sequence: validate the ORIGINAL
//! parses (refuse corrupted input untouched) -> compute the migrated
//! form in memory -> validate THAT parses back to an equivalent,
//! current-schema state -> back up the original (mode 0600, at least as
//! strict as any live secret-bearing state file) -> atomically replace
//! the live file. A failure at any step before the final atomic rename
//! leaves the original file completely untouched.

use crate::CompatError;
use std::path::{Path, PathBuf};

/// Copy `path` to a sibling `<name>.pre-migration-<unix_ts>.bak` file
/// BEFORE any mutation, mode 0600 — same-or-stricter than any live state
/// file this crate writes (`users.json` is 0640; `deployment.toml` is
/// operator-owned but may contain no secrets itself, so 0600 is a safe
/// default for a backup of either). Fsynced before returning, so a crash
/// immediately after backup can never leave a truncated one.
pub fn backup_before_mutate(path: &Path) -> Result<PathBuf, CompatError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    let mut backup_path = path.to_path_buf();
    backup_path.set_file_name(format!("{file_name}.pre-migration-{ts}.bak"));
    let bytes = std::fs::read(path).map_err(|e| CompatError::Io(e.to_string()))?;
    write_file_mode(&backup_path, &bytes, 0o600)?;
    Ok(backup_path)
}

/// Atomically replace `path`'s contents with `bytes` at the given mode:
/// write to a sibling temp file, preserve the live target's ownership
/// onto it (see `ownership.rs` — `rename` never does this itself), then
/// rename over the target. A crash mid-write can never leave `path`
/// truncated or half-written.
pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), CompatError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CompatError::Io(e.to_string()))?;
    }
    let mut tmp = path.to_path_buf();
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    tmp.set_file_name(format!("{file_name}.tmp.{}", std::process::id()));
    write_file_mode(&tmp, bytes, mode)?;
    crate::ownership::preserve_ownership_before_rename(&tmp, path)?;
    std::fs::rename(&tmp, path).map_err(|e| CompatError::Io(e.to_string()))?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

#[cfg(unix)]
fn write_file_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<(), CompatError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)
        .map_err(|e| CompatError::Io(e.to_string()))?;
    f.write_all(bytes)
        .map_err(|e| CompatError::Io(e.to_string()))?;
    f.sync_all().map_err(|e| CompatError::Io(e.to_string()))
}

#[cfg(not(unix))]
fn write_file_mode(path: &Path, bytes: &[u8], _mode: u32) -> Result<(), CompatError> {
    let mut file = std::fs::File::create(path).map_err(|e| CompatError::Io(e.to_string()))?;
    std::io::Write::write_all(&mut file, bytes).map_err(|e| CompatError::Io(e.to_string()))?;
    file.sync_all().map_err(|e| CompatError::Io(e.to_string()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn backup_is_mode_0600_regardless_of_source_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let backup = backup_before_mutate(&path).unwrap();
        let mode = std::fs::metadata(&backup).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&backup).unwrap(), b"{}");
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, b"hello", 0o640).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover.is_empty());
    }
}
