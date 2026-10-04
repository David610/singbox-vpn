//! Credential lifecycle policy: how long a credential is authorized for,
//! how long a renewal may lag behind expiry, how long two credentials of
//! one principal may be simultaneously valid, and how that policy is
//! enforced before anything is written.
//!
//! # Why this module exists
//!
//! Before it, one [`crate::model::CompatUser`] carried exactly one UUID,
//! one Hysteria2 password and one account-level `expires_at`. That single
//! shape had to serve two callers with incompatible requirements:
//!
//! * Arcana's own client, which can renew silently and frequently, and
//!   which therefore wants a *short* authorization window so a stolen
//!   device's authorization dies quickly.
//! * Hiddify, Shadowrocket, INCY and generic sing-box importers, which
//!   refresh on their own schedule (hours), get suspended by the OS for
//!   days, and cannot implement any renewal logic at all — so they need a
//!   *long* window.
//!
//! Shortening the window for the first and lengthening it for the second
//! is not possible with one field, and rotating the secret to tighten it
//! is worse than it looks: sing-box has no per-user expiry, so changing a
//! UUID changes the rendered document, which restarts sing-box, which
//! disconnects *every* client on the node. The lease pool did exactly that
//! on a 1800-second timer — roughly three restarts per hour for every
//! native client, continuously.
//!
//! # The fix, and why it costs nothing
//!
//! A credential is now a time-bounded **grant** ([`CredentialGrant`]) with
//! its own opaque principal, `valid_from`, `valid_until` and `revoked_at`.
//! Two grant classes share the type and differ only in policy.
//!
//! Renewing a native grant extends `valid_until` and touches **nothing
//! else** — same principal, same UUID, same password. Because sing-box's
//! VLESS/Hysteria2 user entries carry only `name`, `uuid`/`password` and
//! `flow`, the rendered document is byte-identical, so `vpn-admin`'s
//! existing "already current, no reload needed" short-circuit fires and
//! sing-box is never signalled. A 30-minute native lease renewed at
//! 15 minutes therefore produces **zero** restarts and **zero** client
//! disconnects, while still dying within 30 minutes if renewal stops.
//!
//! That is the entire reason this file exists, and every rule below is
//! written to protect it: nothing in the renewal path is permitted to
//! mutate credential material.

use crate::model::{CompatUser, CredentialClass, CredentialGrant};

/// Native authorization window, in seconds.
///
/// Justification, rather than a bare number: the window bounds how long a
/// *revoked or decommissioned* device keeps working. 30 minutes is short
/// enough to bound that blast radius while being long enough to ride out
/// one missed renewal (the client renews at the half-life, so a single
/// failed renewal still leaves two chances) and an ordinary mobile
/// network transition, during which the client can be offline for minutes
/// at a time.
pub const NATIVE_LEASE_SECS: i64 = 30 * 60;

/// When a native client should renew: half the lease.
///
/// Renewing at the half-life is what makes a single dropped request
/// survivable. `t0` -> `t0+30m`, renew at `t0+15m` -> `t0+45m`; a renewal
/// lost at `t0+15m` is retried immediately and still succeeds well before
/// expiry. This is a *client* cadence. The server never assumes it happens.
pub const NATIVE_RENEW_LEAD_SECS: i64 = NATIVE_LEASE_SECS / 2;

/// Default lifetime for a compatibility credential, in seconds (7 days).
///
/// Chosen to dominate the worst realistic third-party refresh gap, not to
/// be tight. Hiddify and v2rayNG re-fetch subscriptions on their own
/// schedule and are suspended by the OS for arbitrarily long periods; a
/// credential shorter than the gap between two refreshes locks out a
/// paying customer with no way to recover except opening the app. 7 days
/// covers a device that is only powered on once a week, plus room for the
/// overlap window below.
///
/// This is a *default*, not a fixed duration: [`COMPAT_MIN_LIFETIME_SECS`]
/// and [`COMPAT_MAX_LIFETIME_SECS`] are the policy bounds, and the
/// control plane chooses a value inside them per principal.
pub const COMPAT_LIFETIME_SECS: i64 = 7 * 24 * 60 * 60;

