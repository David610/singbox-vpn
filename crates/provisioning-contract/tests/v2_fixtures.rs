//! Schema v2 conformance fixtures (`fixtures/platform-v2/provisioning-v2`).
//! Valid documents must parse, validate and round-trip; `*-invalid-*`
//! documents must be refused.

use provisioning_contract::v2::{EndpointParams, ProvisioningDocumentV2};
use std::path::PathBuf;

fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/platform-v2/provisioning-v2");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("fixture directory exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
}

#[test]
fn valid_fixtures_validate_and_round_trip() {
    let valid: Vec<_> = fixtures().into_iter().filter(|p| !p.to_string_lossy().contains("-invalid-")).collect();
    assert!(valid.len() >= 3);
    for path in valid {
        let text = std::fs::read_to_string(&path).unwrap();
        let doc = ProvisioningDocumentV2::from_json(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let again = ProvisioningDocumentV2::from_json(&doc.to_json().unwrap()).unwrap();
        assert_eq!(doc, again, "{path:?} does not round-trip");
    }
}

#[test]
fn invalid_fixtures_are_refused() {
    let invalid: Vec<_> = fixtures().into_iter().filter(|p| p.to_string_lossy().contains("-invalid-")).collect();
    assert!(invalid.len() >= 6);
    for path in invalid {
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(ProvisioningDocumentV2::from_json(&text).is_err(), "{path:?} was accepted");
    }
}

#[test]
fn unknown_transport_fixture_keeps_known_routes_usable() {
    let path = fixtures().into_iter().find(|p| p.to_string_lossy().contains("unknown-transport")).unwrap();
    let doc = ProvisioningDocumentV2::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let unknown: Vec<_> = doc.endpoints.iter().filter(|e| e.params == EndpointParams::Unknown).collect();
    assert_eq!(unknown.len(), 1);
    let usable = doc
        .routes
        .iter()
        .filter(|r| r.hops.iter().all(|h| doc.endpoints.iter().any(|e| &e.endpoint_id == h && e.params != EndpointParams::Unknown)))
        .count();
    assert_eq!(usable, doc.routes.len() - 1);
}
