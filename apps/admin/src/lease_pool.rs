//! ADR-0003 lease pool: pure reconciliation of the agent's desired
//! lease-slot set onto the compatibility user store.
//!
//! Lease-slot users are ordinary `CompatUser`s with a reserved id prefix
//! (`lease-NNNN`) and a pseudonymous name equal to that id — never an
//! email, account, subscription or Stripe identifier. Their subscription
//! token hash is of a random token that is immediately discarded, so no
//! subscription URL can ever reach them. Their `expires_at` is the slot's
//! node-enforced end (`valid_until`), so the renderer's `is_active` filter
//! drops them on any render after that instant.

use anyhow::{bail, Result};
use compat_config::authorization::{Authorization, AuthorizationSet, CredentialClass};
use compat_config::credentials;
use compat_config::model::CompatUser;
use compat_config::secret::SecretString;
use serde::Deserialize;
use std::collections::BTreeSet;

pub const LEASE_USER_PREFIX: &str = "lease-";
pub const MAX_SLOTS: usize = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseSlotInput {
    pub slot: u32,
    pub vless_uuid: String,
    pub hysteria2_password: String,
    pub expires_at: i64,
    /// Phase-1 authorization metadata. All four identity/time fields are required together;
    /// their optional representation keeps existing in-flight lease-pool documents readable.
    #[serde(default)]
    pub principal_id: Option<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
    #[serde(default)]
    pub class: Option<CredentialClass>,
    #[serde(default)]
    pub valid_from: Option<i64>,
    #[serde(default)]
    pub revoked: bool,
}

// No Debug: this carries secrets.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityAuthorizationInput {
    pub principal_id: String,
    pub credential_id: String,
    pub class: CredentialClass,
    pub logical_route_id: String,
    pub valid_from: i64,
    pub valid_until: i64,
    #[serde(default)]
    pub revoked: bool,
    pub vless_uuid: String,
    pub hysteria2_password: String,
}

// No Debug: this carries secrets.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeasePoolInput {
    pub slots: Vec<LeaseSlotInput>,
    /// None is deliberately different from Some(empty): an older agent
    /// does not own ext_* state and must preserve it, while a new agent that
    /// has fetched an authoritative empty snapshot must remove ext_* state.
    #[serde(default)]
    pub compatibility_authorizations: Option<Vec<CompatibilityAuthorizationInput>>,
}

pub fn lease_user_id(slot: u32) -> String {
    format!("{LEASE_USER_PREFIX}{slot:04}")
}

pub fn is_native_managed_user(user: &CompatUser) -> bool {
    user.id.starts_with(LEASE_USER_PREFIX)
        || (user.id.starts_with("cred_") && user.name.starts_with("native_"))
}

