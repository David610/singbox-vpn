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
    // The file fsync makes its contents durable; the directory fsync makes the atomic rename
    // durable as well. Without both, a power loss could restore the superseded snapshot.
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

pub struct ExternalReconciler {
    path: PathBuf,
    state: AuthorizationSet,
    // Serialized canonical active authorizations, kept private and never logged. Comparing
    // only credential ids would miss an in-place secret rotation under the same opaque id.
    applied_active_fingerprint: Vec<u8>,
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
            applied_active_fingerprint: Vec::new(),
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
        let active_fingerprint = effective_fingerprint(&self.state, now)?;
        if !self.applied_this_process || active_fingerprint != self.applied_active_fingerprint {
            self.apply(cfg).await?;
            self.applied_active_fingerprint = active_fingerprint;
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

fn effective_fingerprint(state: &AuthorizationSet, now: i64) -> Result<Vec<u8>> {
    // active_at is sorted by opaque principal/id, so irrelevant wire ordering does not cause
    // an apply. The bytes include secrets and bounds, so an in-place rotation or policy change
    // cannot be mistaken for an unchanged credential merely because its id stayed constant.
    serde_json::to_vec(&state.active_at(now)).context("fingerprinting active authorizations")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(id: &str, principal: &str, from: &str, until: &str) -> Value {
        serde_json::json!({
            "principal_id": principal,
            "credential_id": id,
            "class": "compatibility",
            "valid_from": from,
            "valid_until": until,
            "revoked": false,
            "vless_uuid": if id.ends_with('2') {
                "22222222-2222-4222-8222-222222222222"
            } else {
                "11111111-1111-4111-8111-111111111111"
            },
            "hysteria2_password": format!("password-{id}-long")
        })
    }

    #[test]
    fn wire_contract_is_strict_and_privacy_minimal() {
        let good = serde_json::json!({"authorizations": [wire(
            "cred_external01", "ext_principal001",
            "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"
        )]});
        assert!(parse_snapshot(good.clone()).is_ok());
        for identity_field in [
            "account_id",
            "customer_id",
            "user_id",
            "email",
            "stripe_id",
            "subscription_token",
            "device_name",
        ] {
            let mut leaked = good.clone();
            leaked["authorizations"][0][identity_field] = serde_json::json!("must-not-arrive");
            assert!(parse_snapshot(leaked).is_err(), "accepted {identity_field}");
        }
        let mut top_level = good;
        top_level["account_id"] = serde_json::json!("must-not-arrive");
        assert!(parse_snapshot(top_level).is_err());
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

    #[test]
    fn accepts_empty_revoked_and_ab_wire_shapes() {
        assert!(parse_snapshot(serde_json::json!({"authorizations": []})).is_ok());
        let mut revoked = wire(
            "cred_external01",
            "ext_principal001",
            "2026-10-01T00:00:00Z",
            "2026-10-02T00:00:00Z",
        );
        revoked["revoked"] = true.into();
        assert!(parse_snapshot(serde_json::json!({"authorizations": [revoked]})).is_ok());
        assert!(parse_snapshot(serde_json::json!({"authorizations": [
            wire("cred_external01", "ext_principal001", "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"),
            wire("cred_external02", "ext_principal001", "2026-10-01T12:00:00Z", "2026-10-02T12:00:00Z")
        ]})).is_ok());
    }

    #[test]
    fn rejects_malformed_contract_values_and_unsafe_rotation_sets() {
        let base = wire(
            "cred_external01",
            "ext_principal001",
            "2026-10-01T00:00:00Z",
            "2026-10-02T00:00:00Z",
        );
        for (field, bad) in [
            ("principal_id", serde_json::json!("account@example.com")),
            ("credential_id", serde_json::json!("bad")),
            ("class", serde_json::json!("premium")),
            ("valid_from", serde_json::json!("not-a-date")),
            ("valid_until", serde_json::json!("2026-30-99")),
            ("vless_uuid", serde_json::json!("not-a-uuid")),
            ("hysteria2_password", serde_json::json!("short")),
        ] {
            let mut malformed = base.clone();
            malformed[field] = bad;
            assert!(
                parse_snapshot(serde_json::json!({"authorizations": [malformed]})).is_err(),
                "accepted malformed {field}"
            );
        }

        let excessive_lifetime = wire(
            "cred_external01",
            "ext_principal001",
            "2026-01-01T00:00:00Z",
            "2026-03-01T00:00:00Z",
        );
        assert!(
            parse_snapshot(serde_json::json!({"authorizations": [excessive_lifetime]})).is_err()
        );

        let three: Vec<_> = (1..=3)
            .map(|n| {
                let mut item = wire(
                    &format!("cred_external0{n}"),
                    "ext_principal001",
                    "2026-10-01T00:00:00Z",
                    "2026-10-02T00:00:00Z",
                );
                item["vless_uuid"] = serde_json::json!(format!("00000000-0000-4000-8000-{n:012}"));
                item
            })
            .collect();
        assert!(parse_snapshot(serde_json::json!({"authorizations": three})).is_err());

        let excessive_overlap = serde_json::json!({"authorizations": [
            wire("cred_external01", "ext_principal001", "2026-10-01T00:00:00Z", "2026-10-05T00:00:01Z"),
            wire("cred_external02", "ext_principal001", "2026-10-02T00:00:00Z", "2026-10-06T00:00:00Z")
        ]});
        assert!(parse_snapshot(excessive_overlap).is_err());
    }

    #[test]
    fn effective_fingerprint_is_order_independent_but_detects_real_changes() {
        let value = serde_json::json!({"authorizations": [
            wire("cred_external01", "ext_principal001", "2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"),
            wire("cred_external02", "ext_principal001", "2026-10-01T12:00:00Z", "2026-10-02T12:00:00Z")
        ]});
        let set = parse_snapshot(value.clone()).unwrap();
        let mut reversed = value;
        reversed["authorizations"].as_array_mut().unwrap().reverse();
        let reversed = parse_snapshot(reversed).unwrap();
        let now = OffsetDateTime::parse("2026-10-01T13:00:00Z", &Rfc3339)
            .unwrap()
            .unix_timestamp();
        assert_eq!(
            effective_fingerprint(&set, now).unwrap(),
            effective_fingerprint(&reversed, now).unwrap()
        );

        let mut rotated = reversed;
        rotated.authorizations[0].vless_uuid =
            Some(SecretString::new("33333333-3333-4333-8333-333333333333"));
        assert_ne!(
            effective_fingerprint(&set, now).unwrap(),
            effective_fingerprint(&rotated, now).unwrap()
        );
        assert_ne!(
            effective_fingerprint(&set, now).unwrap(),
            effective_fingerprint(&set, set.authorizations[0].valid_until).unwrap()
        );
    }
}