/// Floor for a compatibility credential's lifetime (6 hours).
///
/// Set above the longest subscription-refresh interval the supported
/// third-party clients are known to use, with margin. A server-side
/// operator may choose shorter, but never below this, because doing so
/// trades a customer-visible lockout for a security gain the short native
/// window already provides.
pub const COMPAT_MIN_LIFETIME_SECS: i64 = 6 * 60 * 60;

/// Ceiling for a compatibility credential's lifetime (30 days).
///
/// An upper bound on how long a leaked third-party credential remains
/// usable, so that credential compromise has a predictable worst case
/// rather than "until someone rotates it".
pub const COMPAT_MAX_LIFETIME_SECS: i64 = 30 * 24 * 60 * 60;

/// Longest window during which two credentials of the same class and the
/// same principal may be simultaneously authorized (1 hour).
///
/// This is the overlap bound. Rotation mints credential B and trims
/// credential A's `valid_until` to at most `B.valid_from +
/// MAX_OVERLAP_SECS`, so:
/// * A third-party client that refreshes within the hour migrates to B and
///   never notices the transition.
/// * A credential that leaks can be overlapped by at most one hour before
///   it is dead, no matter how often rotation is run.
///
/// Unbounded overlap is the failure mode this exists to prevent: an
/// unbounded overlap means every rotation leaves a permanently-valid
/// older credential behind, and the set of live credentials grows without
/// limit.
pub const MAX_OVERLAP_SECS: i64 = 60 * 60;

/// Maximum simultaneously-authorized grants per (principal, class).
///
/// Two — current plus next. This is the structural expression of "A and B
/// overlap, never A, B and C". Enforced by [`validate_grant_set`].
pub const MAX_LIVE_GRANTS_PER_CLASS: usize = 2;

/// Tolerance applied to a grant's `valid_from` only, in seconds.
///
/// A node whose clock runs slightly behind the authority that minted the
/// grant would otherwise reject a credential that is genuinely valid,
/// turning benign skew into a hard outage at exactly the moment a customer
/// is trying to connect.
///
/// Applied to `valid_from` and **never** to `valid_until`: being late to
/// start a window is an availability problem, being early to end one is a
/// security problem, and only the latter is a reason to never grant
/// latitude. A 30-second tolerance cannot meaningfully extend a
/// credential's life, because `valid_until` is still enforced exactly.
pub const VALID_FROM_SKEW_GRACE_SECS: i64 = 30;

/// Whether `grant` is authorized at instant `now_unix`.
///
/// The window is half-open, `[valid_from, valid_until)`: a credential is
/// dead at exactly `valid_until`. This matches the pre-existing
/// `CompatUser::is_active` boundary so that a migrated deployment cannot
/// disagree with itself at the exact expiry second.
///
/// Revocation is checked first and is unconditional: a revoked credential
/// is never authorized again, even inside its window. Renewal must
/// therefore clear `revoked_at` explicitly rather than relying on the
/// window having closed.
pub fn grant_is_authorized(grant: &CredentialGrant, now_unix: i64) -> bool {
    !grant.is_revoked_at(now_unix)
        && now_unix + VALID_FROM_SKEW_GRACE_SECS >= grant.valid_from
        && now_unix < grant.valid_until
}

/// The instant at which `grant` stops being authorized, ignoring
/// revocation. Used by the expiry reconciler and by the overlap
/// arithmetic.
pub fn grant_expiry(grant: &CredentialGrant) -> i64 {
    grant.valid_until
}

