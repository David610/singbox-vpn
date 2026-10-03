//! Durable reconciliation for control-plane compatibility authorizations.

use crate::config::AgentConfig;
use crate::dispatch::{parse_json_output, vpn_admin_command};
use crate::worker_client::WorkerClient;
use anyhow::{anyhow, bail, Context, Result};
use compat_config::authorization::{Authorization, AuthorizationSet, CredentialClass};
use compat_config::secret::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const VPN_ADMIN_TIMEOUT: Duration = Duration::from_secs(60);
/// Hard memory/CPU boundary for a complete control-plane snapshot. This is far
/// above the supported small-node population, yet prevents an authenticated
/// endpoint fault from handing validation an unbounded vector.
const MAX_EXTERNAL_AUTHORIZATIONS: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWireSnapshot {
    authorizations: Vec<LegacyWireAuthorization>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V2WireSnapshot {
    schema_version: u8,
    snapshot_revision: u64,
    authorizations: Vec<V2WireAuthorization>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWireAuthorization {
    principal_id: String,
    credential_id: String,
    class: CredentialClass,
    logical_route_id: String,
    valid_from: String,
    valid_until: String,
    #[serde(default)]
    revoked: bool,
    protocol: WireProtocol,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V2WireAuthorization {
    principal_id: String,
    credential_id: String,
    class: CredentialClass,
    valid_from: String,
    valid_until: String,
    #[serde(default)]
    revoked: bool,
    protocol: WireProtocol,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProtocol {
    #[serde(default)]
    vless_uuid: Option<String>,
    #[serde(default)]
    hysteria2_password: Option<String>,
}

struct ParsedSnapshot {
    desired: AuthorizationSet,
    snapshot_revision: Option<u64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedExternalState {
    desired: AuthorizationSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_snapshot_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acknowledged_snapshot_revision: Option<u64>,
}

fn valid_route_id(value: &str) -> bool {
    value.starts_with("route_")
        && (7..=96).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn convert_authorization(
    principal_id: String,
    credential_id: String,
    class: CredentialClass,
    valid_from: String,
    valid_until: String,
    revoked: bool,
    protocol: WireProtocol,
) -> Result<Authorization> {
    Ok(Authorization {
        principal_id,
        credential_id,
        class,
        valid_from: OffsetDateTime::parse(&valid_from, &Rfc3339)
            .context("invalid authorization valid_from")?
            .unix_timestamp(),
        valid_until: OffsetDateTime::parse(&valid_until, &Rfc3339)
            .context("invalid authorization valid_until")?
            .unix_timestamp(),
        revoked,
        vless_uuid: protocol.vless_uuid.map(SecretString::new),
        hysteria2_password: protocol.hysteria2_password.map(SecretString::new),
    })
}

fn validate_external_set(desired: AuthorizationSet) -> Result<AuthorizationSet> {
    if desired.authorizations.len() > MAX_EXTERNAL_AUTHORIZATIONS {
        bail!(
            "external authorization response exceeds the {} credential limit",
            MAX_EXTERNAL_AUTHORIZATIONS
        );
    }
    desired.validate().map_err(anyhow::Error::msg)?;
    if desired
        .authorizations
        .iter()
        .any(|authorization| authorization.class != CredentialClass::Compatibility)
    {
        bail!("external authorization response contains a non-compatibility credential");
    }
    Ok(desired)
}

fn enforce_snapshot_size(size: usize) -> Result<()> {
    if size > MAX_EXTERNAL_AUTHORIZATIONS {
        bail!(
            "external authorization response exceeds the {} credential limit",
            MAX_EXTERNAL_AUTHORIZATIONS
        );
    }
    Ok(())
}

fn parse_snapshot(value: Value) -> Result<ParsedSnapshot> {
    // schema_version is the sole explicit discriminator. Its presence always selects the
    // versioned parser; malformed/unknown versions never fall back to the legacy contract.
    if value.get("schema_version").is_some() {
        let wire: V2WireSnapshot = serde_json::from_value(value)
            .context("invalid v2 external authorization response (contents not shown)")?;
        if wire.schema_version != 2 {
            bail!("unsupported external authorization schema_version");
        }
        enforce_snapshot_size(wire.authorizations.len())?;
        let authorizations = wire
            .authorizations
            .into_iter()
            .map(|item| {
                convert_authorization(
                    item.principal_id,
                    item.credential_id,
                    item.class,
                    item.valid_from,
                    item.valid_until,
                    item.revoked,
                    item.protocol,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(ParsedSnapshot {
            desired: validate_external_set(AuthorizationSet { authorizations })?,
            snapshot_revision: Some(wire.snapshot_revision),
        });
    }

    let wire: LegacyWireSnapshot = serde_json::from_value(value)
        .context("invalid legacy external authorization response (contents not shown)")?;
    enforce_snapshot_size(wire.authorizations.len())?;
    let mut authorizations = Vec::with_capacity(wire.authorizations.len());
    for item in wire.authorizations {
        if !valid_route_id(&item.logical_route_id) {
            bail!("external authorization contains a malformed logical_route_id");
        }
        authorizations.push(convert_authorization(
            item.principal_id,
            item.credential_id,
            item.class,
            item.valid_from,
            item.valid_until,
            item.revoked,
            item.protocol,
        )?);
    }
    Ok(ParsedSnapshot {
        desired: validate_external_set(AuthorizationSet { authorizations })?,
        snapshot_revision: None,
    })
}

fn save(path: &Path, state: &impl Serialize) -> Result<()> {
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
    tmp.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

fn load_state(path: &Path) -> Result<PersistedExternalState> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(error).with_context(|| format!("reading {path:?}")),
    };
    // Explicit one-time compatibility with the state format shipped in #126.
    let state = serde_json::from_slice::<PersistedExternalState>(&bytes)
        .or_else(|_| {
            serde_json::from_slice::<AuthorizationSet>(&bytes).map(|desired| {
                PersistedExternalState {
                    desired,
                    ..Default::default()
                }
            })
        })
        .with_context(|| format!("parsing external state {path:?} (contents not shown)"))?;
    state.desired.validate().map_err(anyhow::Error::msg)?;
    Ok(state)
}

fn adopt_snapshot(state: &mut PersistedExternalState, next: ParsedSnapshot) -> Result<bool> {
    let desired_changed = serde_json::to_vec(&next.desired)? != serde_json::to_vec(&state.desired)?;
    match (state.snapshot_revision, next.snapshot_revision) {
        (Some(current), Some(incoming)) if incoming < current => {
            bail!("stale external authorization snapshot revision")
        }
        (Some(current), Some(incoming)) if incoming == current && desired_changed => {
            bail!("external authorization revision content changed without a new revision")
        }
        (Some(_), None) => bail!("refusing external authorization schema downgrade"),
        _ => {}
    }
    if desired_changed || next.snapshot_revision != state.snapshot_revision {
        state.desired = next.desired;
        state.snapshot_revision = next.snapshot_revision;
        state.applied_snapshot_revision = None;
        state.acknowledged_snapshot_revision = None;
        return Ok(true);
    }
    Ok(false)
}

pub struct ExternalReconciler {
    path: PathBuf,
    state: PersistedExternalState,
    applied_active_fingerprint: Vec<u8>,
    applied_this_process: bool,
}

impl ExternalReconciler {
    pub fn open(cfg: &AgentConfig) -> Result<Self> {
        let path = PathBuf::from(&cfg.external_authorization_state_file);
        let state = load_state(&path)?;
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
            Ok(next) => match adopt_snapshot(&mut self.state, next) {
                Ok(true) => {
                    // Desired state and its exact revision become durable before live mutation.
                    save(&self.path, &self.state)?;
                }
                Ok(false) => {}
                Err(error) => fetch_error = Some(error),
            },
            Err(error) => fetch_error = Some(error),
        }

        let now = OffsetDateTime::now_utc().unix_timestamp();
        let active_fingerprint = effective_fingerprint(&self.state.desired, now)?;
        let revision_needs_apply = self.state.snapshot_revision.is_some()
            && self.state.applied_snapshot_revision != self.state.snapshot_revision;
        if !self.applied_this_process
            || active_fingerprint != self.applied_active_fingerprint
            || revision_needs_apply
        {
            self.apply(cfg).await?;
            self.applied_active_fingerprint = active_fingerprint;
            self.applied_this_process = true;
            self.state.applied_snapshot_revision = self.state.snapshot_revision;
            save(&self.path, &self.state)?;
        }

        if let Some(revision) = self.state.snapshot_revision {
            if self.state.applied_snapshot_revision == Some(revision)
                && self.state.acknowledged_snapshot_revision != Some(revision)
            {
                // Capture and ACK the revision that was applied, never a subsequently fetched
                // or server-side "latest" revision. ACK failure does not roll back live VPN state.
                client.ack_external_authorizations(revision).await?;
                self.state.acknowledged_snapshot_revision = Some(revision);
                save(&self.path, &self.state)?;
            }
        }
        if let Some(error) = fetch_error {
            return Err(error);
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
        save(&input, &self.state.desired)?;
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
            authorizations = self.state.desired.authorizations.len(),
            "external authorizations applied live"
        );
        Ok(())
    }
}

fn effective_fingerprint(state: &AuthorizationSet, now: i64) -> Result<Vec<u8>> {
    serde_json::to_vec(&state.active_at(now)).context("fingerprinting active authorizations")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fixture(name: &str) -> Value {
        let text = match name {
            "legacy-active" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/legacy-active.json"),
            "v2-active" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-active.json"),
            "v2-a-b" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-a-b.json"),
            "v2-revoked" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-revoked.json"),
            "v2-empty" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-empty.json"),
            "identity-leak" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-identity-leak-invalid.json"),
            "unknown-protocol" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-unknown-protocol-invalid.json"),
            "stale" => include_str!("../../../fixtures/arcana-data-plane/external-authorizations/v2-stale-revision.json"),
            _ => panic!("unknown fixture"),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn canonical_legacy_and_v2_fixtures_parse_exactly() {
        let legacy = parse_snapshot(fixture("legacy-active")).unwrap();
        assert_eq!(legacy.snapshot_revision, None);
        assert_eq!(legacy.desired.authorizations.len(), 1);

        let active = parse_snapshot(fixture("v2-active")).unwrap();
        assert_eq!(active.snapshot_revision, Some(10));
        assert_eq!(active.desired.authorizations.len(), 1);
        assert_eq!(
            parse_snapshot(fixture("v2-a-b"))
                .unwrap()
                .desired
                .authorizations
                .len(),
            2
        );
        assert!(
            parse_snapshot(fixture("v2-revoked"))
                .unwrap()
                .desired
                .authorizations[0]
                .revoked
        );
        let empty = parse_snapshot(fixture("v2-empty")).unwrap();
        assert_eq!(empty.snapshot_revision, Some(0));
        assert!(empty.desired.authorizations.is_empty());
    }

    #[test]
    fn privacy_schema_rejects_identity_and_unknown_protocol_fields() {
        assert!(parse_snapshot(fixture("identity-leak")).is_err());
        assert!(parse_snapshot(fixture("unknown-protocol")).is_err());
        let base = fixture("v2-active");
        for field in [
            "account_id",
            "customer_id",
            "user_id",
            "email",
            "stripe_id",
            "subscription_token",
            "device_name",
        ] {
            let mut leaked = base.clone();
            leaked["authorizations"][0][field] = "forbidden".into();
            assert!(parse_snapshot(leaked).is_err(), "accepted {field}");
        }
        let mut top = base;
        top["unexpected"] = true.into();
        assert!(parse_snapshot(top).is_err());
    }

    #[test]
    fn versions_are_explicit_and_never_fall_back() {
        let mut unknown = fixture("v2-active");
        unknown["schema_version"] = 3.into();
        assert!(parse_snapshot(unknown).is_err());
        let mut malformed_v2 = fixture("v2-active");
        malformed_v2
            .as_object_mut()
            .unwrap()
            .remove("snapshot_revision");
        assert!(parse_snapshot(malformed_v2).is_err());
        let mut v2_with_legacy_route = fixture("v2-active");
        v2_with_legacy_route["authorizations"][0]["logical_route_id"] = "route_de_fast".into();
        assert!(parse_snapshot(v2_with_legacy_route).is_err());
        let mut legacy_without_route = fixture("legacy-active");
        legacy_without_route["authorizations"][0]
            .as_object_mut()
            .unwrap()
            .remove("logical_route_id");
        assert!(parse_snapshot(legacy_without_route).is_err());
    }

    #[test]
    fn rejects_malformed_route_identity_class_dates_and_secrets() {
        let base = fixture("legacy-active");
        for (field, bad) in [
            (
                "logical_route_id",
                serde_json::json!("customer@example.com"),
            ),
            ("principal_id", serde_json::json!("bad")),
            ("credential_id", serde_json::json!("bad")),
            ("class", serde_json::json!("native")),
            ("valid_from", serde_json::json!("not-a-date")),
            ("valid_until", serde_json::json!("not-a-date")),
        ] {
            let mut invalid = base.clone();
            invalid["authorizations"][0][field] = bad;
            assert!(
                parse_snapshot(invalid).is_err(),
                "accepted malformed {field}"
            );
        }
        for (field, bad) in [
            ("vless_uuid", serde_json::json!("not-a-uuid")),
            ("hysteria2_password", serde_json::json!("short")),
        ] {
            let mut invalid = base.clone();
            invalid["authorizations"][0]["protocol"][field] = bad;
            assert!(
                parse_snapshot(invalid).is_err(),
                "accepted malformed {field}"
            );
        }
    }

    #[test]
    fn policy_rejects_lifetime_overlap_and_three_credentials() {
        let mut lifetime = fixture("v2-active");
        lifetime["authorizations"][0]["valid_until"] = "2026-12-01T00:00:00Z".into();
        assert!(parse_snapshot(lifetime).is_err());

        let mut three = fixture("v2-a-b");
        let mut third = three["authorizations"][1].clone();
        third["credential_id"] = "cred_external03".into();
        third["protocol"]["vless_uuid"] = "00000000-0000-4000-8000-000000000003".into();
        three["authorizations"].as_array_mut().unwrap().push(third);
        assert!(parse_snapshot(three).is_err());

        let mut overlap = fixture("v2-a-b");
        overlap["authorizations"][0]["valid_until"] = "2026-10-05T00:00:01Z".into();
        overlap["authorizations"][1]["valid_until"] = "2026-10-06T00:00:00Z".into();
        assert!(parse_snapshot(overlap).is_err());
    }

    #[test]
    fn large_bounded_snapshot_validates_and_one_over_limit_fails_closed() {
        let template = fixture("v2-active")["authorizations"][0].clone();
        let mut items = Vec::with_capacity(MAX_EXTERNAL_AUTHORIZATIONS);
        for i in 0..MAX_EXTERNAL_AUTHORIZATIONS {
            let mut item = template.clone();
            item["principal_id"] = format!("ext_stress{i:08x}").into();
            item["credential_id"] = format!("cred_stress{i:08x}").into();
            item["protocol"]["vless_uuid"] = format!("00000000-0000-4000-8000-{i:012x}").into();
            items.push(item);
        }
        let snapshot = serde_json::json!({
            "schema_version": 2, "snapshot_revision": 99, "authorizations": items
        });
        assert_eq!(
            parse_snapshot(snapshot.clone())
                .unwrap()
                .desired
                .authorizations
                .len(),
            MAX_EXTERNAL_AUTHORIZATIONS
        );
        let mut oversized = snapshot;
        oversized["authorizations"]
            .as_array_mut()
            .unwrap()
            .push(template);
        assert!(parse_snapshot(oversized).is_err());
    }

    #[test]
    fn oversized_snapshot_cannot_replace_previously_accepted_state() {
        let accepted = parse_snapshot(fixture("v2-active")).unwrap();
        let state = PersistedExternalState {
            desired: accepted.desired,
            snapshot_revision: accepted.snapshot_revision,
            applied_snapshot_revision: accepted.snapshot_revision,
            acknowledged_snapshot_revision: accepted.snapshot_revision,
        };
        let before = serde_json::to_vec(&state).unwrap();
        let item = fixture("v2-active")["authorizations"][0].clone();
        let oversized = serde_json::json!({
            "schema_version": 2,
            "snapshot_revision": 11,
            "authorizations": vec![item; MAX_EXTERNAL_AUTHORIZATIONS + 1]
        });
        assert!(parse_snapshot(oversized).is_err());
        assert_eq!(serde_json::to_vec(&state).unwrap(), before);
        assert_eq!(state.snapshot_revision, Some(10));

        // A parse failure never reaches adopt_snapshot; make that boundary
        // explicit so future refactors cannot turn an oversized response into
        // an authoritative empty snapshot.
        state.desired.validate().unwrap();
    }

    proptest! {
        #[test]
        fn snapshot_parser_never_panics_on_arbitrary_input(bytes in proptest::collection::vec(any::<u8>(), 0..8192)) {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let _ = parse_snapshot(value);
            }
        }

        #[test]
        fn revision_parser_rejects_non_integer_revision(revision in "[^0-9]{1,32}") {
            let mut value = fixture("v2-empty");
            value["snapshot_revision"] = revision.into();
            prop_assert!(parse_snapshot(value).is_err());
        }
    }

    #[test]
    fn persisted_revision_bookkeeping_survives_restart_and_old_format_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let parsed = parse_snapshot(fixture("stale")).unwrap();
        let state = PersistedExternalState {
            desired: parsed.desired,
            snapshot_revision: parsed.snapshot_revision,
            applied_snapshot_revision: Some(10),
            acknowledged_snapshot_revision: None,
        };
        save(&path, &state).unwrap();
        let restarted = load_state(&path).unwrap();
        assert_eq!(restarted.snapshot_revision, Some(10));
        assert_eq!(restarted.applied_snapshot_revision, Some(10));
        assert_eq!(restarted.acknowledged_snapshot_revision, None);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        let old = dir.path().join("old.json");
        save(&old, &state.desired).unwrap();
        let migrated = load_state(&old).unwrap();
        assert_eq!(migrated.snapshot_revision, None);
        assert_eq!(migrated.desired.authorizations.len(), 1);
    }

    #[test]
    fn revision_zero_advances_to_one_but_cannot_roll_back_or_equivocate() {
        let initial = parse_snapshot(fixture("v2-empty")).unwrap();
        let mut state = PersistedExternalState {
            desired: initial.desired,
            snapshot_revision: initial.snapshot_revision,
            applied_snapshot_revision: Some(0),
            acknowledged_snapshot_revision: Some(0),
        };
        assert!(state.desired.authorizations.is_empty());

        let mut revision_one = fixture("v2-active");
        revision_one["snapshot_revision"] = 1.into();
        assert!(adopt_snapshot(&mut state, parse_snapshot(revision_one.clone()).unwrap()).unwrap());
        assert_eq!(state.snapshot_revision, Some(1));
        assert_eq!(state.desired.authorizations.len(), 1);
        assert_eq!(state.applied_snapshot_revision, None);
        assert_eq!(state.acknowledged_snapshot_revision, None);

        assert!(adopt_snapshot(&mut state, parse_snapshot(fixture("v2-empty")).unwrap()).is_err());
        assert_eq!(state.snapshot_revision, Some(1));

        let mut equivocated = fixture("v2-empty");
        equivocated["snapshot_revision"] = 1.into();
        assert!(adopt_snapshot(&mut state, parse_snapshot(equivocated).unwrap()).is_err());
        assert!(adopt_snapshot(
            &mut state,
            parse_snapshot(fixture("legacy-active")).unwrap()
        )
        .is_err());
        assert_eq!(state.snapshot_revision, Some(1));
        assert_eq!(state.desired.authorizations.len(), 1);
    }

    #[test]
    fn effective_fingerprint_ignores_order_but_detects_secret_and_expiry() {
        let parsed = parse_snapshot(fixture("v2-a-b")).unwrap();
        let mut reversed = parsed.desired.clone();
        reversed.authorizations.reverse();
        let now = OffsetDateTime::parse("2026-10-01T12:00:00Z", &Rfc3339)
            .unwrap()
            .unix_timestamp();
        assert_eq!(
            effective_fingerprint(&parsed.desired, now).unwrap(),
            effective_fingerprint(&reversed, now).unwrap()
        );
        reversed.authorizations[0].vless_uuid =
            Some(SecretString::new("33333333-3333-4333-8333-333333333333"));
        assert_ne!(
            effective_fingerprint(&parsed.desired, now).unwrap(),
            effective_fingerprint(&reversed, now).unwrap()
        );
        assert_ne!(
            effective_fingerprint(&parsed.desired, now).unwrap(),
            effective_fingerprint(
                &parsed.desired,
                parsed.desired.authorizations[0].valid_until
            )
            .unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_apply_is_never_acked_and_retry_acks_exact_persisted_revision() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("v2-empty")))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .and(body_json(serde_json::json!({
                "schema_version": 2,
                "snapshot_revision": 0
            })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let binary = dir.path().join("vpn-admin");
        let fail_flag = dir.path().join("fail");
        std::fs::write(&fail_flag, b"").unwrap();
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\nif [ -e {:?} ]; then exit 1; fi\nprintf '%s\\n' '{{\"live\":true}}'\n",
                fail_flag
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config_path = dir.path().join("agent.toml");
        let state_path = dir.path().join("external.json");
        std::fs::write(
            &config_path,
            format!(
                "worker_url = {:?}\nnode_id = \"node_test\"\nagent_api_key = \"test-key\"\nvpn_admin_binary = {:?}\nvpn_admin_config = \"deployment.toml\"\nexternal_authorization_state_file = {:?}\n",
                server.uri(), binary, state_path
            ),
        )
        .unwrap();
        let cfg = AgentConfig::load(&config_path).unwrap();
        let client = WorkerClient::new(&cfg);
        let mut reconciler = ExternalReconciler::open(&cfg).unwrap();

        assert!(reconciler.tick(&cfg, &client).await.is_err());
        let after_failure = load_state(&state_path).unwrap();
        assert_eq!(after_failure.snapshot_revision, Some(0));
        assert_eq!(after_failure.applied_snapshot_revision, None);
        assert_eq!(after_failure.acknowledged_snapshot_revision, None);
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.method.as_str() == "POST")
                .count(),
            0
        );

        std::fs::remove_file(fail_flag).unwrap();
        reconciler.tick(&cfg, &client).await.unwrap();
        let applied = load_state(&state_path).unwrap();
        assert_eq!(applied.applied_snapshot_revision, Some(0));
        assert_eq!(applied.acknowledged_snapshot_revision, Some(0));
        server.verify().await;
    }
}
