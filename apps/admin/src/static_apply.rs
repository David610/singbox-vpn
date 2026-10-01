//! Atomic apply of an allowlisted static-config revision delivered through
//! `APPLY_NODE_REVISION` / `vpn-admin apply-revision`. The wire format and
//! the field policy (allowlist, forbidden and deferred fields) live in
//! `compat_config::static_revision`; this module is the node-side state
//! machine:
//!
//! 1. parse + validate the document (schema, size, allowlist, values) —
//!    nothing is touched on any failure here;
//! 2. recover a previously interrupted static apply, if one left its
//!    backup behind;
//! 3. plan the candidate `deployment.toml` in memory (only allowlisted
//!    fields may differ — enforced structurally);
//! 4. snapshot dynamic state (`users.json` is read, never written);
//! 5. pre-flight: render the current and candidate sing-box documents and
//!    refuse unless route/outbound/DNS policy (C-16 egress deny-list,
//!    reserved-probe confinement, relay forwarding policy) is identical;
//! 6. stage: back up `deployment.toml` under a fixed name, then atomically
//!    write the candidate;
//! 7. apply through the shared `render_and_apply_singbox_config` pipeline
//!    (`sing-box check`, atomic swap, reload + verify, REALITY self-test,
//!    internal rollback of `config.json`), then restart `vpn-subscription`
//!    when the served endpoint set can have changed;
//! 8. commit the revision stamp and drop the backup.
//!
//! Any failure in 6–8 restores the original `deployment.toml`, re-renders
//! and reloads sing-box from it with the unchanged dynamic state, and
//! restarts `vpn-subscription` again if it had been restarted. The revision
//! stamp only advances on success.

use super::{
    commit_applied_revision_stamp, load_hysteria_params, load_reality_params,
    offline_mutation_allowed, render_and_apply_singbox_config,
};
use crate::service::CompatibilityServiceManager;
use anyhow::{bail, Context, Result};
use common::UnixSeconds;
use compat_config::deployment::DeploymentConfig;
use compat_config::server::render_server_config_for_deployment;
use compat_config::static_revision::{
    parse_static_revision, plan_static_revision, StaticRevision, StaticRevisionPlan,
};
use compat_config::{migrate, store, CompatUser};
use std::path::{Path, PathBuf};

/// Fixed name (never timestamped) so repeated applies can never accumulate
/// backup files; present only while a static apply is in flight.
pub(crate) fn static_revision_backup_path(config_path: &Path) -> PathBuf {
    let mut name = config_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "deployment.toml".into());
    name.push(".static-revision.bak");
    config_path.with_file_name(name)
}

#[cfg(unix)]
fn deployment_toml_mode(config_path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(config_path)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(0o644)
}

#[cfg(not(unix))]
fn deployment_toml_mode(_config_path: &Path) -> u32 {
    0o644
}

pub(crate) fn cmd_apply_static_revision(
    config_path: &Path,
    revision: u64,
    bytes: &[u8],
) -> Result<()> {
    let rev = parse_static_revision(bytes)
        .with_context(|| format!("revision {revision}: nothing was changed"))?;

    recover_interrupted_apply(config_path)?;

    let original_text =
        std::fs::read_to_string(config_path).with_context(|| format!("reading {config_path:?}"))?;
    let original_cfg =
        DeploymentConfig::load(config_path).with_context(|| format!("loading {config_path:?}"))?;

    let (candidate_text, candidate_cfg) = match plan_static_revision(&original_text, &rev)
        .with_context(|| format!("planning static revision {revision}: nothing was changed"))?
    {
        StaticRevisionPlan::Unchanged => {
            commit_applied_revision_stamp(&original_cfg, revision)
                .with_context(|| format!("recording static revision {revision}"))?;
            println!(
                "static revision {revision}: every requested value is already in effect; \
                 nothing reloaded, revision recorded."
            );
            return Ok(());
        }
        StaticRevisionPlan::Changed {
            candidate_text,
            candidate,
        } => (candidate_text, *candidate),
    };

    let users = store::load_users(&original_cfg.users_file())
        .context("loading this node's dynamic state (users.json) to carry across the apply")?;

    preflight_security_policy_unchanged(&original_cfg, &candidate_cfg, &users)
        .with_context(|| format!("static revision {revision}: nothing was changed"))?;
    if !offline_mutation_allowed() {
        compat_config::static_revision::validate_handshake_server_resolution(
            &candidate_cfg.reality.handshake_server,
            candidate_cfg.reality.handshake_port,
        )
        .context("candidate REALITY handshake server did not resolve exclusively to public addresses; nothing was changed")?;
    }

    let mode = deployment_toml_mode(config_path);
    let backup = static_revision_backup_path(config_path);
    migrate::atomic_write(&backup, original_text.as_bytes(), 0o600)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("backing up deployment.toml; nothing was changed")?;
    if let Err(e) = migrate::atomic_write(config_path, candidate_text.as_bytes(), mode) {
        let _ = std::fs::remove_file(&backup);
        bail!("writing candidate deployment.toml failed ({e}); nothing was changed");
    }

    let subscription = CompatibilityServiceManager::new("vpn-subscription");
    let subscription_touched = std::cell::Cell::new(false);
    let outcome = (|| -> Result<()> {
        render_and_apply_singbox_config(&candidate_cfg, &users, true)
            .context("rendering/validating/reloading the candidate sing-box config")?;
        if served_endpoint_inputs_changed(&original_cfg, &candidate_cfg, &rev) {
            if subscription.is_available() && subscription.is_unit_installed() {
                subscription_touched.set(true);
                subscription
                    .reload_and_verify()
                    .map_err(anyhow::Error::msg)
                    .context("restarting vpn-subscription with the candidate config")?;
            } else if !offline_mutation_allowed() {
                bail!(
                    "vpn-subscription.service is not controllable here, so the changed \
                     endpoint parameters cannot be proven live for clients"
                );
            } else {
                eprintln!(
                    "warning: vpn-subscription.service not available — it was NOT restarted \
                     and still serves the previous endpoint parameters."
                );
            }
        }
        Ok(())
    })();

    match outcome {
        Ok(()) => {
            commit_applied_revision_stamp(&candidate_cfg, revision).with_context(|| {
                format!(
                    "static revision {revision} was applied live, but recording it locally \
                     failed — the next heartbeat may under-report observed_revision"
                )
            })?;
            let _ = std::fs::remove_file(&backup);
            println!(
                "static revision {revision} applied ({} dynamic user record(s) carried over \
                 unchanged).",
                users.len()
            );
            Ok(())
        }
        Err(e) => {
            let toml_restored =
                migrate::atomic_write(config_path, original_text.as_bytes(), mode).is_ok();
            let singbox_restored = toml_restored
                && render_and_apply_singbox_config(&original_cfg, &users, true).is_ok();
            let subscription_restored =
                !subscription_touched.get() || subscription.reload_and_verify().is_ok();
            if toml_restored {
                let _ = std::fs::remove_file(&backup);
            }
            let status = if toml_restored && singbox_restored && subscription_restored {
                "The previous deployment.toml, sing-box config and services were restored; \
                 dynamic state (users.json) was never modified and the revision stamp was not \
                 advanced."
                    .to_string()
            } else {
                format!(
                    "ROLLBACK INCOMPLETE (deployment.toml restored={toml_restored}, sing-box \
                     restored={singbox_restored}, vpn-subscription restored=\
                     {subscription_restored}). The pre-apply deployment.toml is kept at \
                     {backup:?}; the next static revision (or manual recovery) restores it \
                     first. Check `systemctl status sing-box vpn-subscription`."
                )
            };
            Err(e).context(format!(
                "static revision {revision} FAILED and did NOT take effect. {status}"
            ))
        }
    }
}

