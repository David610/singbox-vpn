//! Durable reconciliation for control-plane compatibility authorizations.

use crate::config::AgentConfig;
use crate::dispatch::{parse_json_output, vpn_admin_command};
use crate::worker_client::WorkerClient;
use anyhow::{anyhow, Context, Result};
use compat_config::authorization::{Authorization, AuthorizationSet, CredentialClass};
use compat_config::secret::SecretString;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const VPN_ADMIN_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSnapshot {
    authorizations: Vec<WireAuthorization>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAuthorization {
    principal_id: String,
    credential_id: String,
    class: CredentialClass,
    valid_from: String,
    valid_until: String,
    #[serde(default)]
    revoked: bool,
    #[serde(default)]
    vless_uuid: Option<String>,
    #[serde(default)]
    hysteria2_password: Option<String>,
}

fn parse_snapshot(value: Value) -> Result<AuthorizationSet> {
    let wire: WireSnapshot = serde_json::from_value(value)
        .context("invalid external authorization response (contents not shown)")?;
    let mut authorizations = Vec::with_capacity(wire.authorizations.len());
    for item in wire.authorizations {
        authorizations.push(Authorization {
            principal_id: item.principal_id,
            credential_id: item.credential_id,
            class: item.class,
            valid_from: OffsetDateTime::parse(&item.valid_from, &Rfc3339)
                .context("invalid authorization valid_from")?
                .unix_timestamp(),
            valid_until: OffsetDateTime::parse(&item.valid_until, &Rfc3339)
                .context("invalid authorization valid_until")?
                .unix_timestamp(),
            revoked: item.revoked,
            vless_uuid: item.vless_uuid.map(SecretString::new),
            hysteria2_password: item.hysteria2_password.map(SecretString::new),
        });
    }
    let set = AuthorizationSet { authorizations };
    set.validate().map_err(anyhow::Error::msg)?;
    if set
        .authorizations
        .iter()
        .any(|a| a.class != CredentialClass::Compatibility)
    {
        return Err(anyhow!(
            "external authorization response contains a non-compatibility credential"
        ));
    }
    Ok(set)
}

fn save(path: &Path, state: &AuthorizationSet) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("state path has no parent"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    use std::io::Write;
    tmp.write_all(&serde_json::to_vec(state)?)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub struct ExternalReconciler {
    path: PathBuf,
    state: AuthorizationSet,
    applied_active_ids: Vec<String>,
    applied_this_process: bool,
}

impl ExternalReconciler {
    pub fn open(cfg: &AgentConfig) -> Result<Self> {
        let path = PathBuf::from(&cfg.external_authorization_state_file);
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing external state {path:?} (contents not shown)"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AuthorizationSet::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {path:?}")),
        };
        state.validate().map_err(anyhow::Error::msg)?;
        Ok(Self {
            path,
            state,
            applied_active_ids: Vec::new(),
            applied_this_process: false,
        })
    }

    pub async fn tick(&mut self, cfg: &AgentConfig, client: &WorkerClient) -> Result<()> {
        let mut fetch_error = None;
        match client.fetch_authorizations().await.and_then(parse_snapshot) {
            Ok(next) => {
                // Persist before apply: after a crash/restart the node retries
                // this exact desired state and never resurrects the old set.
                if serde_json::to_vec(&next)? != serde_json::to_vec(&self.state)? {
                    save(&self.path, &next)?;
                }
                self.state = next;
            }
            Err(err) => fetch_error = Some(err),
        }
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let active_ids: Vec<_> = self
            .state
            .active_at(now)
            .iter()
            .map(|a| a.credential_id.clone())
            .collect();
        if !self.applied_this_process || active_ids != self.applied_active_ids {
            self.apply(cfg).await?;
            self.applied_active_ids = active_ids;
            self.applied_this_process = true;
        }
        if let Some(err) = fetch_error {
            return Err(err);
        }
        Ok(())
    }

    async fn apply(&self, cfg: &AgentConfig) -> Result<()> {
        let dir = tempfile::Builder::new()
            .prefix("vpn-external-auth-")
            .tempdir()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let input = dir.path().join("authorizations.json");
        save(&input, &self.state)?;
        let output = tokio::time::timeout(
            VPN_ADMIN_TIMEOUT,
            vpn_admin_command(cfg)
                .args(["external-authorizations", "--input"])
                .arg(&input)
                .output(),
        )
        .await
        .context("vpn-admin external-authorizations timed out")??;
        let parsed = parse_json_output(&output, "external-authorizations")?;
        if parsed.get("live").and_then(Value::as_bool) != Some(true) {
            return Err(anyhow!("external authorization state was not proven live"));
        }
        tracing::info!(
            authorizations = self.state.authorizations.len(),
            "external authorizations applied live"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_contract_is_strict_and_privacy_minimal() {
        let good = serde_json::json!({"authorizations": [{
            "principal_id":"ext_principal001", "credential_id":"cred_external01",
            "class":"compatibility", "valid_from":"2026-10-01T00:00:00Z",
            "valid_until":"2026-10-02T00:00:00Z", "revoked":false,
            "vless_uuid":"11111111-1111-4111-8111-111111111111",
            "hysteria2_password":"long-password-value"
        }]});
        assert!(parse_snapshot(good.clone()).is_ok());
        let mut leaked = good;
        leaked["authorizations"][0]["email"] = serde_json::json!("person@example.com");
        assert!(parse_snapshot(leaked).is_err());
    }

    #[test]
    fn rejects_native_or_invalid_lifetime_from_control_plane() {
        let native = serde_json::json!({"authorizations": [{
            "principal_id":"native_principal1", "credential_id":"cred_external01",
            "class":"native", "valid_from":"2026-10-01T00:00:00Z",
            "valid_until":"2026-10-01T00:30:00Z", "vless_uuid":"11111111-1111-4111-8111-111111111111"
        }]});
        assert!(parse_snapshot(native).is_err());
    }
}
