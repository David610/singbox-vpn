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
            validate_opaque_id("principal_id", &item.principal_id)?;
            validate_opaque_id("credential_id", &item.credential_id)?;
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
        // At most two credentials per principal may overlap, and any pairwise overlap is bounded.
        for (i, a) in self.authorizations.iter().enumerate() {
            let mut simultaneous = 1usize;
            for b in self
                .authorizations
                .iter()
                .skip(i + 1)
                .filter(|b| b.principal_id == a.principal_id)
            {
                let overlap = a.valid_until.min(b.valid_until) - a.valid_from.max(b.valid_from);
                if overlap > MAX_OVERLAP_SECS {
                    return Err(format!(
                        "principal {:?} overlap exceeds 48 hours",
                        a.principal_id
                    ));
                }
                if overlap > 0 {
                    simultaneous += 1;
                }
            }
            if simultaneous > 2 {
                return Err(format!(
                    "principal {:?} has more than two overlapping credentials",
                    a.principal_id
                ));
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
        item.valid_until = now
            .checked_add(NATIVE_LEASE_SECS)
            .ok_or("renewal timestamp overflow")?;
        Ok(())
    }
}

fn validate_opaque_id(field: &str, value: &str) -> Result<(), String> {
    let good_prefix =
        value.starts_with("native_") || value.starts_with("ext_") || value.starts_with("cred_");
    if !good_prefix
        || value.len() < 12
        || value.len() > 96
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(format!(
            "{field} must be an opaque native_/ext_/cred_ identifier"
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
        let mut a = auth("cred_native01", 0, NATIVE_LEASE_SECS);
        a.class = CredentialClass::Native;
        a.principal_id = "native_opaque001".into();
        let secret = a.vless_uuid.clone();
        let mut s = AuthorizationSet {
            authorizations: vec![a],
        };
        s.renew_native("cred_native01", NATIVE_RENEW_AFTER_SECS)
            .unwrap();
        assert_eq!(
            s.authorizations[0].valid_until,
            NATIVE_RENEW_AFTER_SECS + NATIVE_LEASE_SECS
        );
        assert_eq!(s.authorizations[0].vless_uuid, secret);
    }
    #[test]
    fn clock_edges_use_closed_open_intervals() {
        let a = auth("cred_edge0001", 100, 100 + COMPAT_MIN_LIFETIME_SECS);
        assert!(!a.is_active_at(99));
        assert!(a.is_active_at(100));
        assert!(!a.is_active_at(100 + COMPAT_MIN_LIFETIME_SECS));
    }
}
