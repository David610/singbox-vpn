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

    // `flow` is per-user and MUST match what the client sends: sing-box's
    // VLESS server compares them directly (sing-vmess `vless/service.go`
    // — `else if request.Flow != userFlow { return E.New("flow mismatch:
    // expected ", ..., ", but got ", ...) }`), so a Vision-off client
    // profile (`?compat=vision-off`, see `render.rs`) can only connect if
    // this user's server-side entry also carries an empty flow. Default
    // stays `xtls-rprx-vision` for every user — only a user explicitly
    // opted into the EXPERIMENTAL `vision_off_experiment` flag renders
    // differently, and only for as long as that flag is set.
    let vless_users: Vec<_> = active
        .iter()
        .map(|u| {
            json!({
                "name": u.id,
                "uuid": u.vless_uuid,
                "flow": if u.vision_off_experiment { "" } else { "xtls-rprx-vision" },
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
    // Fixed-rate (Brutal) congestion control, opt-in only (never
    // hardcode a guessed bandwidth — see model.rs doc comment on
    // `Hysteria2ServerParams::up_mbps`/`down_mbps`). Both fields absent
    // (the default) leaves sing-box's adaptive BBR-based congestion
    // control in place. `ignore_client_bandwidth: true` accompanies an
    // explicit rate so sing-box enforces the operator-measured value
    // rather than trusting a client-declared one, per upstream sing-box
    // guidance for servers whose admin already knows the real bandwidth.
    if let (Some(up), Some(down)) = (hysteria.up_mbps, hysteria.down_mbps) {
        hysteria_inbound["up_mbps"] = json!(up);
        hysteria_inbound["down_mbps"] = json!(down);
        hysteria_inbound["ignore_client_bandwidth"] = json!(true);
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
                vision_off_experiment: false,
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
                vision_off_experiment: false,
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
                vision_off_experiment: false,
            },
        ]
    }

    fn reality() -> RealityServerParams {
        RealityServerParams {
            private_key_hex: SecretString::new("SUPER-SECRET-PRIVATE-KEY"),
            public_key_hex: "pub123".into(),
            short_ids: vec!["0a1b2c3d".into()],
            handshake_server: "www.google.com".into(),
            handshake_port: 443,
        }
    }

    fn hysteria() -> Hysteria2ServerParams {
        Hysteria2ServerParams {
            tls_cert_path: "/etc/vpn/compat/hysteria/cert.pem".into(),
            tls_key_path: "/etc/vpn/compat/hysteria/key.pem".into(),
            obfs_password: None,
            masquerade_dir_path: Some("/etc/vpn/compat/hysteria/masquerade".into()),
            up_mbps: None,
            down_mbps: None,
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

    /// Default, unchanged behavior: every user's inbound entry keeps the
    /// production `xtls-rprx-vision` flow. This is the guarantee the
    /// EXPERIMENTAL per-user Vision-off toggle must never regress.
    #[test]
    fn every_user_keeps_the_vision_flow_by_default() {
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
        for u in cfg["inbounds"][0]["users"].as_array().unwrap() {
            assert_eq!(u["flow"], "xtls-rprx-vision");
        }
    }

    /// sing-box's VLESS server compares the client's requested flow
    /// against the configured per-user flow directly (sing-vmess
    /// `vless/service.go`: `else if request.Flow != userFlow { ... "flow
    /// mismatch" }`), so the EXPERIMENTAL `?compat=vision-off` client
    /// profile can only connect when that ONE user's server-side entry
    /// also carries an empty flow. Opting one user in must not touch any
    /// other user's flow, or anything else in the config.
    #[test]
    fn vision_off_experiment_empties_only_that_users_flow() {
        let mut users = users();
        users.push(CompatUser {
            id: "u-vision-off".into(),
            name: "vision-off".into(),
            enabled: true,
            vless_uuid: "44444444-4444-4444-8444-444444444444".into(),
            hysteria2_password: SecretString::new("pw4"),
            subscription_token_hash_hex: "h4".into(),
            created_at: 0,
            expires_at: None,
            vision_off_experiment: true,
        });
        let ports = ServerPorts {
            vless_reality_port: 443,
            hysteria2_port: 443,
        };
        let cfg = render_singbox_server_config(&users, &reality(), &hysteria(), ports, 1000);
        let vless_users = cfg["inbounds"][0]["users"].as_array().unwrap();
        assert_eq!(vless_users.len(), 2, "only active users are rendered");
        let normal = vless_users
            .iter()
            .find(|u| u["name"] == "u-active")
            .unwrap();
        let experimental = vless_users
            .iter()
            .find(|u| u["name"] == "u-vision-off")
            .unwrap();
        assert_eq!(
            normal["flow"], "xtls-rprx-vision",
            "an unrelated user must be completely unaffected"
        );
        assert_eq!(
            experimental["flow"], "",
            "the opted-in user's flow must be empty so a Vision-off client is accepted"
        );
        assert_eq!(
            experimental["uuid"], "44444444-4444-4444-8444-444444444444",
            "the experiment changes only the flow — not the UUID"
        );

        // Nothing outside the per-user flow may change: REALITY key
        // material, ports, listen addresses, handshake target, and the
        // Hysteria2 inbound must all be identical to the same render
        // with the flag off.
        let mut users_off = users.clone();
        users_off.last_mut().unwrap().vision_off_experiment = false;
        let cfg_off =
            render_singbox_server_config(&users_off, &reality(), &hysteria(), ports, 1000);
        assert_eq!(cfg["inbounds"][0]["tls"], cfg_off["inbounds"][0]["tls"]);
        assert_eq!(
            cfg["inbounds"][0]["listen_port"],
            cfg_off["inbounds"][0]["listen_port"]
        );
        assert_eq!(cfg["inbounds"][1], cfg_off["inbounds"][1]);
        assert_eq!(cfg["outbounds"], cfg_off["outbounds"]);
        assert_eq!(cfg["log"], cfg_off["log"]);
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

    #[test]
    fn hysteria_bandwidth_omitted_by_default() {
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
        let inbound = &cfg["inbounds"][1];
        assert!(inbound.get("up_mbps").is_none());
        assert!(inbound.get("down_mbps").is_none());
        assert!(inbound.get("ignore_client_bandwidth").is_none());
    }

    /// docs/CLIENT_PROTOCOL_BEHAVIOR.md's IPv4/IPv6 statement depends on
    /// both inbounds binding the dual-stack wildcard, not an IPv4-only
    /// address — lock that in so it can't silently regress. A dual-stack
    /// VPS then accepts either family; an IPv4-only VPS is limited by
    /// its own network, not by this listen address.
    #[test]
    fn both_inbounds_bind_the_dual_stack_wildcard_address() {
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
        assert_eq!(cfg["inbounds"][0]["listen"], "::");
        assert_eq!(cfg["inbounds"][1]["listen"], "::");
    }

    #[test]
    fn hysteria_bandwidth_set_together_forces_ignore_client_bandwidth() {
        let mut h = hysteria();
        h.up_mbps = Some(100);
        h.down_mbps = Some(80);
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
        let inbound = &cfg["inbounds"][1];
        assert_eq!(inbound["up_mbps"], 100);
        assert_eq!(inbound["down_mbps"], 80);
        assert_eq!(inbound["ignore_client_bandwidth"], true);
    }
}
