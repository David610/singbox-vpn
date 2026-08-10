//! On-disk persistence for the compatibility user store and server
//! secrets. Single authoritative source (`users.json`) — spec §16: no
//! duplicate user database, sing-box config is *generated from* this, not
//! maintained in parallel.

use crate::model::CompatUser;
use crate::CompatError;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;

/// Load the user list from `path`. Missing file is treated as "no users
/// yet", not an error (fresh install).
pub fn load_users(path: &Path) -> Result<Vec<CompatUser>, CompatError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path).map_err(|e| CompatError::Io(e.to_string()))?;
    let users: Vec<CompatUser> =
        serde_json::from_slice(&bytes).map_err(|e| CompatError::Parse(e.to_string()))?;
    Ok(users)
}

/// Atomically persist the user list: write to a sibling temp file, set
/// mode 0640 (owner read/write, group read), then rename over the
/// target. A crash mid-write can never leave `users.json` truncated or
/// half-written.
///
/// Mode 0640 (not 0600) is a documented, intentional exception to the
/// "secrets are 0600" default (spec §11): the subscription service must
/// read this file to verify tokens and render per-user credentials, and
/// runs as a dedicated non-root `vpn-subscription` service user rather
/// than root — see `deploy/almalinux/install.sh` (`vpn-subscription`
/// group ownership) and `docs/ALMALINUX_DEPLOYMENT.md`. `vpn-admin`
/// (the only writer) still runs as root; only read access is shared.
pub fn save_users_atomic(path: &Path, users: &[CompatUser]) -> Result<(), CompatError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CompatError::Io(e.to_string()))?;
        set_dir_mode_0750(parent)?;
    }
    let json = serde_json::to_vec_pretty(users).map_err(|e| CompatError::Parse(e.to_string()))?;
    let tmp_path = tmp_sibling_path(path);
    write_file_mode_0640(&tmp_path, &json)?;
    // rename() never changes ownership — without this, every save after
    // install silently turns `users.json` back into root:root and breaks
    // vpn-subscription's read access (docs/FINAL_PRODUCTION_AUDIT.md P0-2).
    crate::ownership::preserve_ownership_before_rename(&tmp_path, path)?;
    std::fs::rename(&tmp_path, path).map_err(|e| CompatError::Io(e.to_string()))?;
    fsync_parent_dir(path);
    Ok(())
}

fn tmp_sibling_path(path: &Path) -> std::path::PathBuf {
    let mut tmp = path.to_path_buf();
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("store");
    tmp.set_file_name(format!("{file_name}.tmp.{}", std::process::id()));
    tmp
}

#[cfg(unix)]
fn write_file_mode_0640(path: &Path, bytes: &[u8]) -> Result<(), CompatError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o640)
        .open(path)
        .map_err(|e| CompatError::Io(e.to_string()))?;
    f.write_all(bytes)
        .map_err(|e| CompatError::Io(e.to_string()))?;
    f.sync_all().map_err(|e| CompatError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(not(unix))]
fn write_file_mode_0640(path: &Path, bytes: &[u8]) -> Result<(), CompatError> {
    let mut file = std::fs::File::create(path).map_err(|e| CompatError::Io(e.to_string()))?;
    std::io::Write::write_all(&mut file, bytes).map_err(|e| CompatError::Io(e.to_string()))?;
    file.sync_all().map_err(|e| CompatError::Io(e.to_string()))
}

#[cfg(unix)]
fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn fsync_parent_dir(_path: &Path) {}

#[cfg(unix)]
fn set_dir_mode_0750(path: &Path) -> Result<(), CompatError> {
    use std::os::unix::fs::PermissionsExt;
    // 0750, not 0700: matches the file mode exception above — the
    // `vpn-subscription` group needs to traverse this directory to read
    // `users.json`. Group ownership itself is set once by
    // `deploy/almalinux/install.sh`, not by this process.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750))
        .map_err(|e| CompatError::Io(e.to_string()))
}

#[cfg(not(unix))]
fn set_dir_mode_0750(_path: &Path) -> Result<(), CompatError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CompatUser;
    use crate::secret::SecretString;

    fn sample_user(id: &str) -> CompatUser {
        CompatUser {
            id: id.into(),
            name: "test".into(),
            enabled: true,
            vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
            hysteria2_password: SecretString::new("pw"),
            subscription_token_hash_hex: "deadbeef".into(),
            created_at: 0,
            expires_at: None,
        }
    }

    #[test]
    fn missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let users = load_users(&path).unwrap();
        assert!(users.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let users = vec![sample_user("u1"), sample_user("u2")];
        save_users_atomic(&path, &users).unwrap();
        let loaded = load_users(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "u1");
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_read_write_group_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        save_users_atomic(&path, &[sample_user("u1")]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn repeated_saves_preserve_group_ownership_across_mutations() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        save_users_atomic(&path, &[sample_user("u1")]).unwrap();
        let original_gid = std::fs::metadata(&path).unwrap().gid();

        for i in 0..5 {
            let users = vec![sample_user(&format!("u{i}"))];
            save_users_atomic(&path, &users).unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            assert_eq!(meta.gid(), original_gid, "gid drifted on mutation {i}");
            assert_eq!(meta.permissions().mode() & 0o777, 0o640);
        }
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover.is_empty());
    }

    #[test]
    fn save_is_atomic_no_partial_file_left_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        save_users_atomic(&path, &[sample_user("u1")]).unwrap();
        // no stray .tmp.* files left behind
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover.is_empty());
    }
}