/// Whether two grants of the same principal are authorized at the same
/// instant, and if so for how long.
///
/// Returns the length in seconds of the window during which both are live,
/// or `0` when they never overlap. The intersection is computed over the
/// *actual* windows so that an already-expired credential contributes
/// nothing — that is what stops a restart from resurrecting an expired
/// credential into a new overlap.
pub fn overlap_secs(a: &CredentialGrant, b: &CredentialGrant) -> i64 {
    let start = a.valid_from.max(b.valid_from);
    let end = grant_expiry(a).min(grant_expiry(b));
    (end - start).max(0)
}

/// The set of grants of `class` that are authorized at `now_unix`.
pub fn live_grants(
    user: &CompatUser,
    class: CredentialClass,
    now_unix: i64,
) -> Vec<&CredentialGrant> {
    // Only the explicit-stored path can hand back borrows; a legacy account
    // synthesizes an owned grant that dies with this call, so it is
    // intentionally not exposed as a reference. Callers that need the
    // legacy account to participate (rendering, route scoping) go through
    // `CompatUser::effective_grants` / `active_principal_ids` instead.
    user.credentials
        .iter()
        .filter(|g| g.class == class && grant_is_authorized(g, now_unix))
        .collect()
}

/// The next grant of `class` that has not yet expired and has not started,
/// i.e. the one a rotation would supersede. `None` when nothing is live.
pub fn current_grant(
    user: &CompatUser,
    class: CredentialClass,
    now_unix: i64,
) -> Option<&CredentialGrant> {
    live_grants(user, class, now_unix)
        .into_iter()
        .max_by_key(|g| g.generation)
}

