//! On-disk persistence for the compatibility user store and server
//! secrets. Single authoritative source (`users.json`) — spec §16: no
//! duplicate user database, sing-box config is *generated from* this, not
//! maintained in parallel.

use crate::model::CompatUser;
use crate::CompatError;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};

/// Current on-disk schema version for `users.json`. Every `users.json`
/// written before this versioning was introduced is a bare JSON array
/// (`[{...}, {...}]`) with no version marker anywhere — that legacy
/// shape is still fully understood by `load_users` (see
/// `detect_users_schema`/`UsersSchemaState::Legacy`) but is normalized
/// to the versioned envelope below on the next save, or explicitly via
/// `vpn-admin config migrate`.
pub const USERS_SCHEMA_VERSION: u32 = 1;

/// On-disk shape written by every save since versioning was introduced:
/// `{"schema_version": 1, "users": [...]}`. Never constructed directly
/// outside this module — `load_users`/`save_users_atomic` are the only
/// supported entry points.
#[derive(Serialize, Deserialize)]
struct UsersFile {
    schema_version: u32,
    users: Vec<CompatUser>,
}

/// Result of inspecting `users.json` without mutating it — used by
/// `vpn-admin config validate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsersSchemaState {
    /// File does not exist (fresh install, no users yet).
    Missing,
    /// Bare JSON array, pre-versioning — loadable, but `config migrate`
    /// has not normalized it to the versioned envelope yet.
    Legacy,
    /// Versioned envelope at `USERS_SCHEMA_VERSION` — nothing to do.
    Current,
    /// Versioned envelope at a version newer than this binary supports
    /// — refuse to load (see `load_users`).
    Future(u32),
    /// Neither the versioned envelope nor the legacy bare-array shape
    /// parses.
    Corrupted(String),
}

/// Inspect `path` without loading/mutating it. See `UsersSchemaState`.
pub fn detect_users_schema(path: &Path) -> UsersSchemaState {
    if !path.exists() {
        return UsersSchemaState::Missing;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return UsersSchemaState::Corrupted(e.to_string()),
    };
    if let Ok(versioned) = serde_json::from_slice::<UsersFile>(&bytes) {
        return if versioned.schema_version > USERS_SCHEMA_VERSION {
            UsersSchemaState::Future(versioned.schema_version)
        } else {
            UsersSchemaState::Current
        };
    }
    if serde_json::from_slice::<Vec<CompatUser>>(&bytes).is_ok() {
        return UsersSchemaState::Legacy;
    }
    UsersSchemaState::Corrupted(
        "neither the current versioned format nor the legacy bare-array format parses".to_string(),
    )
}

/// Load the user list from `path`. Missing file is treated as "no users
/// yet", not an error (fresh install). Understands both the current
/// versioned envelope and the legacy pre-versioning bare-array shape —
/// this is the read path the always-running `vpn-subscription` service
/// uses too, so it must keep working through an upgrade window without
/// requiring `vpn-admin config migrate` to run first. Refuses (fails
/// closed) only a schema version genuinely newer than this binary
/// understands, where blindly trusting the deserialized shape could be
/// wrong rather than merely "not yet normalized".
pub fn load_users(path: &Path) -> Result<Vec<CompatUser>, CompatError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path).map_err(|e| CompatError::Io(e.to_string()))?;
    parse_users_bytes(&bytes)
}

/// Parse `users.json` bytes already read from disk, understanding both
/// the current versioned envelope and the legacy pre-versioning
/// bare-array shape. Shared by `load_users` and `vpn-admin restore` (a
/// restored backup's `users/users.json` is whatever shape was live when
/// the backup was taken — possibly still the legacy shape, or the
/// current envelope — and must be accepted the same way a normal load
/// would be, not treated as a special/stricter format). Refuses (fails
/// closed) only a schema version genuinely newer than this binary
/// understands.
pub fn parse_users_bytes(bytes: &[u8]) -> Result<Vec<CompatUser>, CompatError> {
    // Try the current versioned envelope first (a JSON object); fall
    // back to the legacy bare array. The two shapes are structurally
    // disjoint at the top level, so there is no ambiguity between them.
    if let Ok(versioned) = serde_json::from_slice::<UsersFile>(bytes) {
        if versioned.schema_version > USERS_SCHEMA_VERSION {
            return Err(CompatError::UnsupportedSchema {
                what: "users.json",
                found: versioned.schema_version,
                max_supported: USERS_SCHEMA_VERSION,
            });
        }
        return Ok(versioned.users);
    }
    let legacy: Vec<CompatUser> =
        serde_json::from_slice(bytes).map_err(|e| CompatError::Parse(e.to_string()))?;
    Ok(legacy)
}