pub fn is_external_managed_user(user: &CompatUser) -> bool {
    user.id.starts_with("cred_") && user.name.starts_with("ext_")
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn valid_password(s: &str) -> bool {
    (16..=128).contains(&s.len()) && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

fn valid_route_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("route_") else {
        return false;
    };
    (3..=60).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Reconcile the one managed authorization projection.
///
/// Native lease slots are always owned by this input. External compatibility
/// state is owned only when `compatibility_authorizations` is present; an
/// older agent omitting that field cannot delete ext_* users. Reserved probe
/// and legacy normal users are never owned by this path.
pub fn reconcile(
    users: &[CompatUser],
    input: &LeasePoolInput,
    now: i64,
) -> Result<(Vec<CompatUser>, bool)> {
    if input.slots.len() > MAX_SLOTS {
        bail!("lease pool larger than {MAX_SLOTS} slots");
    }
    if input
        .compatibility_authorizations
        .as_ref()
        .is_some_and(|items| items.len() > MAX_SLOTS)
    {
        bail!("external authorization snapshot larger than {MAX_SLOTS} records");
    }

    let mut seen_slots = BTreeSet::new();
    let mut declared_uuids = BTreeSet::new();
    let mut authorizations = Vec::new();
    for s in &input.slots {
        if s.slot as usize >= MAX_SLOTS || !seen_slots.insert(s.slot) {
            bail!("lease slot {} is out of range or duplicated", s.slot);
        }
        if !valid_uuid(&s.vless_uuid) || !valid_password(&s.hysteria2_password) {
            bail!("lease slot {} has a malformed credential", s.slot);
        }
        if !declared_uuids.insert(s.vless_uuid.to_ascii_lowercase()) {
            bail!("managed authorization snapshot contains a duplicate VLESS uuid");
        }
        if s.expires_at <= 0 {
            bail!("lease slot {} has no expiry", s.slot);
        }
        let metadata_count = [
            s.principal_id.is_some(),
            s.credential_id.is_some(),
            s.class.is_some(),
            s.valid_from.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if metadata_count != 0 && metadata_count != 4 {
            bail!("lease slot {} must provide principal_id, credential_id, class, and valid_from together", s.slot);
        }
        if metadata_count == 4 {
            if s.class != Some(CredentialClass::Native) {
                bail!("lease slot {} must use the native credential class", s.slot);
            }
            authorizations.push(Authorization {
                principal_id: s.principal_id.clone().expect("count checked"),
                credential_id: s.credential_id.clone().expect("count checked"),
                class: s.class.expect("count checked"),
                valid_from: s.valid_from.expect("count checked"),
                valid_until: s.expires_at,
                revoked: s.revoked,
                vless_uuid: Some(SecretString::new(s.vless_uuid.clone())),
                hysteria2_password: Some(SecretString::new(s.hysteria2_password.clone())),
            });
        } else if s.revoked {
            bail!(
                "lease slot {} cannot set revoked without authorization metadata",
                s.slot
            );
        }
    }

    if let Some(external) = &input.compatibility_authorizations {
        for a in external {
            if a.class != CredentialClass::Compatibility {
                bail!("external authorization class must be compatibility");
            }
            if !valid_route_id(&a.logical_route_id) {
                bail!("external authorization has invalid logical route");
            }
            if !valid_uuid(&a.vless_uuid) || !valid_password(&a.hysteria2_password) {
                bail!("external authorization has a malformed credential");
            }
            if !declared_uuids.insert(a.vless_uuid.to_ascii_lowercase()) {
                bail!("managed authorization snapshot contains a duplicate VLESS uuid");
            }
            authorizations.push(Authorization {
                principal_id: a.principal_id.clone(),
                credential_id: a.credential_id.clone(),
                class: a.class,
                valid_from: a.valid_from,
                valid_until: a.valid_until,
                revoked: a.revoked,
                vless_uuid: Some(SecretString::new(a.vless_uuid.clone())),
                hysteria2_password: Some(SecretString::new(a.hysteria2_password.clone())),
            });
        }
    }

    let set = AuthorizationSet { authorizations };
    set.validate().map_err(anyhow::Error::msg)?;
    let active_ids: BTreeSet<String> = set
        .active_at(now)
        .into_iter()
        .map(|a| a.credential_id.clone())
        .collect();

    let owns_external = input.compatibility_authorizations.is_some();
    let mut next: Vec<CompatUser> = users
        .iter()
        .filter(|u| !is_native_managed_user(u) && (!owns_external || !is_external_managed_user(u)))
        .cloned()
        .collect();
    let mut managed_uuids = BTreeSet::new();

    let mut slots: Vec<&LeaseSlotInput> = input.slots.iter().collect();
    slots.sort_by_key(|s| s.slot);
    for s in slots {
        let id = s
            .credential_id
            .clone()
            .unwrap_or_else(|| lease_user_id(s.slot));
        let metadata = s.credential_id.is_some();
        if (metadata && !active_ids.contains(&id)) || (!metadata && s.expires_at <= now) {
            continue;
        }
        let uuid = s.vless_uuid.to_ascii_lowercase();
        if !managed_uuids.insert(uuid.clone())
            || next
                .iter()
                .any(|u| u.vless_uuid.eq_ignore_ascii_case(&uuid))
        {
            bail!("a managed authorization uuid collides with another user");
        }
        let mut user = users
            .iter()
            .find(|u| u.id == id)
            .cloned()
            .unwrap_or_else(|| CompatUser {
                id: id.clone(),
                name: id.clone(),
                enabled: true,
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
        user.name = s.principal_id.clone().unwrap_or_else(|| id.clone());
        user.enabled = true;
        user.vless_uuid = uuid;
        user.hysteria2_password = SecretString::new(s.hysteria2_password.clone());
        user.created_at = s.valid_from.unwrap_or(user.created_at);
        user.expires_at = Some(s.expires_at);
        user.vision_off_experiment = false;
        user.google_egress_hairpin = false;
        next.push(user);
    }

    if let Some(external) = &input.compatibility_authorizations {
        let mut external: Vec<_> = external.iter().collect();
        external.sort_by(|a, b| {
            (&a.principal_id, &a.credential_id).cmp(&(&b.principal_id, &b.credential_id))
        });
        for a in external {
            if !active_ids.contains(&a.credential_id) {
                continue;
            }
            let uuid = a.vless_uuid.to_ascii_lowercase();
            if !managed_uuids.insert(uuid.clone())
                || next
                    .iter()
                    .any(|u| u.vless_uuid.eq_ignore_ascii_case(&uuid))
            {
                bail!("a managed authorization uuid collides with another user");
            }
            let mut user = users
                .iter()
                .find(|u| u.id == a.credential_id)
                .cloned()
                .unwrap_or_else(|| CompatUser {
                    id: a.credential_id.clone(),
                    name: a.principal_id.clone(),
                    enabled: true,
                    vless_uuid: String::new(),
                    hysteria2_password: SecretString::new(String::new()),
                    subscription_token_hash_hex: credentials::hash_token(
                        &credentials::generate_subscription_token(),
                    ),
                    created_at: a.valid_from,
                    expires_at: None,
                    vision_off_experiment: false,
                    google_egress_hairpin: false,
                    peer_credentials: Default::default(),
                    is_reserved_probe: false,
                });
            user.name = a.principal_id.clone();
            user.enabled = true;
            user.vless_uuid = uuid;
            user.hysteria2_password = SecretString::new(a.hysteria2_password.clone());
            user.created_at = a.valid_from;
            user.expires_at = Some(a.valid_until);
            user.vision_off_experiment = false;
            user.google_egress_hairpin = false;
            next.push(user);
        }
    }

    let changed = serde_json::to_value(&next)? != serde_json::to_value(users)?;
    Ok((next, changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use compat_config::authorization::COMPAT_MIN_LIFETIME_SECS;

    fn slot(slot: u32, uuid_tail: &str, exp: i64) -> LeaseSlotInput {
        LeaseSlotInput {
            slot,
            vless_uuid: format!("00000000-0000-4000-8000-{uuid_tail:0>12}"),
            hysteria2_password: format!("test-password-{slot:04}-xxxx"),
            expires_at: exp,
            principal_id: None,
            credential_id: None,
            class: None,
            valid_from: None,
            revoked: false,
        }
    }

    fn customer() -> CompatUser {
        CompatUser {
            id: "user_1".into(),
            name: "customer".into(),
            enabled: true,
            vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
            hysteria2_password: SecretString::new("customer-password-0001"),
            subscription_token_hash_hex: "00".into(),
            created_at: 1,
            expires_at: None,
            vision_off_experiment: false,
            google_egress_hairpin: false,
            peer_credentials: Default::default(),
            is_reserved_probe: false,
        }
    }

    #[test]
    fn creates_pseudonymous_expiring_slot_users_and_keeps_others() {
        let users = vec![customer()];
        let input = LeasePoolInput {
            slots: vec![slot(1, "b", 2000), slot(0, "a", 1000)],
            compatibility_authorizations: None,
        };
        let (next, changed) = reconcile(&users, &input, 10).unwrap();
        assert!(changed);
        assert_eq!(next[0].id, "user_1");
        assert_eq!(next[1].id, "lease-0000");
        assert_eq!(next[1].name, "lease-0000");
        assert_eq!(next[1].expires_at, Some(1000));
        assert!(next[1].is_active(999));
        assert!(!next[1].is_active(1000), "expired slot must not render");
        assert_eq!(next[2].id, "lease-0001");
    }

    #[test]
    fn rotation_replaces_secret_and_is_idempotent() {
        let (a, _) = reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1000)],
                compatibility_authorizations: None,
            },
            10,
        )
        .unwrap();
        let (same, changed) = reconcile(
            &a,
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1000)],
                compatibility_authorizations: None,
            },
            20,
        )
        .unwrap();
        assert!(!changed, "re-applying the same pool is a no-op");
        assert_eq!(
            same[0].subscription_token_hash_hex,
            a[0].subscription_token_hash_hex
        );
        let (rotated, changed) = reconcile(
            &a,
            &LeasePoolInput {
                slots: vec![slot(0, "c", 2000)],
                compatibility_authorizations: None,
            },
            20,
        )
        .unwrap();
        assert!(changed);
        assert!(rotated[0].vless_uuid.ends_with('c'));
    }

    #[test]
    fn shrinking_the_pool_removes_slot_users_only() {
        let (a, _) = reconcile(
            &[customer()],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1000), slot(1, "b", 1000)],
                compatibility_authorizations: None,
            },
            10,
        )
        .unwrap();
        let (b, _) = reconcile(
            &a,
            &LeasePoolInput {
                slots: vec![],
                compatibility_authorizations: None,
            },
            10,
        )
        .unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].id, "user_1");
    }

    #[test]
    fn rejects_malformed_duplicate_or_colliding_input() {
        let bad_uuid = LeaseSlotInput {
            vless_uuid: "not-a-uuid".into(),
            ..slot(0, "a", 1000)
        };
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![bad_uuid],
                compatibility_authorizations: None,
            },
            1
        )
        .is_err());
        let short_pw = LeaseSlotInput {
            hysteria2_password: "short".into(),
            ..slot(0, "a", 1000)
        };
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![short_pw],
                compatibility_authorizations: None,
            },
            1
        )
        .is_err());
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1), slot(0, "b", 1)],
                compatibility_authorizations: None,
            },
            1
        )
        .is_err());
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1), slot(1, "a", 1)],
                compatibility_authorizations: None,
            },
            1
        )
        .is_err());
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 0)],
                compatibility_authorizations: None,
            },
            1
        )
        .is_err());
        let collide = LeaseSlotInput {
            vless_uuid: customer().vless_uuid,
            ..slot(0, "a", 1000)
        };
        assert!(reconcile(
            &[customer()],
            &LeasePoolInput {
                slots: vec![collide],
                compatibility_authorizations: None,
            },
            1
        )
        .is_err());
    }

    #[test]
    fn authorization_metadata_projects_into_the_authoritative_user_store() {
        let mut native = slot(0, "a", 1_800);
        native.principal_id = Some("native_opaque001".into());
        native.credential_id = Some("cred_native001".into());
        native.class = Some(CredentialClass::Native);
        native.valid_from = Some(0);
        let (first, _) = reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![native],
                compatibility_authorizations: None,
            },
            0,
        )
        .unwrap();
        assert_eq!(first[0].id, "cred_native001");
        assert_eq!(first[0].name, "native_opaque001");
        assert!(first[0].is_active(1_799));

        let mut renewed = slot(0, "a", 2_700);
        renewed.principal_id = Some("native_opaque001".into());
        renewed.credential_id = Some("cred_native001".into());
        renewed.class = Some(CredentialClass::Native);
        renewed.valid_from = Some(900);
        let (second, changed) = reconcile(
            &first,
            &LeasePoolInput {
                slots: vec![renewed],
                compatibility_authorizations: None,
            },
            900,
        )
        .unwrap();
        assert!(changed);
        assert_eq!(second[0].vless_uuid, first[0].vless_uuid);
        assert_eq!(second[0].hysteria2_password, first[0].hysteria2_password);
        assert_eq!(second[0].expires_at, Some(2_700));
    }

    fn external(
        credential_id: &str,
        uuid_tail: &str,
        valid_from: i64,
        valid_until: i64,
        revoked: bool,
    ) -> CompatibilityAuthorizationInput {
        CompatibilityAuthorizationInput {
            principal_id: "ext_opaque0001".into(),
            credential_id: credential_id.into(),
            class: CredentialClass::Compatibility,
            logical_route_id: "route_de_fast".into(),
            valid_from,
            valid_until,
            revoked,
            vless_uuid: format!("10000000-0000-4000-8000-{uuid_tail:0>12}"),
            hysteria2_password: format!("external-password-{uuid_tail}-xxxx"),
        }
    }

    #[test]
    fn compatibility_overlap_and_revocation_project_fail_closed() {
        let input = LeasePoolInput {
            slots: vec![],
            compatibility_authorizations: Some(vec![
                external("cred_rotate001", "a", 0, COMPAT_MIN_LIFETIME_SECS, true),
                external(
                    "cred_rotate002",
                    "b",
                    1,
                    1 + COMPAT_MIN_LIFETIME_SECS,
                    false,
                ),
            ]),
        };
        let (users, _) = reconcile(&[], &input, 10).unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "cred_rotate002");
        assert!(users[0].is_active(10));
        let persisted = serde_json::to_vec(&users).unwrap();
        let restored: Vec<CompatUser> = serde_json::from_slice(&persisted).unwrap();
        assert!(restored[0].is_active(10));
    }

    #[test]
    fn native_and_external_ownership_coexist_without_cross_deletion() {
        let mut native = slot(0, "a", 1_800);
        native.principal_id = Some("native_opaque001".into());
        native.credential_id = Some("cred_native001".into());
        native.class = Some(CredentialClass::Native);
        native.valid_from = Some(0);
        let combined = LeasePoolInput {
            slots: vec![native],
            compatibility_authorizations: Some(vec![external(
                "cred_external001",
                "b",
                0,
                COMPAT_MIN_LIFETIME_SECS,
                false,
            )]),
        };
        let (users, _) = reconcile(&[customer()], &combined, 10).unwrap();
        assert!(users.iter().any(|u| u.id == "cred_native001"));
        assert!(users.iter().any(|u| u.id == "cred_external001"));
        assert!(users.iter().any(|u| u.id == "user_1"));

        // Staged rollout: a native-only/older agent omits the external field,
        // so ext_* users survive unchanged.
        let native_only = LeasePoolInput {
            slots: combined.slots,
            compatibility_authorizations: None,
        };
        let (preserved, _) = reconcile(&users, &native_only, 20).unwrap();
        assert!(preserved.iter().any(|u| u.id == "cred_external001"));

        // Once a new agent supplies an authoritative empty external snapshot,
        // only ext_* managed users disappear; native and legacy users remain.
        let clear_external = LeasePoolInput {
            slots: native_only.slots,
            compatibility_authorizations: Some(vec![]),
        };
        let (cleared, _) = reconcile(&preserved, &clear_external, 20).unwrap();
        assert!(!cleared.iter().any(|u| u.id == "cred_external001"));
        assert!(cleared.iter().any(|u| u.id == "cred_native001"));
        assert!(cleared.iter().any(|u| u.id == "user_1"));
    }
}