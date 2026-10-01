//! Time-bounded, opaque VPN authorization records.
//!
//! This module is deliberately independent from account and billing data.  A node accepts
//! snapshots containing only opaque principal/credential identifiers, protocol secrets and
//! validity bounds.  Callers render [`AuthorizationSet::active_at`] into sing-box; therefore an
//! expired or revoked credential cannot be resurrected by a process restart.

use crate::secret::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const NATIVE_LEASE_SECS: i64 = 30 * 60;
pub const NATIVE_RENEW_AFTER_SECS: i64 = 15 * 60;
pub const COMPAT_MIN_LIFETIME_SECS: i64 = 6 * 60 * 60;
pub const COMPAT_MAX_LIFETIME_SECS: i64 = 30 * 24 * 60 * 60;
pub const MAX_OVERLAP_SECS: i64 = 48 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialClass {
    Native,
    Compatibility,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Authorization {
    pub principal_id: String,
    pub credential_id: String,
    pub class: CredentialClass,
    pub valid_from: i64,
    pub valid_until: i64,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vless_uuid: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hysteria2_password: Option<SecretString>,
}

impl Authorization {
    pub fn is_active_at(&self, now: i64) -> bool {
        !self.revoked && self.valid_from <= now && now < self.valid_until
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthorizationSet {
    pub authorizations: Vec<Authorization>,
}

impl AuthorizationSet {
    /// Validate a control-plane snapshot before it can replace live state.
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for item in &self.authorizations {
            validate_principal_id(&item.principal_id)?;
            validate_credential_id(&item.credential_id)?;
            match item.class {
                CredentialClass::Native if !item.principal_id.starts_with("native_") => {
                    return Err("native credential requires a native_ principal".into())
                }
                CredentialClass::Compatibility if !item.principal_id.starts_with("ext_") => {
                    return Err("compatibility credential requires an ext_ principal".into())
                }
                _ => {}
            }
            if !ids.insert(item.credential_id.as_str()) {
                return Err(format!("duplicate credential_id {:?}", item.credential_id));
            }
            let lifetime = item
                .valid_until
                .checked_sub(item.valid_from)
                .ok_or("invalid validity range")?;
            if lifetime <= 0 {
                return Err(format!(
                    "credential {:?} has an empty validity interval",
                    item.credential_id
                ));
            }
            if item.vless_uuid.is_none() && item.hysteria2_password.is_none() {
                return Err(format!(
                    "credential {:?} has no protocol secret",
                    item.credential_id
                ));
            }
            match item.class {
                CredentialClass::Native if lifetime > NATIVE_LEASE_SECS => {
                    return Err(format!(
                        "native credential {:?} exceeds the 30 minute lease policy",
                        item.credential_id
                    ))
                }
                CredentialClass::Compatibility
                    if !(COMPAT_MIN_LIFETIME_SECS..=COMPAT_MAX_LIFETIME_SECS)
                        .contains(&lifetime) =>
                {
                    return Err(format!(
                        "compatibility credential {:?} lifetime is outside policy",
                        item.credential_id
                    ))
                }
                _ => {}
            }
        }
        for principal in self
            .authorizations
            .iter()
            .map(|a| &a.principal_id)
            .collect::<BTreeSet<_>>()
        {
            let credentials: Vec<_> = self
                .authorizations
                .iter()
                .filter(|a| &a.principal_id == principal)
                .collect();
            // Bound every pair's overlap independently.
            for (i, a) in credentials.iter().enumerate() {
                for b in credentials.iter().skip(i + 1) {
                    let overlap = a.valid_until.min(b.valid_until) - a.valid_from.max(b.valid_from);
                    if overlap > MAX_OVERLAP_SECS {
                        return Err(format!(
                            "principal {:?} overlap exceeds 48 hours",
                            a.principal_id
                        ));
                    }
                }
            }
            // End events sort before start events, preserving [from, until) semantics.
            let mut events = Vec::with_capacity(credentials.len() * 2);
            for credential in credentials {
                events.push((credential.valid_from, 1i8));
                events.push((credential.valid_until, -1i8));
            }
            events.sort_unstable_by_key(|&(at, delta)| (at, delta));
            let mut active = 0i64;
            for (_, delta) in events {
                active += i64::from(delta);
                if active > 2 {
                    return Err(format!(
                        "principal {principal:?} has more than two simultaneous credentials"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Deterministic, fail-closed view used for every render and restart.
    pub fn active_at(&self, now: i64) -> Vec<&Authorization> {
        let mut active: Vec<_> = self
            .authorizations
            .iter()
            .filter(|a| a.is_active_at(now))
            .collect();
        active.sort_by(|a, b| {
            (&a.principal_id, &a.credential_id).cmp(&(&b.principal_id, &b.credential_id))
        });
        active
    }

    /// Renew authorization without changing protocol secrets or disconnecting an existing tunnel.
    pub fn renew_native(&mut self, credential_id: &str, now: i64) -> Result<(), String> {
        let item = self
            .authorizations
            .iter_mut()
            .find(|a| a.credential_id == credential_id)
            .ok_or("credential not found")?;
        if item.class != CredentialClass::Native || item.revoked || !item.is_active_at(now) {
            return Err("credential is not an active native lease".into());
        }
        // This is a rolling authorization window, not credential age. Advancing valid_from
        // keeps the persisted interval self-validating after arbitrarily many renewals.
        item.valid_from = now;
        item.valid_until = now
            .checked_add(NATIVE_LEASE_SECS)
            .ok_or("renewal timestamp overflow")?;
        Ok(())
    }
}

fn validate_principal_id(value: &str) -> Result<(), String> {
    validate_id(value, &["native_", "ext_"], "principal_id")
}

fn validate_credential_id(value: &str) -> Result<(), String> {
    validate_id(value, &["cred_"], "credential_id")
}

fn validate_id(value: &str, prefixes: &[&str], field: &str) -> Result<(), String> {
    if !prefixes.iter().any(|prefix| value.starts_with(prefix))
        || value.len() < 12
        || value.len() > 96
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(format!(
            "{field} has an invalid opaque identifier prefix or shape"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn auth(id: &str, from: i64, until: i64) -> Authorization {
        Authorization {
            principal_id: "ext_opaque001".into(),
            credential_id: id.into(),
            class: CredentialClass::Compatibility,
            valid_from: from,
            valid_until: until,
            revoked: false,
            vless_uuid: Some(SecretString::new("secret")),
            hysteria2_password: None,
        }
    }
    fn native(id: &str, from: i64) -> Authorization {
        Authorization {
            principal_id: "native_opaque001".into(),
            credential_id: id.into(),
            class: CredentialClass::Native,
            valid_from: from,
            valid_until: from + NATIVE_LEASE_SECS,
            revoked: false,
            vless_uuid: Some(SecretString::new("11111111-1111-4111-8111-111111111111")),
            hysteria2_password: Some(SecretString::new("native-secret-password")),
        }
    }
    #[test]
    fn rotation_boundaries_and_revocation_are_fail_closed() {
        let mut s = AuthorizationSet {
            authorizations: vec![
                auth("cred_aaaa001", 0, 30_000),
                auth("cred_bbbb001", 20_000, 50_000),
            ],
        };
        assert_eq!(s.active_at(19_999).len(), 1);
        assert_eq!(s.active_at(20_000).len(), 2);
        assert_eq!(s.active_at(30_000).len(), 1);
        s.authorizations[0].revoked = true;
        assert_eq!(s.active_at(25_000).len(), 1);
        s.authorizations[1].revoked = true;
        assert!(s.active_at(25_000).is_empty());
    }
    #[test]
    fn restart_view_never_restores_expired_state() {
        let json = serde_json::to_vec(&AuthorizationSet {
            authorizations: vec![auth("cred_aaaa001", 0, 30_000)],
        })
        .unwrap();
        let restored: AuthorizationSet = serde_json::from_slice(&json).unwrap();
        assert!(restored.active_at(30_000).is_empty());
    }
    #[test]
    fn duplicate_and_unbounded_overlap_are_rejected() {
        let mut s = AuthorizationSet {
            authorizations: vec![
                auth("cred_aaaa001", 0, 30_000),
                auth("cred_aaaa001", 1, 30_001),
            ],
        };
        assert!(s.validate().unwrap_err().contains("duplicate"));
        s.authorizations[1].credential_id = "cred_bbbb001".into();
        s.authorizations[0].valid_until = 200_000;
        s.authorizations[1].valid_until = 200_000;
        assert!(s.validate().is_err());
    }
    #[test]
    fn native_renewal_preserves_secret_and_extends_only_expiry() {
        let a = native("cred_native01", 0);
        let secret = a.vless_uuid.clone();
        let id = a.credential_id.clone();
        let mut s = AuthorizationSet {
            authorizations: vec![a],
        };
        s.validate().unwrap();
        s.renew_native("cred_native01", NATIVE_RENEW_AFTER_SECS)
            .unwrap();
        s.validate().unwrap();
        assert_eq!(s.authorizations[0].valid_from, NATIVE_RENEW_AFTER_SECS);
        assert_eq!(
            s.authorizations[0].valid_until,
            NATIVE_RENEW_AFTER_SECS + NATIVE_LEASE_SECS
        );
        assert_eq!(s.authorizations[0].vless_uuid, secret);
        assert_eq!(s.authorizations[0].credential_id, id);
    }

    #[test]
    fn native_can_renew_for_days_without_extending_the_horizon() {
        let mut set = AuthorizationSet {
            authorizations: vec![native("cred_native01", 0)],
        };
        let original = serde_json::to_value((
            &set.authorizations[0].credential_id,
            &set.authorizations[0].vless_uuid,
            &set.authorizations[0].hysteria2_password,
        ))
        .unwrap();
        let mut now = NATIVE_RENEW_AFTER_SECS;
        for _ in 0..(4 * 24 * 7) {
            set.renew_native("cred_native01", now).unwrap();
            set.validate().unwrap();
            assert_eq!(set.authorizations[0].valid_until - now, NATIVE_LEASE_SECS);
            now += NATIVE_RENEW_AFTER_SECS;
        }
        assert_eq!(
            serde_json::to_value((
                &set.authorizations[0].credential_id,
                &set.authorizations[0].vless_uuid,
                &set.authorizations[0].hysteria2_password
            ))
            .unwrap(),
            original
        );
    }

    #[test]
    fn native_renewal_rejects_expiry_revocation_boundary_and_overflow() {
        let mut expired = AuthorizationSet {
            authorizations: vec![native("cred_native01", 0)],
        };
        assert!(expired
            .renew_native("cred_native01", NATIVE_LEASE_SECS)
            .is_err());
        let mut revoked = AuthorizationSet {
            authorizations: vec![native("cred_native02", 0)],
        };
        revoked.authorizations[0].revoked = true;
        assert!(revoked.renew_native("cred_native02", 1).is_err());
        let mut overflow = AuthorizationSet {
            authorizations: vec![native("cred_native03", i64::MAX - NATIVE_LEASE_SECS)],
        };
        assert!(overflow
            .renew_native("cred_native03", i64::MAX - 1)
            .is_err());
    }

    #[test]
    fn sweep_line_accepts_sequential_rotation_and_rejects_true_triple_overlap() {
        let mut a = auth("cred_rotate01", 0, COMPAT_MIN_LIFETIME_SECS);
        let mut b = auth("cred_rotate02", 1, 1 + COMPAT_MIN_LIFETIME_SECS);
        let mut c = auth(
            "cred_rotate03",
            COMPAT_MIN_LIFETIME_SECS,
            2 * COMPAT_MIN_LIFETIME_SECS,
        );
        // A/B overlap, then B/C overlap; A/C merely touch.
        assert!(AuthorizationSet {
            authorizations: vec![a.clone(), b.clone(), c.clone()]
        }
        .validate()
        .is_ok());
        c.valid_from = 2;
        assert!(AuthorizationSet {
            authorizations: vec![a.clone(), b.clone(), c]
        }
        .validate()
        .unwrap_err()
        .contains("simultaneous"));
        // Exactly 48 hours is accepted; one second more is not.
        a.valid_until = MAX_OVERLAP_SECS + COMPAT_MIN_LIFETIME_SECS;
        b.valid_from = COMPAT_MIN_LIFETIME_SECS;
        b.valid_until = b.valid_from + COMPAT_MAX_LIFETIME_SECS;
        assert!(AuthorizationSet {
            authorizations: vec![a.clone(), b.clone()]
        }
        .validate()
        .is_ok());
        a.valid_until += 1;
        assert!(AuthorizationSet {
            authorizations: vec![a, b]
        }
        .validate()
        .unwrap_err()
        .contains("48 hours"));
    }

    #[test]
    fn identifiers_are_typed_and_class_compatible() {
        let good_native = native("cred_native01", 0);
        assert!(AuthorizationSet {
            authorizations: vec![good_native.clone()]
        }
        .validate()
        .is_ok());
        for principal in [
            "cred_wrong000",
            "ext_email@example.com",
            "native_stripe cus_123",
        ] {
            let mut bad = good_native.clone();
            bad.principal_id = principal.into();
            assert!(AuthorizationSet {
                authorizations: vec![bad]
            }
            .validate()
            .is_err());
        }
        let mut wrong_class = good_native.clone();
        wrong_class.principal_id = "ext_opaque001".into();
        assert!(AuthorizationSet {
            authorizations: vec![wrong_class]
        }
        .validate()
        .is_err());
        let mut wrong_credential = good_native;
        wrong_credential.credential_id = "native_wrong01".into();
        assert!(AuthorizationSet {
            authorizations: vec![wrong_credential]
        }
        .validate()
        .is_err());
    }
    #[test]
    fn clock_edges_use_closed_open_intervals() {
        let a = auth("cred_edge0001", 100, 100 + COMPAT_MIN_LIFETIME_SECS);
        assert!(!a.is_active_at(99));
        assert!(a.is_active_at(100));
        assert!(!a.is_active_at(100 + COMPAT_MIN_LIFETIME_SECS));
    }
}