fn served_endpoint_inputs_changed(
    original: &DeploymentConfig,
    candidate: &DeploymentConfig,
    rev: &StaticRevision,
) -> bool {
    rev.touches_served_endpoints()
        && (original.reality.handshake_server != candidate.reality.handshake_server
            || original.hysteria2.up_mbps != candidate.hysteria2.up_mbps
            || original.hysteria2.down_mbps != candidate.hysteria2.down_mbps)
}

/// Security-policy invariant for a routine static revision: the rendered
/// route (C-16 egress deny-list, reserved-probe confinement, relay
/// forwarding policy), outbounds and DNS sections must be exactly what the
/// node renders today. None of the allowlisted fields feed them, so any
/// difference means the allowlist or the renderer changed underneath this
/// path — refuse rather than risk a window of weaker isolation.
fn preflight_security_policy_unchanged(
    original: &DeploymentConfig,
    candidate: &DeploymentConfig,
    users: &[CompatUser],
) -> Result<()> {
    let now = UnixSeconds::now().0 as i64;
    let render = |cfg: &DeploymentConfig| -> Result<serde_json::Value> {
        let reality = load_reality_params(cfg).context(
            "a static revision requires this node's complete REALITY keyset (run `vpn-admin \
             init` first)",
        )?;
        let hysteria = load_hysteria_params(cfg);
        render_server_config_for_deployment(cfg, users, &reality, &hysteria, now)
            .context("rendering the sing-box config")
    };
    let current = render(original).context("pre-flight render of the CURRENT config")?;
    let proposed = render(candidate).context("pre-flight render of the CANDIDATE config")?;
    for section in ["route", "outbounds", "dns"] {
        if current[section] != proposed[section] {
            bail!(
                "pre-flight refused: the candidate would change the rendered {section:?} \
                 section (security policy: C-16 egress, probe confinement, relay forwarding). \
                 A routine static revision must never alter it."
            );
        }
    }
    Ok(())
}

/// A leftover backup means a previous static apply was interrupted between
/// staging and commit (e.g. the process was killed). Restore that
/// pre-apply `deployment.toml` and re-converge the live services to it
/// before doing anything else, so a new revision is always planned against
/// the last committed static state.
fn recover_interrupted_apply(config_path: &Path) -> Result<()> {
    let backup = static_revision_backup_path(config_path);
    let text = match std::fs::read_to_string(&backup) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => bail!("reading interrupted static-revision backup {backup:?}: {e}"),
    };
    let restored: DeploymentConfig = toml::from_str(&text)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .and_then(|cfg: DeploymentConfig| {
            cfg.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(cfg)
        })
        .with_context(|| {
            format!(
                "an interrupted static revision left {backup:?}, but it is not a valid \
                 deployment.toml — refusing to continue; recover manually"
            )
        })?;
    migrate::atomic_write(
        config_path,
        text.as_bytes(),
        deployment_toml_mode(config_path),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
    .context("restoring deployment.toml from an interrupted static revision")?;
    let users = store::load_users(&restored.users_file())?;
    render_and_apply_singbox_config(&restored, &users, true)
        .context("re-converging sing-box after restoring an interrupted static revision")?;
    let subscription = CompatibilityServiceManager::new("vpn-subscription");
    if subscription.is_available() && subscription.is_unit_installed() {
        subscription
            .reload_and_verify()
            .map_err(anyhow::Error::msg)
            .context(
                "restarting vpn-subscription after restoring an interrupted static revision",
            )?;
    }
    std::fs::remove_file(&backup)
        .with_context(|| format!("removing recovered backup {backup:?}"))?;
    eprintln!(
        "recovered an interrupted static revision: deployment.toml restored from {backup:?} \
         and services re-converged."
    );
    Ok(())
}
