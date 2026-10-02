//! Control-plane-owned compatibility authorization reconciliation.
//!
//! This writer owns only `cred_*` users whose opaque principal (`name`) is
//! `ext_*`. Native lease, probe, legacy, and operator-managed users are
//! deliberately carried through unchanged.

use anyhow::{bail, Result};
use compat_config::authorization::{Authorization, AuthorizationSet, CredentialClass};
use compat_config::credentials;
use compat_config::model::CompatUser;
use compat_config::secret::SecretString;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuthorizationInput {
    pub authorizations: Vec<Authorization>,
}

fn is_external(user: &CompatUser) -> bool {
    user.id.starts_with("cred_") && user.name.starts_with("ext_")
}

pub fn reconcile(
    users: &[CompatUser],
    input: &ExternalAuthorizationInput,
    now: i64,
) -> Result<(Vec<CompatUser>, bool)> {
    let set = AuthorizationSet {
        authorizations: input.authorizations.clone(),
    };
    set.validate().map_err(anyhow::Error::msg)?;
    if set
        .authorizations
        .iter()
        .any(|a| a.class != CredentialClass::Compatibility)
    {
        bail!("external sync accepts compatibility authorizations only");
    }

    let mut uuids = BTreeSet::new();
    for auth in &set.authorizations {
        let uuid = auth
            .vless_uuid
            .as_ref()
            .map(|v| v.expose().to_ascii_lowercase());
        if uuid
            .as_ref()
            .is_some_and(|uuid| !uuids.insert(uuid.clone()))
        {
            bail!("external authorizations must have distinct VLESS UUIDs");
        }
    }
    if users
        .iter()
        .filter(|u| !is_external(u))
        .any(|u| uuids.contains(&u.vless_uuid.to_ascii_lowercase()))
    {
        bail!("an external VLESS UUID collides with a non-external user");
    }

    let mut next: Vec<_> = users.iter().filter(|u| !is_external(u)).cloned().collect();
    let mut desired = set.authorizations;
    desired.sort_by(|a, b| a.credential_id.cmp(&b.credential_id));
    for auth in desired {
        let mut user = users
            .iter()
            .find(|u| u.id == auth.credential_id && is_external(u))
            .cloned()
            .unwrap_or_else(|| CompatUser {
                id: auth.credential_id.clone(),
                name: auth.principal_id.clone(),
                enabled: false,
                vless_uuid: String::new(),
                hysteria2_password: SecretString::new(String::new()),
                subscription_token_hash_hex: credentials::hash_token(
                    &credentials::generate_subscription_token(),
                ),
                created_at: now,
                expires_at: None,
                vision_off_experiment: false,
                google_egress_hairpin: false,
                peer_credentials: Default::default(),
                is_reserved_probe: false,
            });
        user.name = auth.principal_id;
        user.enabled = !auth.revoked;
        user.created_at = auth.valid_from;
        user.expires_at = Some(auth.valid_until);
        user.vless_uuid = auth
            .vless_uuid
            .map(|v| v.expose().to_ascii_lowercase())
            .unwrap_or_default();
        user.hysteria2_password = auth
            .hysteria2_password
            .unwrap_or_else(|| SecretString::new(String::new()));
        next.push(user);
    }
    let changed = serde_json::to_value(&next)? != serde_json::to_value(users)?;
    Ok((next, changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use compat_config::authorization::COMPAT_MIN_LIFETIME_SECS;

    fn external(id: &str, from: i64) -> Authorization {
        Authorization {
            principal_id: "ext_principal001".into(),
            credential_id: id.into(),
            class: CredentialClass::Compatibility,
            valid_from: from,
            valid_until: from + COMPAT_MIN_LIFETIME_SECS,
            revoked: false,
            vless_uuid: Some(SecretString::new(format!(
                "00000000-0000-4000-8000-{:012}",
                if id.ends_with('2') { 2 } else { 1 }
            ))),
            hysteria2_password: Some(SecretString::new(format!("password-{id}-long"))),
        }
    }

    fn user(id: &str, name: &str) -> CompatUser {
        CompatUser {
            id: id.into(),
            name: name.into(),
            enabled: true,
            vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
            hysteria2_password: SecretString::new("native-password-long"),
            subscription_token_hash_hex: "00".into(),
            created_at: 0,
            expires_at: Some(1800),
            vision_off_experiment: false,
            google_egress_hairpin: false,
            peer_credentials: Default::default(),
            is_reserved_probe: false,
        }
    }

    #[test]
    fn replaces_only_external_ownership_and_supports_ab_rotation() {
        let native = user("cred_native001", "native_principal001");
        let legacy = user("legacy", "operator");
        let probe = user("probe", "probe");
        let (one, _) = reconcile(
            &[native.clone(), legacy.clone(), probe.clone()],
            &ExternalAuthorizationInput {
                authorizations: vec![external("cred_external1", 0)],
            },
            1,
        )
        .unwrap();
        let (two, _) = reconcile(
            &one,
            &ExternalAuthorizationInput {
                authorizations: vec![external("cred_external1", 0), external("cred_external2", 1)],
            },
            2,
        )
        .unwrap();
        assert!(two.iter().any(|u| u.id == native.id));
        assert!(two.iter().any(|u| u.id == legacy.id));
        assert!(two.iter().any(|u| u.id == probe.id));
        assert_eq!(
            two.iter().filter(|u| u.name == "ext_principal001").count(),
            2
        );
    }

    #[test]
    fn omission_and_revocation_fail_closed_without_touching_native() {
        let native = user("cred_native001", "native_principal001");
        let mut revoked = external("cred_external1", 0);
        revoked.revoked = true;
        let (state, _) = reconcile(
            std::slice::from_ref(&native),
            &ExternalAuthorizationInput {
                authorizations: vec![revoked],
            },
            1,
        )
        .unwrap();
        assert!(
            !state
                .iter()
                .find(|u| u.id == "cred_external1")
                .unwrap()
                .enabled
        );
        let (removed, _) = reconcile(
            &state,
            &ExternalAuthorizationInput {
                authorizations: vec![],
            },
            1,
        )
        .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id, native.id);
    }
}
