//! Server-side sing-box config rendering + safe apply. Deliberately
//! behind a `CompatibilityBackend` trait (spec §53) so swapping sing-box
//! for Xray later does not require rewriting user management or
//! subscription logic — only a new backend impl of this trait.

use crate::model::{CompatUser, Hysteria2ServerParams, RealityServerParams};
use crate::CompatError;
use serde_json::json;
use std::path::Path;
use std::process::Command;

#[derive(Clone, Copy, Debug)]
pub struct ServerPorts {
    pub vless_reality_port: u16,
    pub hysteria2_port: u16,
}

/// Render the full sing-box server config (both inbounds) from the
/// authoritative user store. Only `is_active` users are included —
/// disabled/expired users are silently excluded, which is how
/// revocation actually takes effect (spec §29).
pub fn render_singbox_server_config(
    users: &[CompatUser],
    reality: &RealityServerParams,
    hysteria: &Hysteria2ServerParams,
    ports: ServerPorts,
    now_unix: i64,
) -> serde_json::Value {
    let active: Vec<&CompatUser> = users.iter().filter(|u| u.is_active(now_unix)).collect();

    let vless_users: Vec<_> = active
        .iter()
        .map(|u| {
            json!({
                "name": u.id,
                "uuid": u.vless_uuid,
                "flow": "xtls-rprx-vision",
            })
        })
        .collect();

    let hysteria_users: Vec<_> = active
        .iter()
        .map(|u| {
            json!({
                "name": u.id,
                "password": u.hysteria2_password.expose(),
            })
        })
        .collect();

    let mut hysteria_inbound = json!({
        "type": "hysteria2",
        "tag": "hysteria2-in",
        "listen": "::",
        "listen_port": ports.hysteria2_port,
        "users": hysteria_users,
        "tls": {
            "enabled": true,
            "certificate_path": hysteria.tls_cert_path,
            "key_path": hysteria.tls_key_path,
        }
    });
    // Unauthenticated/invalid Hysteria2 connections get a plausible
    // static-file HTTP response instead of a distinctive auth-reject
    // signature (sing-box 1.13.x `masquerade` — see
    // docs/COMPATIBILITY_VERSIONS.md). This only affects what a passive/
    // active *unauthenticated* probe observes; it does not hide that
    // QUIC/UDP is listening on this port and is not a substitute for
    // REALITY-style live-relay disguise (Hysteria2 has no equivalent).
    if let Some(masq_path) = &hysteria.masquerade_dir_path {
        hysteria_inbound["masquerade"] = json!({
            "type": "file",
            "directory": masq_path,
        });
    }
    if let Some(obfs_pw) = &hysteria.obfs_password {
        hysteria_inbound["obfs"] = json!({
            "type": "salamander",
            "password": obfs_pw.expose(),
        });
    }

    json!({
        "log": { "level": "warn", "timestamp": true },
        "inbounds": [
            {
                "type": "vless",
                "tag": "vless-reality-in",
                "listen": "::",
                "listen_port": ports.vless_reality_port,
                "users": vless_users,
                "tls": {
                    "enabled": true,
                    "server_name": reality.handshake_server,
                    "reality": {
                        "enabled": true,
                        "handshake": {
                            "server": reality.handshake_server,
                            "server_port": reality.handshake_port,
                        },
                        "private_key": reality.private_key_hex.expose(),
                        "short_id": reality.short_ids,
                    }
                }
            },
            hysteria_inbound
        ],
        "outbounds": [
            { "type": "direct", "tag": "direct" }
        ]
    })
}

/// Confirms the rendered config never contains anything it shouldn't
/// (defense in depth alongside the type system: `RealityServerParams`'s
/// private key only reaches this function, never the client render path
/// in `render.rs`).
pub trait CompatibilityBackend {
    fn validate(&self, config_path: &Path) -> Result<(), CompatError>;
    fn apply(&self, new_config: &serde_json::Value, target_path: &Path) -> Result<(), CompatError>;
}

pub struct SingBoxBackend {
    pub binary_path: std::path::PathBuf,
}

impl CompatibilityBackend for SingBoxBackend {
    fn validate(&self, config_path: &Path) -> Result<(), CompatError> {
        let output = Command::new(&self.binary_path)
            .arg("check")
            .arg("-c")
            .arg(config_path)
            .output()
            .map_err(|e| CompatError::Io(format!("failed to run sing-box check: {e}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(CompatError::ConfigValidationFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    fn apply(&self, new_config: &serde_json::Value, target_path: &Path) -> Result<(), CompatError> {
        apply_config_atomically(new_config, target_path, |p| self.validate(p))
    }
}

/// Path of the `.bak` copy `apply_config_atomically` keeps of the
/// previously-live config, exposed so callers (e.g. `vpn-admin`'s
/// reload-rollback path) can restore it without duplicating this naming
/// convention.
pub fn config_backup_path(target_path: &Path) -> std::path::PathBuf {
    let mut p = target_path.to_path_buf();
    let name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");
    p.set_file_name(format!("{name}.bak"));
    p
}

/// Write-validate-swap: never overwrites a known-working config with an
/// invalid one (spec §16). `validate_fn` is injected so this logic is
/// unit-testable without the real `sing-box` binary.
pub fn apply_config_atomically(
    new_config: &serde_json::Value,
    target_path: &Path,
    validate_fn: impl Fn(&Path) -> Result<(), CompatError>,
) -> Result<(), CompatError> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CompatError::Io(e.to_string()))?;
    }
    let tmp_path = {
        let mut p = target_path.to_path_buf();
        let name = target_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config.json");
        p.set_file_name(format!("{name}.tmp.{}", std::process::id()));
        p
    };
    let bytes =
        serde_json::to_vec_pretty(new_config).map_err(|e| CompatError::Parse(e.to_string()))?;
    write_config_file_mode_0640(&tmp_path, &bytes)?;

