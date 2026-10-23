//! Durable reconciliation for control-plane compatibility authorizations.

use crate::config::AgentConfig;
use crate::dispatch::{parse_json_output, run_vpn_admin, vpn_admin_command};
use crate::worker_client::{AuthorizationsFetch, WorkerClient};
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
    // Raw values: each item is decoded on its own. A schema violation still refuses the whole
    // response; a value-level problem quarantines only that credential (I-102).
    authorizations: Vec<Value>,
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
    /// credential_ids excluded because they were individually invalid. Fail closed: a
    /// quarantined credential is simply absent from `desired`, so a copy we already hold is
    /// removed, never kept. Labels are identifiers only, never secrets.
    quarantined: Vec<String>,
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

/// Identifier safe to log for an item we could not accept. Never includes any other field.
fn quarantine_label(credential_id: &str) -> String {
    if credential_id.starts_with("cred_")
        && credential_id.len() <= 96
        && credential_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        credential_id.to_owned()
    } else {
        "<unidentifiable>".to_owned()
    }
}

/// Value-level checks for one already-decoded item. A failure here quarantines only this item.
fn finish_v2_item(item: V2WireAuthorization) -> Result<Authorization> {
    let authorization = convert_authorization(
        item.principal_id,
        item.credential_id,
        item.class,
        item.valid_from,
        item.valid_until,
        item.revoked,
        item.protocol,
    )?;
    authorization.validate_item().map_err(anyhow::Error::msg)?;
    if authorization.class != CredentialClass::Compatibility {
        bail!("external authorization item is not a compatibility credential");
    }
    Ok(authorization)
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
        let mut authorizations = Vec::with_capacity(wire.authorizations.len());
        let mut quarantined = Vec::new();
        for raw in wire.authorizations {
            // Schema violations (unknown or missing fields, wrong types, unknown class) mean
            // the response broke the contract, so the whole response is refused as before.
            let item: V2WireAuthorization = serde_json::from_value(raw)
                .context("invalid v2 authorization item (contents not shown)")?;
            let label = quarantine_label(&item.credential_id);
            match finish_v2_item(item) {
                Ok(authorization) => authorizations.push(authorization),
                Err(_) => quarantined.push(label),
            }
        }
        // Set-level violations (duplicate credential ids, per-principal bounds) are ambiguous
        // about *which* credential is wrong, so they still reject the whole snapshot.
        return Ok(ParsedSnapshot {
            desired: validate_external_set(AuthorizationSet { authorizations })?,
            snapshot_revision: Some(wire.snapshot_revision),
            quarantined,
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
        quarantined: Vec::new(),
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

/// True when `next` is `current` made strictly more restrictive: every credential in `next`
/// already exists in `current` with identical fields except that some have newly become
/// `revoked`, and every credential missing from `next` is one we deliberately quarantined
/// (removal can only revoke access). Un-revoking, secret/validity changes and added
/// credentials are never a tightening.
fn only_tightens(
    current: &AuthorizationSet,
    next: &AuthorizationSet,
    quarantined: &[String],
) -> bool {
    let same = |a: &Authorization, b: &Authorization| {
        a.principal_id == b.principal_id && a.credential_id == b.credential_id
    };
    let dropped_only_if_quarantined = current.authorizations.iter().all(|held| {
        next.authorizations
            .iter()
            .any(|incoming| same(held, incoming))
            || quarantined.iter().any(|label| label == &held.credential_id)
    });
    dropped_only_if_quarantined
        && next.authorizations.iter().all(|incoming| {
            let Some(held) = current
                .authorizations
                .iter()
                .find(|held| same(held, incoming))
            else {
                return false;
            };
            if held.revoked && !incoming.revoked {
                return false;
            }
            let mut normalized = incoming.clone();
            normalized.revoked = held.revoked;
            matches!(
                (serde_json::to_vec(&normalized), serde_json::to_vec(held)),
                (Ok(a), Ok(b)) if a == b
            )
        })
}

fn adopt_snapshot(state: &mut PersistedExternalState, next: ParsedSnapshot) -> Result<bool> {
    let desired_changed = serde_json::to_vec(&next.desired)? != serde_json::to_vec(&state.desired)?;
    match (state.snapshot_revision, next.snapshot_revision) {
        (Some(current), Some(incoming)) if incoming < current => {
            bail!("stale external authorization snapshot revision")
        }
        (Some(current), Some(incoming)) if incoming == current && desired_changed => {
            // The control plane derives `revoked` from live entitlement (billing lapse,
            // suspension, device revocation), which can change without a new revision.
            // Refusing that left a revoked credential live indefinitely. A snapshot that
            // only sets `revoked` on credentials we already hold can never grant access,
            // so it is accepted; any other same-revision change is still equivocation.
            if !only_tightens(&state.desired, &next.desired, &next.quarantined) {
                bail!("external authorization revision content changed without a new revision")
            }
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
    /// ETag of the snapshot currently held, so an unchanged state costs a
    /// 304. In memory only: after a restart the first fetch is a full one.
    etag: Option<String>,
    /// Credentials excluded from the last accepted snapshot; while non-empty the revision is
    /// not acknowledged, because it was not applied in full.
    quarantined: Vec<String>,
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
            etag: None,
            quarantined: Vec::new(),
        })
    }

    /// `fetch_due` gates ONLY the network fetch. Local enforcement (expiry of
    /// the held desired state, applying it, ACKing it) runs on every call, so
    /// it never depends on the control plane or on how often it is polled.
    pub async fn tick(
        &mut self,
        cfg: &AgentConfig,
        client: &WorkerClient,
        fetch_due: bool,
    ) -> Result<()> {
        let mut fetch_error = None;
        if fetch_due {
            match client
                .fetch_authorizations_conditional(self.etag.as_deref())
                .await
            {
                Ok(AuthorizationsFetch::NotModified) => {}
                Ok(AuthorizationsFetch::Fresh { body, etag }) => {
                    let parsed = parse_snapshot(body);
                    let quarantined = parsed
                        .as_ref()
                        .map(|next| next.quarantined.clone())
                        .unwrap_or_default();
                    match parsed.and_then(|next| adopt_snapshot(&mut self.state, next)) {
                        Ok(changed) => {
                            self.quarantined = quarantined;
                            if changed {
                                // Desired state and its exact revision become durable before live mutation.
                                save(&self.path, &self.state)?;
                            }
                            self.etag = etag;
                        }
                        Err(error) => {
                            self.etag = None;
                            fetch_error = Some(error);
                        }
                    }
                }
                Err(error) => fetch_error = Some(error),
            }
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
                && self.quarantined.is_empty()
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
        if !self.quarantined.is_empty() {
            // Loud on every tick, and never acknowledged, so the control plane keeps seeing
            // this revision as not applied until the credential is corrected.
            bail!(
                "{} credential(s) quarantined and excluded (fail closed), revision not acknowledged: {}",
                self.quarantined.len(),
                self.quarantined.join(", ")
            );
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
        // Killed and reaped on timeout (see dispatch::run_vpn_admin), so an
        // apply the agent reported as failed cannot land afterwards.
        let mut command = vpn_admin_command(cfg);
        command
            .args(["external-authorizations", "--input"])
            .arg(&input);
        let output = run_vpn_admin(
            command,
            VPN_ADMIN_TIMEOUT,
            "vpn-admin external-authorizations",
        )
        .await?;
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
        // An over-long lifetime is still refused, but now only that credential is: it is
        // quarantined (never applied, never acknowledged) rather than failing the snapshot.
        let parsed = parse_snapshot(lifetime).unwrap();
        assert_eq!(parsed.quarantined, ["cred_external01"]);
        assert!(parsed.desired.authorizations.is_empty());

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

    fn held(revoked: bool) -> PersistedExternalState {
        let mut doc = fixture("v2-active");
        doc["snapshot_revision"] = 5.into();
        doc["authorizations"][0]["revoked"] = revoked.into();
        let parsed = parse_snapshot(doc).unwrap();
        PersistedExternalState {
            desired: parsed.desired,
            snapshot_revision: parsed.snapshot_revision,
            applied_snapshot_revision: Some(5),
            acknowledged_snapshot_revision: Some(5),
        }
    }

    fn incoming(mutate: impl FnOnce(&mut Value)) -> ParsedSnapshot {
        let mut doc = fixture("v2-active");
        doc["snapshot_revision"] = 5.into();
        mutate(&mut doc);
        parse_snapshot(doc).unwrap()
    }

    #[test]
    fn same_revision_revocation_from_entitlement_is_adopted_and_reapplied() {
        let mut state = held(false);
        let changed = adopt_snapshot(
            &mut state,
            incoming(|d| d["authorizations"][0]["revoked"] = true.into()),
        )
        .expect("a revocation at an unchanged revision must not be refused");
        assert!(changed);
        assert!(state.desired.authorizations[0].revoked);
        assert_eq!(state.snapshot_revision, Some(5));
        // Must be re-applied and re-acknowledged, never skipped as already applied.
        assert_eq!(state.applied_snapshot_revision, None);
        assert_eq!(state.acknowledged_snapshot_revision, None);
    }

    #[test]
    fn same_revision_changes_that_could_grant_access_are_still_equivocation() {
        // un-revoking
        let mut state = held(true);
        assert!(adopt_snapshot(&mut state, incoming(|_| {})).is_err());
        assert!(
            state.desired.authorizations[0].revoked,
            "state untouched on refusal"
        );
        // extending validity while also revoking is not a pure tightening
        let mut state = held(false);
        assert!(adopt_snapshot(
            &mut state,
            incoming(|d| {
                d["authorizations"][0]["revoked"] = true.into();
                d["authorizations"][0]["valid_until"] = "2026-10-02T06:00:00Z".into();
            })
        )
        .is_err());
        // swapping the secret
        let mut state = held(false);
        assert!(adopt_snapshot(
            &mut state,
            incoming(|d| {
                d["authorizations"][0]["protocol"]["vless_uuid"] =
                    "99999999-9999-4999-8999-999999999999".into();
            })
        )
        .is_err());
        // dropping or adding a credential
        let mut state = held(false);
        assert!(adopt_snapshot(
            &mut state,
            incoming(|d| d["authorizations"] = serde_json::json!([]))
        )
        .is_err());
        assert!(!state.desired.authorizations.is_empty());
    }

    /// A clone of v2-active's credential under a new id, edited by `edit`.
    fn item_like(credential_id: &str, edit: impl FnOnce(&mut Value)) -> Value {
        let mut item = fixture("v2-active")["authorizations"][0].clone();
        item["credential_id"] = credential_id.into();
        edit(&mut item);
        item
    }

    /// v2-active at `revision` plus `extras` appended to its authorizations.
    fn snapshot_with(revision: u64, extras: Vec<Value>) -> Value {
        let mut doc = fixture("v2-active");
        doc["snapshot_revision"] = revision.into();
        doc["authorizations"].as_array_mut().unwrap().extend(extras);
        doc
    }

    #[test]
    fn one_malformed_credential_is_quarantined_and_the_rest_are_still_adopted() {
        let doc = snapshot_with(
            7,
            vec![
                item_like("cred_badsecret01", |i| {
                    i["protocol"]["vless_uuid"] = "not-a-uuid".into()
                }),
                item_like("cred_baddate0001", |i| {
                    i["valid_until"] = "yesterday".into()
                }),
                // revoked with an inverted window: exactly what a real revocation can leave
                item_like("cred_revokedshort", |i| {
                    i["revoked"] = true.into();
                    i["valid_until"] = i["valid_from"].clone();
                }),
            ],
        );
        let parsed = parse_snapshot(doc).expect("a bad item must not fail the snapshot");
        let mut quarantined = parsed.quarantined.clone();
        quarantined.sort();
        assert_eq!(quarantined, ["cred_baddate0001", "cred_badsecret01"]);
        let ids: Vec<_> = parsed
            .desired
            .authorizations
            .iter()
            .map(|a| a.credential_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["cred_external01", "cred_revokedshort"],
            "valid + the revoked one are kept"
        );
        assert!(parsed.desired.authorizations[1].revoked);
        assert_eq!(parsed.snapshot_revision, Some(7));
    }

    #[test]
    fn a_held_credential_whose_new_version_is_malformed_is_removed_not_kept() {
        let mut state = held(false);
        let next = parse_snapshot(incoming_doc(|d| {
            d["authorizations"][0]["protocol"]["vless_uuid"] = "not-a-uuid".into();
        }))
        .unwrap();
        assert_eq!(next.quarantined, ["cred_external01"]);
        // same revision, credential removed because it was quarantined: a tightening
        assert!(adopt_snapshot(&mut state, next)
            .expect("removal of a quarantined credential must be adopted"));
        assert!(
            state.desired.authorizations.is_empty(),
            "fail closed: not left live"
        );
    }

    #[test]
    fn removal_is_only_adopted_at_an_unchanged_revision_when_the_credential_was_quarantined() {
        let mut state = held(false);
        let empty = parse_snapshot(incoming_doc(|d| {
            d["authorizations"] = serde_json::json!([])
        }))
        .unwrap();
        assert!(empty.quarantined.is_empty());
        assert!(
            adopt_snapshot(&mut state, empty).is_err(),
            "a silent drop is still equivocation"
        );
        assert_eq!(state.desired.authorizations.len(), 1);
    }

    #[test]
    fn a_revocation_is_applied_even_when_another_credential_in_the_snapshot_is_malformed() {
        let mut state = held(false);
        let doc = {
            let mut d = snapshot_with(
                5,
                vec![item_like("cred_badsecret01", |i| {
                    i["protocol"]["vless_uuid"] = "x".into()
                })],
            );
            d["authorizations"][0]["revoked"] = true.into();
            d
        };
        let next = parse_snapshot(doc).unwrap();
        assert_eq!(next.quarantined, ["cred_badsecret01"]);
        assert!(adopt_snapshot(&mut state, next).unwrap());
        assert!(
            state.desired.authorizations[0].revoked,
            "legitimate revocation was not blocked"
        );
    }

    #[test]
    fn a_schema_violation_in_any_item_still_refuses_the_whole_response() {
        // Unknown field, wrong type and unknown class are contract breaches, not bad values.
        let cases = vec![
            item_like("cred_schemabad01", |i| i["surprise"] = 1.into()),
            item_like("cred_schemabad02", |i| i["revoked"] = "yes".into()),
            item_like("cred_schemabad03", |i| i["class"] = "admin".into()),
        ];
        for item in cases {
            assert!(parse_snapshot(snapshot_with(5, vec![item])).is_err());
        }
    }

    #[test]
    fn set_level_ambiguity_still_rejects_the_whole_snapshot() {
        let dup = snapshot_with(5, vec![item_like("cred_external01", |_| {})]);
        assert!(
            parse_snapshot(dup).is_err(),
            "duplicate credential ids are ambiguous"
        );
    }

    fn incoming_doc(mutate: impl FnOnce(&mut Value)) -> Value {
        let mut doc = fixture("v2-active");
        doc["snapshot_revision"] = 5.into();
        mutate(&mut doc);
        doc
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
    async fn a_quarantined_credential_is_excluded_applied_around_and_never_acknowledged() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let doc = snapshot_with(
            3,
            vec![item_like("cred_badsecret01", |i| {
                i["protocol"]["vless_uuid"] = "not-a-uuid".into()
            })],
        );
        Mock::given(method("GET"))
            .and(path("/api/agent/authorizations"))
            .and(query_param("schema", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(doc))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/agent/authorizations"))
            .respond_with(ResponseTemplate::new(204))
            .expect(0)
            .mount(&server)
            .await;

        let applied = dir.path().join("applied.json");
        let binary = dir.path().join("vpn-admin");
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\ncp \"$5\" {:?}\nprintf '%s\\n' '{{\"live\":true}}'\n",
                applied
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

        let err = reconciler
            .tick(&cfg, &client)
            .await
            .expect_err("quarantine must be reported");
        assert!(err.to_string().contains("cred_badsecret01"), "{err}");
        assert!(
            !err.to_string().contains("not-a-uuid"),
            "no secret in the message"
        );
        let applied_set: AuthorizationSet =
            serde_json::from_slice(&std::fs::read(&applied).unwrap()).unwrap();
        assert_eq!(
            applied_set.authorizations.len(),
            1,
            "the valid credential was applied"
        );
        assert_eq!(
            applied_set.authorizations[0].credential_id,
            "cred_external01"
        );
        let persisted = load_state(&state_path).unwrap();
        assert_eq!(persisted.applied_snapshot_revision, Some(3));
        assert_eq!(
            persisted.acknowledged_snapshot_revision, None,
            "partially applied revision is not acknowledged"
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

        assert!(reconciler.tick(&cfg, &client, true).await.is_err());
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
        reconciler.tick(&cfg, &client, true).await.unwrap();
        let applied = load_state(&state_path).unwrap();
        assert_eq!(applied.applied_snapshot_revision, Some(0));
        assert_eq!(applied.acknowledged_snapshot_revision, Some(0));
        server.verify().await;
    }
}
