//! Persistent AmneziaWG node state and `deployment.toml` enable/disable.
//!
//! File ownership (enforced by the installer, documented here):
//!
//! | file | owner:group mode | readers |
//! |---|---|---|
//! | `amneziawg/server.key` | `root:root 0600` | vpn-admin only |
//! | `amneziawg/server.pub` | `root:vpn-subscription 0640` | vpn-admin, vpn-subscription |
//! | `amneziawg/params.json` | `root:vpn-subscription 0640` | vpn-admin, vpn-subscription |
//! | `amneziawg/<iface>.conf` | `root:root 0600` | vpn-admin, `awg setconf` |

use crate::amneziawg::{
    derive_public_key, generate_private_key, AwgNodeConfig, AwgParams, AwgProfile,
};
use crate::deployment::{
    AmneziaWgSection, DeploymentConfig, DEPLOYMENT_SCHEMA_VERSION,
    DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG,
};
use crate::secret::SecretString;
use crate::CompatError;
use std::path::Path;

fn io(e: impl std::fmt::Display) -> CompatError {
    CompatError::Io(e.to_string())
}

/// Build the node configuration from `deployment.toml` plus state files.
/// `None` when the transport is not enabled. The private key is loaded
/// only when requested (vpn-admin); the subscription service passes
/// `false` and never touches `server.key`.
pub fn load_node_config(
    cfg: &DeploymentConfig,
    with_private_key: bool,
) -> Result<Option<AwgNodeConfig>, CompatError> {
    let Some(section) = &cfg.amneziawg else {
        return Ok(None);
    };
    let server_public_key = std::fs::read_to_string(cfg.amneziawg_server_public_key_file())
        .map_err(|e| {
            io(format!(
                "reading AmneziaWG server public key ({e}); run `vpn-admin transport enable amneziawg`"
            ))
        })?
        .trim()
        .to_string();
    let params: AwgParams = serde_json::from_slice(
        &std::fs::read(cfg.amneziawg_params_file())
            .map_err(|e| io(format!("reading AmneziaWG parameters ({e})")))?,
    )
    .map_err(|e| CompatError::Parse(format!("AmneziaWG params.json: {e}")))?;
    let server_private_key = if with_private_key {
        Some(SecretString::new(
            std::fs::read_to_string(cfg.amneziawg_server_private_key_file())
                .map_err(|e| io(format!("reading AmneziaWG server private key ({e})")))?
                .trim(),
        ))
    } else {
        None
    };
    let node = node_config_from_section(
        cfg,
        section,
        server_private_key,
        server_public_key,
        params,
    )?;
    Ok(Some(node))
}

fn node_config_from_section(
    cfg: &DeploymentConfig,
    section: &AmneziaWgSection,
    server_private_key: Option<SecretString>,
    server_public_key: String,
    params: AwgParams,
) -> Result<AwgNodeConfig, CompatError> {
    let node = AwgNodeConfig {
        interface: section.interface.clone(),
        public_host: cfg.public_host.clone(),
        listen_port: section.listen_port,
        subnet_v4: section.subnet_v4.parse().map_err(CompatError::Parse)?,
        subnet_v6: section
            .subnet_v6
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(CompatError::Parse)?,
        mtu: section.mtu,
        persistent_keepalive: section.persistent_keepalive,
        fallback_client_dns: section.fallback_client_dns.clone(),
        server_private_key,
        server_public_key,
        params,
    };
    use platform_core::transport::TransportProvider;
    crate::amneziawg::AmneziaWgProvider
        .validate(&node)
        .map_err(|e| CompatError::ConfigValidationFailed(e.to_string()))?;
    Ok(node)
}

#[derive(Debug, PartialEq, Eq)]
pub enum StateOutcome {
    Created,
    AlreadyPresent,
    Rotated,
}

fn write_mode(path: &Path, contents: &[u8], mode: u32) -> Result<(), CompatError> {
    crate::migrate::atomic_write(path, contents, mode)
}