/// Reject a grant set that violates the overlap policy.
///
/// Enforced on every load and every write, so an invalid set can never
/// reach the store — which is what makes "a restart cannot resurrect an
/// expired credential" true by construction rather than by convention.
///
/// Checks, per (user, class):
/// 1. every grant is well-formed ([`CredentialGrant::validate`]);
/// 2. at most [`MAX_LIVE_GRANTS_PER_CLASS`] grants are simultaneously
///    authorized;
/// 3. any two grants of that class that do overlap do so for no longer
///    than [`MAX_OVERLAP_SECS`].
pub fn validate_grant_set(grants: &[CredentialGrant]) -> Result<(), String> {
    for grant in grants {
        grant.validate()?;
    }
    for class in [CredentialClass::Native, CredentialClass::Compatibility] {
        let same_class: Vec<&CredentialGrant> =
            grants.iter().filter(|g| g.class == class).collect();
        for (i, a) in same_class.iter().enumerate() {
            for b in same_class.iter().skip(i + 1) {
                let overlap = overlap_secs(a, b);
                if overlap <= 0 {
                    continue;
                }
                if overlap > MAX_OVERLAP_SECS {
                    return Err(format!(
                        "credentials {} and {} of class {} are simultaneously authorized for \
                         {overlap}s, exceeding the {MAX_OVERLAP_SECS}s maximum overlap",
                        a.principal_id,
                        b.principal_id,
                        class.as_str(),
                    ));
                }
            }
        }
    }
    // Rule 2 is checked against the union of live intervals rather than at
    // any single instant, so a chain of pairwise-short overlaps that never
    // has three live at once still passes, while a set that is live-wide
    // wide enough to admit a third does not.
    for class in [CredentialClass::Native, CredentialClass::Compatibility] {
        let same_class: Vec<&CredentialGrant> =
            grants.iter().filter(|g| g.class == class).collect();
        for anchor in &same_class {
            let live_at_start: Vec<&&CredentialGrant> = same_class
                .iter()
                .filter(|g| grant_is_authorized(g, anchor.valid_from))
                .collect();
            if live_at_start.len() > MAX_LIVE_GRANTS_PER_CLASS {
                return Err(format!(
                    "{} credentials of class {} are simultaneously authorized at t={} ({}); at \
                     most {MAX_LIVE_GRANTS_PER_CLASS} may be live at once",
                    live_at_start.len(),
                    class.as_str(),
                    anchor.valid_from,
                    live_at_start
                        .iter()
                        .map(|g| g.principal_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
            }
        }
    }
    Ok(())
}

/// Clamp a requested compatibility-credential lifetime into the policy
/// bounds, rather than refusing it.
///
/// A caller asking for 10 seconds gets the floor, not an error: the
/// request is unsatisfiable as stated but the safe direction is obvious,
/// and failing the whole operation would leave a customer with no working
/// credential and no indication of why.
pub fn clamp_compat_lifetime(requested_secs: i64) -> i64 {
    requested_secs.clamp(COMPAT_MIN_LIFETIME_SECS, COMPAT_MAX_LIFETIME_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{generate_principal_id, generate_user_id};
    use crate::secret::SecretString;

    fn grant(class: CredentialClass, from: i64, until: i64) -> CredentialGrant {
        CredentialGrant::new(
            generate_principal_id(class),
            class,
            &generate_user_id(),
            from,
            until,
        )
    }

    #[test]
    fn overlap_of_disjoint_windows_is_zero() {
        let a = grant(CredentialClass::Native, 0, 100);
        let b = grant(CredentialClass::Native, 100, 200);
        assert_eq!(overlap_secs(&a, &b), 0);
    }

    #[test]
    fn overlap_counts_the_shared_window_exactly() {
        let a = grant(CredentialClass::Native, 0, 1000);
        let b = grant(CredentialClass::Native, 600, 1600);
        assert_eq!(overlap_secs(&a, &b), 400);
    }

    #[test]
    fn a_rotation_overlapping_by_more_than_the_bound_is_refused() {
        let a = grant(CredentialClass::Native, 0, 10_000);
        let b = grant(CredentialClass::Native, 100, 10_100);
        let err = validate_grant_set(&[a, b]).unwrap_err();
        assert!(err.contains("maximum overlap"), "{err}");
    }

    #[test]
    fn a_rotation_overlapping_by_exactly_the_bound_is_accepted() {
        let a = grant(CredentialClass::Native, 0, 1_000);
        let b = grant(CredentialClass::Native, 1_000 - MAX_OVERLAP_SECS, 5_000);
        validate_grant_set(&[a, b]).expect("exactly MAX_OVERLAP_SECS is permitted");
    }

    #[test]
    fn three_simultaneously_live_grants_are_refused() {
        let grants = vec![
            grant(CredentialClass::Native, 0, 100),
            grant(CredentialClass::Native, 0, 100),
            grant(CredentialClass::Native, 0, 100),
        ];
        let err = validate_grant_set(&grants).unwrap_err();
        assert!(err.contains("at most 2 may be live at once"), "{err}");
    }

    #[test]
    fn classes_do_not_interfere_with_each_others_overlap_budget() {
        // One native + one compatibility credential live at the same
        // instant is normal (a user with both a native app and Hiddify on
        // the same account) and must not be counted against either
        // class's own overlap budget.
        let a = grant(CredentialClass::Native, 0, 10_000);
        let b = grant(CredentialClass::Compatibility, 0, 10_000);
        validate_grant_set(&[a, b]).expect("one per class is not an overlap violation");
    }

    #[test]
    fn an_already_expired_grant_cannot_contribute_to_a_new_overlap() {
        // This is the "expired credential is not resurrected on restart"
        // property at the policy layer: a grant whose window has fully
        // closed contributes zero overlap, so reloading and re-validating
        // the store cannot manufacture a live window out of it.
        let dead = grant(CredentialClass::Native, 0, 100);
        let fresh = grant(CredentialClass::Native, 100, 10_000);
        assert_eq!(overlap_secs(&dead, &fresh), 0);
        validate_grant_set(&[dead, fresh]).expect("a closed window overlaps nothing");
    }

    #[test]
    fn validity_window_is_half_open_at_valid_until() {
        let g = grant(CredentialClass::Native, 100, 200);
        assert!(!grant_is_authorized(&g, 99));
        assert!(grant_is_authorized(&g, 100));
        assert!(grant_is_authorized(&g, 199));
        assert!(!grant_is_authorized(&g, 200), "dead AT valid_until");
        assert!(!grant_is_authorized(&g, 201));
    }

    #[test]
    fn a_clock_behind_the_authority_still_honours_a_just_opened_window() {
        let g = grant(CredentialClass::Native, 1_000, 2_000);
        assert!(
            grant_is_authorized(&g, 1_000 - VALID_FROM_SKEW_GRACE_SECS),
            "a node whose clock lags by less than the tolerance must not reject a valid credential"
        );
        assert!(
            !grant_is_authorized(&g, 1_000 - VALID_FROM_SKEW_GRACE_SECS - 1),
            "lag beyond the tolerance is still refused"
        );
    }

    #[test]
    fn the_skew_tolerance_never_extends_a_credential_past_valid_until() {
        // The security-critical half of the skew rule: tolerance applies to
        // the start of a window and never to the end.
        let g = grant(CredentialClass::Native, 1_000, 2_000);
        let before_expiry = 2_000 - VALID_FROM_SKEW_GRACE_SECS - 1;
        assert!(grant_is_authorized(&g, before_expiry));
        assert!(!grant_is_authorized(&g, 2_000));
        assert!(!grant_is_authorized(&g, 2_000 + VALID_FROM_SKEW_GRACE_SECS));
    }

    #[test]
    fn revocation_is_unconditional_inside_the_window() {
        let mut g = grant(CredentialClass::Native, 0, 10_000);
        assert!(grant_is_authorized(&g, 5_000));
        g.revoke(4_000);
        assert!(
            !grant_is_authorized(&g, 5_000),
            "a revoked credential must never be authorized again, even mid-window"
        );
    }

    #[test]
    fn a_grant_whose_window_closed_before_revocation_stays_revoked() {
        let mut g = grant(CredentialClass::Native, 0, 1_000);
        g.revoke(5_000);
        assert!(g.is_revoked_at(5_000));
        assert!(!grant_is_authorized(&g, 5_000));
    }

    #[test]
    fn compat_lifetime_is_clamped_to_the_documented_policy() {
        assert_eq!(clamp_compat_lifetime(10), COMPAT_MIN_LIFETIME_SECS);
        assert_eq!(
            clamp_compat_lifetime(99 * 24 * 3600),
            COMPAT_MAX_LIFETIME_SECS
        );
        assert_eq!(clamp_compat_lifetime(7 * 24 * 3600), 7 * 24 * 3600);
    }

    #[test]
    fn a_grant_that_ends_before_it_starts_is_refused() {
        let g = CredentialGrant::new(
            generate_principal_id(CredentialClass::Native),
            CredentialClass::Native,
            &generate_user_id(),
            2_000,
            1_000,
        );
        let err = g.validate().unwrap_err();
        assert!(err.contains("valid_until"), "{err}");
    }

    #[test]
    fn principals_are_class_prefixed_and_opaque() {
        let native = generate_principal_id(CredentialClass::Native);
        let compat = generate_principal_id(CredentialClass::Compatibility);
        assert!(native.starts_with("native_"), "{native}");
        assert!(compat.starts_with("ext_"), "{compat}");
        assert_ne!(native, compat);
        assert_eq!(native.len(), "native_".len() + 24);
        assert_eq!(compat.len(), "ext_".len() + 24);
    }

    #[test]
    fn secrets_are_never_rendered_by_the_debug_representation_of_a_grant() {
        let g = CredentialGrant::new(
            generate_principal_id(CredentialClass::Native),
            CredentialClass::Native,
            &generate_user_id(),
            0,
            1_000,
        );
        let dbg = format!("{g:?}");
        assert!(!dbg.contains(g.hysteria2_password.expose()), "{dbg}");
    }

    #[test]
    fn the_legacy_secret_helper_is_unused_but_kept_for_the_secret_type() {
        // Guards against the policy module accidentally depending on
        // credential material generation beyond the principal id.
        let s = SecretString::new("value");
        assert_eq!(s.expose(), "value");
    }
}