/// Atomically persist the user list in the current versioned envelope:
/// write to a sibling temp file, set mode 0640 (owner read/write, group
/// read), then rename over the target. A crash mid-write can never
/// leave `users.json` truncated or half-written. Every save normalizes
/// the on-disk shape to the current envelope, so a legacy bare-array
/// file self-heals on its next mutation even without an explicit
/// `config migrate`.
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
    let envelope = UsersFile {
        schema_version: USERS_SCHEMA_VERSION,
        users: users.to_vec(),
    };
    let json =
        serde_json::to_vec_pretty(&envelope).map_err(|e| CompatError::Parse(e.to_string()))?;
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

/// Explicitly migrate `users.json` at `path` to the current versioned
/// envelope — `vpn-admin config migrate`'s users.json half. Unlike
/// `save_users_atomic` (called by every normal user mutation, which
/// already self-heals the format), this is for an operator who wants to
/// normalize the file WITHOUT changing any user data, with an explicit
/// backup and validation step. Idempotent: a no-op if already current.
/// Refuses (leaving the file untouched) on corrupted input or a future
/// schema version — migration only ever moves forward.
pub fn migrate_users(path: &Path) -> Result<UsersMigrationOutcome, CompatError> {
    match detect_users_schema(path) {
        UsersSchemaState::Missing => return Ok(UsersMigrationOutcome::Missing),
        UsersSchemaState::Current => return Ok(UsersMigrationOutcome::AlreadyCurrent),
        UsersSchemaState::Future(found) => {
            return Err(CompatError::UnsupportedSchema {
                what: "users.json",
                found,
                max_supported: USERS_SCHEMA_VERSION,
            })
        }
        UsersSchemaState::Corrupted(msg) => {
            return Err(CompatError::Parse(format!(
                "cannot migrate {path:?}: {msg}; no changes made"
            )))
        }
        UsersSchemaState::Legacy => {}
    }
    let bytes = std::fs::read(path).map_err(|e| CompatError::Io(e.to_string()))?;
    let legacy: Vec<CompatUser> =
        serde_json::from_slice(&bytes).map_err(|e| CompatError::Parse(e.to_string()))?;
    let envelope = UsersFile {
        schema_version: USERS_SCHEMA_VERSION,
        users: legacy,
    };
    let json =
        serde_json::to_vec_pretty(&envelope).map_err(|e| CompatError::Parse(e.to_string()))?;
    // Validate before touching anything live: the migrated bytes must
    // reparse to the same user list we just read.
    let reparsed: UsersFile =
        serde_json::from_slice(&json).map_err(|e| CompatError::Parse(e.to_string()))?;
    if reparsed.schema_version != USERS_SCHEMA_VERSION
        || reparsed.users.len() != envelope.users.len()
    {
        return Err(CompatError::Parse(
            "migrated users.json failed to reparse to the expected shape — this is a bug, not applying"
                .to_string(),
        ));
    }
    let backup_path = crate::migrate::backup_before_mutate(path)?;
    crate::migrate::atomic_write(path, &json, 0o640)?;
    Ok(UsersMigrationOutcome::Migrated { backup_path })
}

