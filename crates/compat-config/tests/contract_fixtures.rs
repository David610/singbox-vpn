//! Cross-repo contract fixtures.
//!
//! These tests generate provisioning documents with the REAL server code
//! path (`compat_config::contract`) from clearly fake credentials, and
//! assert they match the checked-in fixtures under
//! `fixtures/singbox-client-contract/`. `singbox-client`'s CI consumes
//! those same files to test its parser — see the fixture directory's
//! README.
//!
//! Comparison is on PARSED JSON (`serde_json::Value`), never on raw
//! bytes, so reformatting a fixture cannot break the suite while a real
//! change of shape or value still does.
//!
//! Set `UPDATE_CONTRACT_FIXTURES=1` to rewrite the generated fixtures
//! after a deliberate contract change; review the resulting diff.

use compat_config::contract::{provisioning_document_with_mode, DiagnosticMode};
use compat_config::model::{CompatEndpoint, CompatTransport, CompatUser, PublicParameters};
use compat_config::render::standard_endpoints;
use compat_config::secret::SecretString;
use provisioning_contract as contract;
use std::path::PathBuf;

// --- deliberately fake, obviously-not-real credentials ---------------

const FAKE_UUID: &str = "00000000-0000-4000-8000-000000000001";
const FAKE_REALITY_PUBLIC_KEY: &str = "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake";
const FAKE_SHORT_ID: &str = "0a1b2c3d";
const FAKE_HYSTERIA2_PASSWORD: &str = "fake-hysteria2-password-not-a-real-secret";
const FAKE_OBFS_PASSWORD: &str = "fake-salamander-obfs-password";
const FAKE_HOST: &str = "vpn.example.com";
const FAKE_SNI: &str = "www.example-decoy.com";

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/singbox-client-contract")
        .canonicalize()
        .expect("fixture directory exists")
}

fn user(vision_off: bool) -> CompatUser {
    CompatUser {
        id: "user_fixture".into(),
        name: "fixture".into(),
        enabled: true,
        vless_uuid: FAKE_UUID.into(),
        hysteria2_password: SecretString::new(FAKE_HYSTERIA2_PASSWORD),
        subscription_token_hash_hex: "0".repeat(64),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: vision_off,
    }
}

fn endpoints(hysteria2: bool, obfs: Option<&str>) -> Vec<CompatEndpoint> {
    let mut eps = standard_endpoints(
        FAKE_HOST,
        443,
        443,
        FAKE_REALITY_PUBLIC_KEY,
        FAKE_SHORT_ID,
        FAKE_SNI,
        obfs,
    );
    if !hysteria2 {
        eps.retain(|e| e.transport == CompatTransport::VlessReality);
    }
    eps
}

fn reality_only_endpoints() -> Vec<CompatEndpoint> {
    endpoints(false, None)
}

fn hysteria2_only_endpoints(obfs: Option<&str>) -> Vec<CompatEndpoint> {
    endpoints(true, obfs)
        .into_iter()
        .filter(|e| e.transport == CompatTransport::Hysteria2)
        .collect()
}

/// Placeholder substituted for the real release version before a
/// document is compared against (or written to) a fixture.
///
/// `server.version` is the one field that legitimately changes on every
/// release. Pinning the real value would make a routine version bump
/// fail this suite and, worse, fail `singbox-client`'s CI — turning a
/// release chore into a cross-repo break for no contract change at all.
/// The FIELD is still exercised end to end: the fixtures contain it, and
/// the parser validates it as a non-empty string.
const FIXTURE_SERVER_VERSION: &str = "0.0.0-fixture";

fn normalize(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(v) = value.pointer_mut("/server/version") {
        *v = serde_json::Value::String(FIXTURE_SERVER_VERSION.to_string());
    }
    value
}

/// Compare a generated document against its fixture on parsed structure.
fn assert_matches_fixture(name: &str, generated: &serde_json::Value) {
    let generated = &normalize(generated.clone());
    let path = fixture_dir().join(name);
    if std::env::var_os("UPDATE_CONTRACT_FIXTURES").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(generated).unwrap()),
        )
        .expect("write fixture");
    }
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()));
    let expected: serde_json::Value = normalize(
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("fixture {name} is not JSON: {e}")),
    );
    assert_eq!(
        &expected, generated,
        "generated contract no longer matches {name}. If this is a deliberate contract change, \
         re-run with UPDATE_CONTRACT_FIXTURES=1, review the diff, and mirror it in \
         singbox-client."
    );
}

#[test]
fn fixture_reality_only_matches_generated_document() {
    let doc = provisioning_document_with_mode(
        &user(false),
        &reality_only_endpoints(),
        DiagnosticMode::None,
    )
    .unwrap();
    assert_matches_fixture("01-reality-only.json", &serde_json::to_value(&doc).unwrap());
}

#[test]
fn fixture_hysteria2_only_matches_generated_document() {
    let doc = provisioning_document_with_mode(
        &user(false),
        &hysteria2_only_endpoints(None),
        DiagnosticMode::None,
    )
    .unwrap();
    assert_matches_fixture(
        "02-hysteria2-only.json",
        &serde_json::to_value(&doc).unwrap(),
    );
}

#[test]
fn fixture_both_transports_matches_generated_document() {
    let doc =
        provisioning_document_with_mode(&user(false), &endpoints(true, None), DiagnosticMode::None)
            .unwrap();
    assert_matches_fixture(
        "03-both-transports.json",
        &serde_json::to_value(&doc).unwrap(),
    );
}