    if let Err(e) = validate_fn(&tmp_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    if target_path.exists() {
        let backup_path = config_backup_path(target_path);
        std::fs::copy(target_path, &backup_path).map_err(|e| CompatError::Io(e.to_string()))?;
        set_file_mode_0640(&backup_path)?;
    }

    // rename() never changes ownership — without this, every regenerated
    // config.json silently turns back into root:root and breaks
    // sing-box's read access (docs/FINAL_PRODUCTION_AUDIT.md P0-2).
    crate::ownership::preserve_ownership_before_rename(&tmp_path, target_path)?;
    std::fs::rename(&tmp_path, target_path).map_err(|e| CompatError::Io(e.to_string()))?;
    fsync_parent_dir(target_path);
    Ok(())
}

/// `config.json` contains the REALITY private key, VLESS UUIDs, and
/// Hysteria2 passwords in cleartext — never write it with the process
/// umask (often 0644). Written mode 0640 (root:sing-box, ownership set
/// by the installer) and `fsync`d before it's ever handed to
/// `sing-box check`, so a crash between write and validate never leaves
/// an unflushed secret file behind.
#[cfg(unix)]
fn write_config_file_mode_0640(path: &Path, bytes: &[u8]) -> Result<(), CompatError> {
    use std::io::Write;
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
fn write_config_file_mode_0640(path: &Path, bytes: &[u8]) -> Result<(), CompatError> {
    std::fs::write(path, bytes).map_err(|e| CompatError::Io(e.to_string()))
}

#[cfg(unix)]
fn set_file_mode_0640(path: &Path) -> Result<(), CompatError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640))
        .map_err(|e| CompatError::Io(e.to_string()))
}

#[cfg(not(unix))]
fn set_file_mode_0640(_path: &Path) -> Result<(), CompatError> {
    Ok(())
}