/// Outcome of `migrate_users`. See `DeploymentMigrationOutcome` for the
/// `deployment.toml` equivalent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsersMigrationOutcome {
    Missing,
    AlreadyCurrent,
    Migrated { backup_path: PathBuf },
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

    // --- schema versioning / migration ---

    #[test]
    fn save_writes_the_versioned_envelope_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        save_users_atomic(&path, &[sample_user("u1")]).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["schema_version"], USERS_SCHEMA_VERSION);
        assert_eq!(raw["users"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn detect_schema_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        assert_eq!(detect_users_schema(&path), UsersSchemaState::Missing);
    }

    #[test]
    fn detect_schema_current_after_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        save_users_atomic(&path, &[sample_user("u1")]).unwrap();
        assert_eq!(detect_users_schema(&path), UsersSchemaState::Current);
    }

    #[test]
    fn detect_schema_legacy_bare_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        std::fs::write(&path, serde_json::to_vec(&[sample_user("u1")]).unwrap()).unwrap();
        assert_eq!(detect_users_schema(&path), UsersSchemaState::Legacy);
    }

    #[test]
    fn detect_schema_future_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let raw = serde_json::json!({"schema_version": 99, "users": []});
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(detect_users_schema(&path), UsersSchemaState::Future(99));
    }

    #[test]
    fn detect_schema_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        std::fs::write(&path, b"{not valid json at all").unwrap();
        assert!(matches!(
            detect_users_schema(&path),
            UsersSchemaState::Corrupted(_)
        ));
    }

    #[test]
    fn load_users_reads_legacy_bare_array_transparently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let users = vec![sample_user("u1"), sample_user("u2")];
        std::fs::write(&path, serde_json::to_vec(&users).unwrap()).unwrap();
        let loaded = load_users(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "u1");
    }

    #[test]
    fn load_users_refuses_future_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let raw = serde_json::json!({"schema_version": 99, "users": []});
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let err = load_users(&path).unwrap_err();
        assert!(matches!(
            err,
            CompatError::UnsupportedSchema { found: 99, .. }
        ));
    }

    #[test]
    fn load_users_refuses_corrupted_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        std::fs::write(&path, b"{not valid json at all").unwrap();
        assert!(load_users(&path).is_err());
    }

    #[test]
    fn migrate_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        assert_eq!(
            migrate_users(&path).unwrap(),
            UsersMigrationOutcome::Missing
        );
    }

    #[test]
    fn migrate_already_current_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        save_users_atomic(&path, &[sample_user("u1")]).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            migrate_users(&path).unwrap(),
            UsersMigrationOutcome::AlreadyCurrent
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "no-op must not rewrite the file"
        );
    }

    #[test]
    fn migrate_legacy_bare_array_end_to_end_preserves_users_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let users = vec![sample_user("u1"), sample_user("u2")];
        let legacy_bytes = serde_json::to_vec_pretty(&users).unwrap();
        std::fs::write(&path, &legacy_bytes).unwrap();

        let outcome = migrate_users(&path).unwrap();
        let backup_path = match outcome {
            UsersMigrationOutcome::Migrated { backup_path } => backup_path,
            other => panic!("expected Migrated, got {other:?}"),
        };
        assert!(backup_path.exists());
        assert_eq!(std::fs::read(&backup_path).unwrap(), legacy_bytes);

        assert_eq!(detect_users_schema(&path), UsersSchemaState::Current);
        let loaded = load_users(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "u1");
        assert_eq!(loaded[0].vless_uuid, users[0].vless_uuid);
        assert_eq!(loaded[1].id, "u2");

        // idempotent: running again is a no-op
        assert_eq!(
            migrate_users(&path).unwrap(),
            UsersMigrationOutcome::AlreadyCurrent
        );
    }

    #[test]
    fn migrate_refuses_corrupted_input_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let corrupted = b"{not valid json at all".to_vec();
        std::fs::write(&path, &corrupted).unwrap();
        assert!(migrate_users(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), corrupted);
    }

    #[test]
    fn migrate_refuses_future_schema_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        let raw = serde_json::json!({"schema_version": 99, "users": []});
        let bytes = serde_json::to_vec(&raw).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let err = migrate_users(&path).unwrap_err();
        assert!(matches!(
            err,
            CompatError::UnsupportedSchema { found: 99, .. }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn migrate_backup_is_mode_0600_at_least_as_strict_as_live_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users.json");
        std::fs::write(&path, serde_json::to_vec(&[sample_user("u1")]).unwrap()).unwrap();
        let outcome = migrate_users(&path).unwrap();
        let backup_path = match outcome {
            UsersMigrationOutcome::Migrated { backup_path } => backup_path,
            other => panic!("expected Migrated, got {other:?}"),
        };
        let mode = std::fs::metadata(&backup_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