/// Generate server keys and parameters if absent. Never overwrites an
/// existing, coherent keyset (that would silently break every client).
pub fn ensure_node_state(
    cfg: &DeploymentConfig,
    profile: AwgProfile,
) -> Result<StateOutcome, CompatError> {
    let dir = cfg.amneziawg_dir();
    std::fs::create_dir_all(&dir).map_err(io)?;
    let (key, public, params) = (
        cfg.amneziawg_server_private_key_file(),
        cfg.amneziawg_server_public_key_file(),
        cfg.amneziawg_params_file(),
    );
    let present = [key.exists(), public.exists(), params.exists()];
    if present.iter().all(|p| *p) {
        let private = std::fs::read_to_string(&key).map_err(io)?;
        let public_text = std::fs::read_to_string(&public).map_err(io)?;
        let derived = derive_public_key(private.trim()).map_err(CompatError::Parse)?;
        if derived != public_text.trim() {
            return Err(CompatError::Parse(
                "AmneziaWG server.key and server.pub do not match; refusing to continue (restore a \
                 backup or run `vpn-admin transport rotate amneziawg`)"
                    .into(),
            ));
        }
        return Ok(StateOutcome::AlreadyPresent);
    }
    if present.iter().any(|p| *p) {
        return Err(CompatError::Parse(format!(
            "AmneziaWG state in {dir:?} is incomplete (key/pub/params present: {present:?}); \
             refusing to guess. Restore a backup or rotate deliberately."
        )));
    }
    write_new_state(cfg, profile)?;
    Ok(StateOutcome::Created)
}

/// Replace keys and parameters. Client-breaking: every AWG profile must be
/// re-imported afterwards.
pub fn rotate_node_state(
    cfg: &DeploymentConfig,
    profile: AwgProfile,
) -> Result<StateOutcome, CompatError> {
    std::fs::create_dir_all(cfg.amneziawg_dir()).map_err(io)?;
    write_new_state(cfg, profile)?;
    Ok(StateOutcome::Rotated)
}

fn write_new_state(cfg: &DeploymentConfig, profile: AwgProfile) -> Result<(), CompatError> {
    let private = generate_private_key();
    let public = derive_public_key(private.expose()).map_err(CompatError::Parse)?;
    let params = AwgParams::generate(profile);
    params
        .validate()
        .map_err(|e| CompatError::ConfigValidationFailed(e.to_string()))?;
    let params_json =
        serde_json::to_vec_pretty(&params).map_err(|e| CompatError::Parse(e.to_string()))?;
    // Private key first: a crash leaves an incomplete set that
    // `ensure_node_state` refuses, never a public key without its private half
    // silently regenerated later.
    write_mode(
        &cfg.amneziawg_server_private_key_file(),
        format!("{}\n", private.expose()).as_bytes(),
        0o600,
    )?;
    write_mode(
        &cfg.amneziawg_server_public_key_file(),
        format!("{public}\n").as_bytes(),
        0o640,
    )?;
    write_mode(&cfg.amneziawg_params_file(), &params_json, 0o640)?;
    Ok(())
}