#[test]
fn fixture_hysteria2_salamander_obfs_matches_generated_document() {
    let doc = provisioning_document_with_mode(
        &user(false),
        &endpoints(true, Some(FAKE_OBFS_PASSWORD)),
        DiagnosticMode::None,
    )
    .unwrap();
    assert_matches_fixture(
        "04-hysteria2-salamander-obfs.json",
        &serde_json::to_value(&doc).unwrap(),
    );
}

#[test]
fn fixture_diagnostic_tcp_only_matches_generated_document() {
    let doc = provisioning_document_with_mode(
        &user(false),
        &endpoints(true, None),
        DiagnosticMode::TcpOnly,
    )
    .unwrap();
    assert_matches_fixture(
        "05-diagnostic-tcp-only.json",
        &serde_json::to_value(&doc).unwrap(),
    );
}

#[test]
fn fixture_diagnostic_vision_off_matches_generated_document() {
    let doc = provisioning_document_with_mode(
        &user(true),
        &endpoints(true, None),
        DiagnosticMode::VisionOff,
    )
    .unwrap();
    assert_matches_fixture(
        "06-diagnostic-vision-off.json",
        &serde_json::to_value(&doc).unwrap(),
    );
}

#[test]
fn fixture_unsupported_schema_version_error_matches_the_served_body() {
    let body = contract::UnsupportedSchemaVersion::new(2);
    assert_matches_fixture(
        "07-error-unsupported-schema-version.json",
        &serde_json::to_value(&body).unwrap(),
    );
}

/// The malformed fixture is NOT generated — it is a hand-written example
/// of a document that must be REJECTED. Both this repository and
/// `singbox-client` are expected to refuse it; this test proves this
/// side does, and pins the exact reason.
#[test]
fn fixture_malformed_document_is_rejected_with_the_documented_error() {
    let raw = std::fs::read_to_string(fixture_dir().join("08-invalid-missing-short-id.json"))
        .expect("fixture present");
    let err = contract::ProvisioningDocument::from_json(&raw)
        .expect_err("a REALITY endpoint with no short_id must never validate");
    assert_eq!(
        err,
        contract::ContractError::MissingRealityParameter {
            endpoint_id: "reality-1".into(),
            missing: "short_id",
        }
    );
}

/// Every generated fixture must parse and validate as a real contract
/// document — a fixture that this repository's own parser would reject
/// is worse than no fixture at all.
#[test]
fn every_generated_fixture_parses_and_validates() {
    for name in [
        "01-reality-only.json",
        "02-hysteria2-only.json",
        "03-both-transports.json",
        "04-hysteria2-salamander-obfs.json",
        "05-diagnostic-tcp-only.json",
        "06-diagnostic-vision-off.json",
    ] {
        let raw = std::fs::read_to_string(fixture_dir().join(name)).expect("fixture present");
        let doc = contract::ProvisioningDocument::from_json(&raw)
            .unwrap_or_else(|e| panic!("{name} failed validation: {e}"));
        assert_eq!(doc.schema_version, contract::SCHEMA_VERSION);
        assert_eq!(doc.server.product, contract::PRODUCT);
    }
}

/// The whole point of publishing these files: they must be safe to
/// commit to a public repository and to hand to another project's CI.
#[test]
fn no_fixture_contains_anything_that_could_be_a_real_secret() {
    for entry in std::fs::read_dir(fixture_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        let lowered = raw.to_lowercase();
        for forbidden in [
            "private_key",
            "-----begin",
            ".pem",
            "/etc/",
            "insecure",
            "auto_route",
            "kill_switch",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "{} contains forbidden content {forbidden}",
                path.display()
            );
        }
        // Every credential in a fixture must announce itself as fake.
        if lowered.contains("password") {
            assert!(
                lowered.contains("fake-"),
                "{} carries a password that does not look obviously fake",
                path.display()
            );
        }
    }
}

/// Guards the fixture set itself: a new transport or diagnostic added
/// without a fixture (or a fixture deleted) fails here rather than
/// silently leaving `singbox-client`'s CI testing an outdated surface.
#[test]
fn the_published_fixture_set_is_exactly_the_documented_one() {
    let mut found: Vec<String> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".json"))
        .collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            "01-reality-only.json",
            "02-hysteria2-only.json",
            "03-both-transports.json",
            "04-hysteria2-salamander-obfs.json",
            "05-diagnostic-tcp-only.json",
            "06-diagnostic-vision-off.json",
            "07-error-unsupported-schema-version.json",
            "08-invalid-missing-short-id.json",
        ]
    );
}

/// Documents the fake-credential contract itself: these constants appear
/// in the published fixtures, so a change to them is a change other
/// repositories see.
#[test]
fn fixtures_use_the_documented_fake_credentials() {
    let raw = std::fs::read_to_string(fixture_dir().join("03-both-transports.json")).unwrap();
    for expected in [
        FAKE_UUID,
        FAKE_REALITY_PUBLIC_KEY,
        FAKE_SHORT_ID,
        FAKE_HYSTERIA2_PASSWORD,
        FAKE_HOST,
        FAKE_SNI,
    ] {
        assert!(raw.contains(expected), "fixture is missing {expected}");
    }
    // Sanity: the reality endpoint really is a REALITY endpoint.
    let doc = contract::ProvisioningDocument::from_json(&raw).unwrap();
    assert!(matches!(
        doc.endpoints[0].params,
        contract::TransportParams::VlessReality { .. }
    ));
    assert!(matches!(
        doc.endpoints[1].params,
        contract::TransportParams::Hysteria2 { .. }
    ));
    // And the domain model it came from carries only public parameters.
    let eps = endpoints(true, None);
    assert!(matches!(
        eps[0].public_parameters,
        PublicParameters::Reality { .. }
    ));
}
