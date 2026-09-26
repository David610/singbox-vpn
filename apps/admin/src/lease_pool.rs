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
use compat_config::credentials;
use compat_config::model::CompatUser;
use compat_config::secret::SecretString;
use serde::Deserialize;
use std::collections::BTreeSet;

pub const LEASE_USER_PREFIX: &str = "lease-";
pub const MAX_SLOTS: usize = 4096;

#[derive(Deserialize)]
pub struct LeaseSlotInput {
    pub slot: u32,
    pub vless_uuid: String,
    pub hysteria2_password: String,
    pub expires_at: i64,
}

// No Debug: this carries secrets.
#[derive(Deserialize)]
pub struct LeasePoolInput {
    pub slots: Vec<LeaseSlotInput>,
}

pub fn lease_user_id(slot: u32) -> String {
    format!("{LEASE_USER_PREFIX}{slot:04}")
}

pub fn is_lease_user(user: &CompatUser) -> bool {
    user.id.starts_with(LEASE_USER_PREFIX)
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

/// Returns the new user list and whether it differs from `users`.
/// Non-lease users are carried over untouched and in order; lease users
/// are replaced by exactly the input slots (sorted by slot).
pub fn reconcile(
    users: &[CompatUser],
    input: &LeasePoolInput,
    now: i64,
) -> Result<(Vec<CompatUser>, bool)> {
    if input.slots.len() > MAX_SLOTS {
        bail!("lease pool larger than {MAX_SLOTS} slots");
    }
    let mut seen = BTreeSet::new();
    for s in &input.slots {
        if s.slot as usize >= MAX_SLOTS || !seen.insert(s.slot) {
            bail!("lease slot {} is out of range or duplicated", s.slot);
        }
        if !valid_uuid(&s.vless_uuid) || !valid_password(&s.hysteria2_password) {
            bail!("lease slot {} has a malformed credential", s.slot);
        }
        if s.expires_at <= 0 {
            bail!("lease slot {} has no expiry", s.slot);
        }
    }
    let lease_uuids: BTreeSet<String> = input
        .slots
        .iter()
        .map(|s| s.vless_uuid.to_ascii_lowercase())
        .collect();
    if lease_uuids.len() != input.slots.len() {
        bail!("lease slots must have distinct VLESS uuids");
    }
    if users
        .iter()
        .any(|u| !is_lease_user(u) && lease_uuids.contains(&u.vless_uuid.to_ascii_lowercase()))
    {
        bail!("a lease slot uuid collides with an existing user");
    }

    let mut next: Vec<CompatUser> = users
        .iter()
        .filter(|u| !is_lease_user(u))
        .cloned()
        .collect();
    let mut slots: Vec<&LeaseSlotInput> = input.slots.iter().collect();
    slots.sort_by_key(|s| s.slot);
    for s in slots {
        let id = lease_user_id(s.slot);
        let mut user = match users.iter().find(|u| u.id == id) {
            Some(existing) => existing.clone(),
            None => CompatUser {
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
            },
        };
        user.name = id;
        user.enabled = true;
        user.vless_uuid = s.vless_uuid.to_ascii_lowercase();
        user.hysteria2_password = SecretString::new(s.hysteria2_password.clone());
        user.expires_at = Some(s.expires_at);
        user.vision_off_experiment = false;
        user.google_egress_hairpin = false;
        next.push(user);
    }

    let changed = serde_json::to_value(&next)? != serde_json::to_value(users)?;
    Ok((next, changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(slot: u32, uuid_tail: &str, exp: i64) -> LeaseSlotInput {
        LeaseSlotInput {
            slot,
            vless_uuid: format!("00000000-0000-4000-8000-{uuid_tail:0>12}"),
            hysteria2_password: format!("test-password-{slot:04}-xxxx"),
            expires_at: exp,
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
        }
    }

    #[test]
    fn creates_pseudonymous_expiring_slot_users_and_keeps_others() {
        let users = vec![customer()];
        let input = LeasePoolInput {
            slots: vec![slot(1, "b", 2000), slot(0, "a", 1000)],
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
            },
            10,
        )
        .unwrap();
        let (same, changed) = reconcile(
            &a,
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1000)],
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
            },
            10,
        )
        .unwrap();
        let (b, _) = reconcile(&a, &LeasePoolInput { slots: vec![] }, 10).unwrap();
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
                slots: vec![bad_uuid]
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
                slots: vec![short_pw]
            },
            1
        )
        .is_err());
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1), slot(0, "b", 1)]
            },
            1
        )
        .is_err());
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 1), slot(1, "a", 1)]
            },
            1
        )
        .is_err());
        assert!(reconcile(
            &[],
            &LeasePoolInput {
                slots: vec![slot(0, "a", 0)]
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
                slots: vec![collide]
            },
            1
        )
        .is_err());
    }
}