fn is_table_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// Text patch enabling `[amneziawg]`: stamps schema 3 and appends the
/// section, preserving every other line and comment. Refuses a file that
/// already has the section.
pub fn enable_in_toml(original: &str, section: &AmneziaWgSection) -> Result<String, CompatError> {
    if original
        .lines()
        .any(|l| l.trim() == "[amneziawg]")
    {
        return Err(CompatError::Parse(
            "[amneziawg] is already present in deployment.toml".into(),
        ));
    }
    let body = set_top_level_schema(original, DEPLOYMENT_SCHEMA_VERSION_AMNEZIAWG);
    let section_toml =
        toml::to_string(section).map_err(|e| CompatError::Parse(e.to_string()))?;
    let mut out = body.trim_end().to_string();
    out.push_str("\n\n[amneziawg]\n");
    out.push_str(&section_toml);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Text patch removing the `[amneziawg]` table and restamping schema 2.
pub fn disable_in_toml(original: &str) -> Result<String, CompatError> {
    let mut out = String::new();
    let mut skipping = false;
    let mut found = false;
    for line in original.lines() {
        if is_table_header(line) {
            skipping = line.trim() == "[amneziawg]";
            found |= skipping;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        return Err(CompatError::Parse(
            "[amneziawg] is not present in deployment.toml".into(),
        ));
    }
    Ok(set_top_level_schema(out.trim_end(), DEPLOYMENT_SCHEMA_VERSION) + "\n")
}

fn set_top_level_schema(original: &str, version: u32) -> String {
    let mut out = String::new();
    let mut in_top = true;
    let mut written = false;
    for line in original.lines() {
        if is_table_header(line) {
            if in_top && !written {
                out.push_str(&format!("schema_version = {version}\n"));
                written = true;
            }
            in_top = false;
        }
        let key = line
            .trim_start()
            .split_once('=')
            .map(|(k, _)| k.trim());
        if in_top && key == Some("schema_version") && !line.trim_start().starts_with('#') {
            if !written {
                out.push_str(&format!("schema_version = {version}\n"));
                written = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !written {
        out = format!("schema_version = {version}\n{out}");
    }
    out
}

/// Apply `patch` to `path` only if the result loads and every setting other
/// than the schema version and `[amneziawg]` is unchanged.
pub fn patch_deployment_file(
    path: &Path,
    patch: impl FnOnce(&str) -> Result<String, CompatError>,
) -> Result<DeploymentConfig, CompatError> {
    let original = std::fs::read_to_string(path).map_err(io)?;
    let before: DeploymentConfig =
        toml::from_str(&original).map_err(|e| CompatError::Parse(e.to_string()))?;
    let patched = patch(&original)?;
    let after: DeploymentConfig =
        toml::from_str(&patched).map_err(|e| CompatError::Parse(e.to_string()))?;
    after.validate()?;
    let project = |c: &DeploymentConfig| -> Result<serde_json::Value, CompatError> {
        let mut v = serde_json::to_value(c).map_err(|e| CompatError::Parse(e.to_string()))?;
        if let Some(o) = v.as_object_mut() {
            o.remove("schema_version");
            o.remove("amneziawg");
        }
        Ok(v)
    };
    if project(&before)? != project(&after)? {
        return Err(CompatError::Parse(
            "deployment.toml patch would change settings other than [amneziawg]; refusing (bug)"
                .into(),
        ));
    }
    crate::migrate::backup_before_mutate(path)?;
    crate::migrate::atomic_write(path, patched.as_bytes(), 0o644)?;
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"# operator comment kept
schema_version = 2
node_id = "fi1"
role = "exit"
public_host = "fi1.example.net"
subscription_host = "fi1.example.net"

[reality]
listen_port = 443
handshake_server = "www.example.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#;

    fn section() -> AmneziaWgSection {
        toml::from_str("listen_port = 51820").unwrap()
    }

    #[test]
    fn enable_then_disable_round_trips_and_keeps_comments() {
        let enabled = enable_in_toml(BASE, &section()).unwrap();
        assert!(enabled.contains("# operator comment kept"));
        let cfg: DeploymentConfig = toml::from_str(&enabled).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.schema_version, 3);
        assert_eq!(cfg.amneziawg.as_ref().unwrap().interface, "awg0");
        assert!(enable_in_toml(&enabled, &section()).is_err());

        let disabled = disable_in_toml(&enabled).unwrap();
        let cfg: DeploymentConfig = toml::from_str(&disabled).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.schema_version, 2);
        assert!(cfg.amneziawg.is_none());
        assert!(disabled.contains("# operator comment kept"));
        assert!(disable_in_toml(&disabled).is_err());
    }

    #[test]
    fn schema_and_section_must_agree() {
        let mut no_stamp = BASE.to_string();
        no_stamp.push_str("\n[amneziawg]\nlisten_port = 51820\n");
        let cfg: DeploymentConfig = toml::from_str(&no_stamp).unwrap();
        assert!(cfg.validate().is_err(), "[amneziawg] at schema 2 must fail");

        let orphan = BASE.replace("schema_version = 2", "schema_version = 3");
        let cfg: DeploymentConfig = toml::from_str(&orphan).unwrap();
        assert!(cfg.validate().is_err(), "schema 3 without [amneziawg] must fail");

        let future = BASE.replace("schema_version = 2", "schema_version = 4");
        let cfg: DeploymentConfig = toml::from_str(&future).unwrap();
        assert!(matches!(cfg.validate(), Err(CompatError::UnsupportedSchema { .. })));
    }

    #[test]
    fn section_rules() {
        let enabled = |extra: &str| {
            let text = enable_in_toml(BASE, &section()).unwrap().replace(
                "listen_port = 51820\n",
                &format!("listen_port = 51820\n{extra}"),
            );
            toml::from_str::<DeploymentConfig>(&text)
                .map_err(|e| CompatError::Parse(e.to_string()))
                .and_then(|c| c.validate())
        };
        assert!(enabled("").is_ok());
        assert!(enabled("bogus = 1\n").is_err(), "unknown keys are refused");
        let collide = enable_in_toml(BASE, &toml::from_str("listen_port = 443").unwrap()).unwrap();
        assert!(toml::from_str::<DeploymentConfig>(&collide).unwrap().validate().is_err());
        let relay = enable_in_toml(&BASE.replace("role = \"exit\"", "role = \"relay\""), &section());
        // A relay without access paths fails role validation anyway; the
        // AmneziaWG rule must also refuse it on its own.
        if let Ok(text) = relay {
            assert!(toml::from_str::<DeploymentConfig>(&text).unwrap().validate().is_err());
        }
    }

    #[test]
    fn migration_never_downgrades_an_amneziawg_file() {
        let enabled = enable_in_toml(BASE, &section()).unwrap();
        assert!(crate::deployment::migrate_deployment_toml_text(&enabled).is_none());
    }

    #[test]
    fn state_generation_is_idempotent_and_refuses_partial_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg: DeploymentConfig =
            toml::from_str(&enable_in_toml(BASE, &section()).unwrap()).unwrap();
        cfg.state_dir = dir.path().to_path_buf();
        assert_eq!(ensure_node_state(&cfg, AwgProfile::Awg3).unwrap(), StateOutcome::Created);
        let pubkey = std::fs::read_to_string(cfg.amneziawg_server_public_key_file()).unwrap();
        assert_eq!(ensure_node_state(&cfg, AwgProfile::Awg3).unwrap(), StateOutcome::AlreadyPresent);
        assert_eq!(std::fs::read_to_string(cfg.amneziawg_server_public_key_file()).unwrap(), pubkey);

        let public_only = load_node_config(&cfg, false).unwrap().unwrap();
        assert!(public_only.server_private_key.is_none());
        let full = load_node_config(&cfg, true).unwrap().unwrap();
        assert!(full.server_private_key.is_some());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(cfg.amneziawg_server_private_key_file()), 0o600);
            assert_eq!(mode(cfg.amneziawg_params_file()), 0o640);
        }

        std::fs::remove_file(cfg.amneziawg_params_file()).unwrap();
        assert!(ensure_node_state(&cfg, AwgProfile::Awg3).is_err());

        rotate_node_state(&cfg, AwgProfile::Awg3).unwrap();
        assert_ne!(std::fs::read_to_string(cfg.amneziawg_server_public_key_file()).unwrap(), pubkey);
    }

    #[test]
    fn patch_refuses_changes_outside_the_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deployment.toml");
        std::fs::write(&path, BASE).unwrap();
        let err = patch_deployment_file(&path, |t| {
            Ok(enable_in_toml(t, &section())?.replace("fi1.example.net", "evil.example.net"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), BASE);
        let cfg = patch_deployment_file(&path, |t| enable_in_toml(t, &section())).unwrap();
        assert_eq!(cfg.schema_version, 3);
    }
}