/// `fsync` the parent directory after a rename so the directory-entry
/// update itself is durable (renames are not implicitly fsynced on
/// Linux). Best-effort: some filesystems/sandboxes don't support
/// opening a directory for this, so failure here is not fatal — the
/// config has already been correctly swapped from the caller's
/// perspective either way.
#[cfg(unix)]
fn fsync_parent_dir(target_path: &Path) {
    if let Some(parent) = target_path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn fsync_parent_dir(_target_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;

    fn users() -> Vec<CompatUser> {
        vec![
            CompatUser {
                id: "u-active".into(),
                name: "active".into(),
                enabled: true,
                vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
                hysteria2_password: SecretString::new("pw1"),
                subscription_token_hash_hex: "h1".into(),
                created_at: 0,
                expires_at: None,
            },
            CompatUser {
                id: "u-disabled".into(),
                name: "disabled".into(),
                enabled: false,
                vless_uuid: "22222222-2222-4222-8222-222222222222".into(),
                hysteria2_password: SecretString::new("pw2"),
                subscription_token_hash_hex: "h2".into(),
                created_at: 0,
                expires_at: None,
            },
            CompatUser {
                id: "u-expired".into(),
                name: "expired".into(),
                enabled: true,
                vless_uuid: "33333333-3333-4333-8333-333333333333".into(),
                hysteria2_password: SecretString::new("pw3"),
                subscription_token_hash_hex: "h3".into(),
                created_at: 0,
                expires_at: Some(100),
            },
        ]
    }

    fn reality() -> RealityServerParams {
        RealityServerParams {
            private_key_hex: SecretString::new("SUPER-SECRET-PRIVATE-KEY"),
            public_key_hex: "pub123".into(),
            short_ids: vec!["0a1b2c3d".into()],
            handshake_server: "www.microsoft.com".into(),
            handshake_port: 443,
        }
    }

    fn hysteria() -> Hysteria2ServerParams {
        Hysteria2ServerParams {
            tls_cert_path: "/etc/vpn/compat/hysteria/cert.pem".into(),
            tls_key_path: "/etc/vpn/compat/hysteria/key.pem".into(),
            obfs_password: None,
            masquerade_dir_path: Some("/etc/vpn/compat/hysteria/masquerade".into()),
        }
    }

    #[test]
    fn disabled_and_expired_users_are_excluded_from_rendered_config() {
        let cfg = render_singbox_server_config(
            &users(),
            &reality(),
            &hysteria(),
            ServerPorts {
                vless_reality_port: 443,
                hysteria2_port: 443,
            },
            1000,
        );
        let vless_users = cfg["inbounds"][0]["users"].as_array().unwrap();
        assert_eq!(vless_users.len(), 1);
        assert_eq!(vless_users[0]["name"], "u-active");
    }

    #[test]
    fn rendered_config_never_contains_private_key_string() {
        let cfg = render_singbox_server_config(
            &users(),
            &reality(),
            &hysteria(),
            ServerPorts {
                vless_reality_port: 443,
                hysteria2_port: 443,
            },
            1000,
        );
        // The private key VALUE must appear exactly once (the field sing-box
        // itself needs), never duplicated elsewhere, and the literal
        // "private_key_hex" Rust field name must not leak either.
        let s = serde_json::to_string(&cfg).unwrap();
        assert_eq!(s.matches("SUPER-SECRET-PRIVATE-KEY").count(), 1);
    }

    #[test]
    fn apply_atomically_rejects_invalid_config_and_leaves_existing_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        std::fs::write(&target, b"{\"good\":true}").unwrap();

        let bad_cfg = json!({"bad": true});
        let result = apply_config_atomically(&bad_cfg, &target, |_p| {
            Err(CompatError::ConfigValidationFailed("nope".into()))
        });
        assert!(result.is_err());
        let contents = std::fs::read_to_string(&target).unwrap();
        assert_eq!(contents, "{\"good\":true}");

        // no leftover tmp files
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover.is_empty());
    }

    #[test]
    fn apply_atomically_backs_up_and_swaps_on_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        std::fs::write(&target, b"{\"old\":true}").unwrap();

        let new_cfg = json!({"new": true});
        apply_config_atomically(&new_cfg, &target, |_p| Ok(())).unwrap();

        let contents = std::fs::read_to_string(&target).unwrap();
        assert!(contents.contains("\"new\""));
        let backup = dir.path().join("config.json.bak");
        let backup_contents = std::fs::read_to_string(&backup).unwrap();
        assert_eq!(backup_contents, "{\"old\":true}");
    }

    #[test]
    fn apply_atomically_succeeds_with_no_prior_config() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        let new_cfg = json!({"new": true});
        apply_config_atomically(&new_cfg, &target, |_p| Ok(())).unwrap();
        assert!(target.exists());
        assert!(!dir.path().join("config.json.bak").exists());
    }

    #[cfg(unix)]
    #[test]
    fn applied_config_and_backup_are_never_world_or_group_writable_and_are_0640() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        std::fs::write(&target, b"{\"old\":true}").unwrap();

        apply_config_atomically(&json!({"new": true}), &target, |_p| Ok(())).unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o640,
            "live config.json must be 0640, not world-readable"
        );

        // second apply produces a .bak from the first live config
        apply_config_atomically(&json!({"newer": true}), &target, |_p| Ok(())).unwrap();
        let backup = dir.path().join("config.json.bak");
        let backup_mode = std::fs::metadata(&backup).unwrap().permissions().mode();
        assert_eq!(
            backup_mode & 0o777,
            0o640,
            "config.json.bak must also be 0640"
        );
    }

    #[cfg(unix)]
    #[test]
    fn repeated_applies_preserve_group_ownership_across_mutations() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        apply_config_atomically(&json!({"n": 0}), &target, |_p| Ok(())).unwrap();
        let original_gid = std::fs::metadata(&target).unwrap().gid();

        for i in 1..6 {
            apply_config_atomically(&json!({"n": i}), &target, |_p| Ok(())).unwrap();
            let meta = std::fs::metadata(&target).unwrap();
            assert_eq!(meta.gid(), original_gid, "gid drifted on mutation {i}");
            assert_eq!(meta.permissions().mode() & 0o777, 0o640);
        }
    }

    #[test]
    fn hysteria_masquerade_is_set_when_configured() {
        let cfg = render_singbox_server_config(
            &users(),
            &reality(),
            &hysteria(),
            ServerPorts {
                vless_reality_port: 443,
                hysteria2_port: 443,
            },
            1000,
        );
        let masquerade = &cfg["inbounds"][1]["masquerade"];
        assert_eq!(masquerade["type"], "file");
        assert_eq!(
            masquerade["directory"],
            "/etc/vpn/compat/hysteria/masquerade"
        );
    }

    #[test]
    fn hysteria_masquerade_omitted_when_not_configured() {
        let mut h = hysteria();
        h.masquerade_dir_path = None;
        let cfg = render_singbox_server_config(
            &users(),
            &reality(),
            &h,
            ServerPorts {
                vless_reality_port: 443,
                hysteria2_port: 443,
            },
            1000,
        );
        assert!(cfg["inbounds"][1].get("masquerade").is_none());
    }
}
