//! Guards `docs/YOUTUBE_FIX_INVARIANTS.md`: the document, the tests that
//! enforce it, the Google/YouTube domain set and the repo-level pointer to it
//! must not be silently removed or narrowed. Changing an invariant is allowed
//! — but it has to be done on purpose, in the same commit, in the test and in
//! the document.

use compat_config::model::GOOGLE_EGRESS_DOMAINS;

const INVARIANTS: &str = include_str!("../../../docs/YOUTUBE_FIX_INVARIANTS.md");
const CLAUDE_MD: &str = include_str!("../../../CLAUDE.md");
const POLICY_TESTS: &str = include_str!("relay_role_policy.rs");
const ADMIN_CLI_TESTS: &str = include_str!("../../../apps/admin/tests/cli.rs");
const SERVER_RS: &str = include_str!("../src/server.rs");
const DEPLOYMENT_RS: &str = include_str!("../src/deployment.rs");

#[test]
fn every_invariant_is_still_documented() {
    for n in 1..=13 {
        assert!(
            INVARIANTS.contains(&format!("INV-{n} ")),
            "docs/YOUTUBE_FIX_INVARIANTS.md lost INV-{n}; remove an invariant only on purpose"
        );
    }
}

#[test]
fn the_tests_the_invariants_name_still_exist() {
    for name in [
        "exit_with_hairpin_configured_adds_one_outbound_and_sniff_plus_route_rule",
        "exit_hairpin_rejects_literal_ipv6_google_destinations_without_dropping",
        "exit_hairpin_has_exactly_one_route_for_google_domains_and_no_fallback",
        "plain_exit_ipv6_policy_is_explicit_and_scoped",
    ] {
        assert!(
            INVARIANTS.contains(name),
            "{name} is no longer named in docs/YOUTUBE_FIX_INVARIANTS.md"
        );
        assert!(
            POLICY_TESTS.contains(&format!("fn {name}(")),
            "enforcing test {name} was removed or renamed; INV-* is no longer enforced"
        );
    }
}

/// The end-to-end transition test lives in the admin crate; the document and
/// that test must not drift apart or be removed without a conscious edit.
#[test]
fn the_transition_test_the_invariants_name_still_exists() {
    let name = "hairpin_invariants_survive_user_churn_reconcile_repair_and_policy_changes";
    assert!(
        INVARIANTS.contains(name),
        "{name} is no longer named in the invariants document"
    );
    assert!(
        ADMIN_CLI_TESTS.contains(&format!("fn {name}(")),
        "{name} was removed or renamed; hairpin invariants are no longer checked across transitions"
    );
    assert!(
        ADMIN_CLI_TESTS.contains("const HAIRPIN_DOMAIN_FLOOR"),
        "the frozen domain floor was removed from the transition test"
    );
}

#[test]
fn claude_md_still_points_at_the_invariants() {
    assert!(
        CLAUDE_MD.contains("docs/YOUTUBE_FIX_INVARIANTS.md"),
        "CLAUDE.md must keep pointing every session at docs/YOUTUBE_FIX_INVARIANTS.md"
    );
}

/// INV-1: a hole in the domain set breaks Shorts, because auth, player API,
/// static assets and media each live on a different domain.
#[test]
fn the_google_youtube_domain_set_is_not_narrowed() {
    for required in [
        "youtube.com",
        "youtu.be",
        "googlevideo.com",
        "ytimg.com",
        "ggpht.com",
        "youtubei.googleapis.com",
        "googleapis.com",
        "gstatic.com",
        "google.com",
    ] {
        assert!(
            GOOGLE_EGRESS_DOMAINS.contains(&required),
            "{required} was removed from GOOGLE_EGRESS_DOMAINS (INV-1)"
        );
    }
}

/// INV-7: no third-party egress service (owner decision, 2026-10-11).
#[test]
fn no_third_party_egress_service_is_wired_into_the_renderer() {
    for (file, text) in [("server.rs", SERVER_RS), ("deployment.rs", DEPLOYMENT_RS)] {
        let lower = text.to_ascii_lowercase();
        for forbidden in ["cloudflareclient", "engage.cloudflare", "\"wireguard\""] {
            assert!(
                !lower.contains(forbidden),
                "{file} mentions {forbidden}: Cloudflare WARP and other third-party egress are forbidden (INV-7)"
            );
        }
    }
}

#[test]
fn per_family_evidence_never_claims_native_recovery() {
    assert!(INVARIANTS.contains("BLOCKED_ON_DEVICE_EVIDENCE"));
    assert!(INVARIANTS.contains("does not prove"));
    assert!(INVARIANTS.contains("real_core_literal_family_and_proxy_recovery_boundaries"));
}
