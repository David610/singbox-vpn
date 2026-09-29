//! Cross-repo contract test for static-config revisions: the same fixture
//! file is asserted by vpn-web's validator
//! (`functions/lib/__tests__/static-revision.test.js`), so the control
//! plane can never accept a document this node rejects, or vice versa.

use compat_config::static_revision::parse_static_revision;

const CONTRACT: &str = include_str!("fixtures/static_revision_v1_contract.json");

fn cases(kind: &str) -> Vec<serde_json::Value> {
    let doc: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();
    doc[kind].as_array().unwrap().clone()
}

#[test]
fn every_valid_contract_document_is_accepted() {
    for case in cases("valid") {
        let bytes = serde_json::to_vec(&case).unwrap();
        assert!(
            parse_static_revision(&bytes).is_ok(),
            "must be accepted: {case} -> {:?}",
            parse_static_revision(&bytes).err()
        );
    }
}

#[test]
fn every_invalid_contract_document_is_rejected() {
    let invalid = cases("invalid");
    assert!(invalid.len() > 40);
    for case in invalid {
        let bytes = serde_json::to_vec(&case).unwrap();
        assert!(
            parse_static_revision(&bytes).is_err(),
            "must be rejected: {case}"
        );
    }
}
