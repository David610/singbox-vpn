//! Tests for the additive `schema_version` 1 extension: optional endpoint
//! metadata (`failure_domain`, operator labels, `path`) and the embedded,
//! opaque `singbox_config`.
//!
//! Two properties matter most here and are asserted directly rather than
//! inferred:
//!
//! 1. A deployment that configures none of this emits a document that is
//!    **byte-identical** to what it emitted before these fields existed.
//! 2. The forbidden-content audit still covers the WHOLE document, but the
//!    embedded config is audited structurally rather than by substring —
//!    which is what lets a correct `"insecure": false` through while still
//!    rejecting `"insecure": true`, something a substring scan cannot do.

use provisioning_contract::*;
use serde_json::json;

fn reality_endpoint(id: &str, tag: &str, host: &str) -> Endpoint {
    Endpoint {
        id: id.to_string(),
        tag: tag.to_string(),
        host: host.to_string(),
        port: 443,
        server_name: "www.example-decoy.com".to_string(),
        failure_domain: None,
        region: None,
        provider: None,
        asn: None,
        path: None,
        params: TransportParams::VlessReality {
            uuid: "00000000-0000-4000-8000-000000000001".to_string(),
            flow: Some(VLESS_FLOW_VISION.to_string()),
            reality: RealityParams {
                public_key: "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake".to_string(),
                short_id: "0a1b2c3d".to_string(),
                fingerprint: "chrome".to_string(),
            },
        },
    }
}

fn document(endpoints: Vec<Endpoint>) -> ProvisioningDocument {
    ProvisioningDocument::new(
        ServerInfo::current("0.0.0-test"),
        vec![Capability::VlessReality],
        endpoints,
    )
}

/// A config shaped exactly like `render_singbox_client_subscription`
/// emits: one outbound per endpoint, an `auto` urltest group, a `select`
/// selector listing every endpoint tag plus `auto`, a `direct` outbound,
/// and `route.final` naming the selector.
fn singbox_config_for(tags: &[&str]) -> serde_json::Value {
    let mut outbounds: Vec<serde_json::Value> = tags
        .iter()
        .map(|t| {
            json!({
                "type": "vless",
                "tag": t,
                "server": "vpn.example.com",
                "server_port": 443,
                "uuid": "00000000-0000-4000-8000-000000000001",
                "flow": VLESS_FLOW_VISION,
                "tls": {
                    "enabled": true,
                    "server_name": "www.example-decoy.com",
                    "utls": { "enabled": true, "fingerprint": "chrome" },
                    "reality": {
                        "enabled": true,
                        "public_key": "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake",
                        "short_id": "0a1b2c3d"
                    }
                }
            })
        })
        .collect();
    let mut selector_options: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
    selector_options.push("auto".to_string());
    outbounds.push(json!({
        "type": "urltest",
        "tag": "auto",
        "outbounds": tags,
        "url": "https://www.gstatic.com/generate_204",
        "interval": "1m",
    }));
    outbounds.push(json!({
        "type": "selector",
        "tag": "select",
        "outbounds": selector_options,
        "default": tags[0],
    }));
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));
    json!({ "outbounds": outbounds, "route": { "final": "select" } })
}

// ---------------------------------------------------------------------
// 1. Absence changes nothing
// ---------------------------------------------------------------------

#[test]
fn a_document_with_no_new_fields_serializes_without_any_of_their_keys() {
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Reality",
        "vpn.example.com",
    )]);
    let json = doc.to_json().expect("valid");
    for absent in [
        "failure_domain",
        "region",
        "provider",
        "asn",
        "\"path\"",
        "singbox_config",
    ] {
        assert!(
            !json.contains(absent),
            "an unconfigured deployment must not emit {absent}; got {json}"
        );
    }
}

#[test]
fn schema_version_stays_one_with_the_new_fields_populated() {
    let mut ep = reality_endpoint("reality-1", "Reality", "vpn.example.com");
    ep.failure_domain = Some("eu1".into());
    ep.region = Some("nl".into());
    ep.path = Some(PathType::Direct);
    let doc = document(vec![ep]).with_singbox_config(singbox_config_for(&["Reality"]));
    doc.validate().expect("valid");
    assert_eq!(
        doc.schema_version, SCHEMA_VERSION,
        "these are additive fields; the wire version must not change"
    );
}

#[test]
fn metadata_round_trips_through_json() {
    let mut ep = reality_endpoint("eu2-reality", "Europe 2", "vpn2.example.net");
    ep.failure_domain = Some("eu2".into());
    ep.region = Some("nl".into());
    ep.provider = Some("provider-b".into());
    ep.asn = Some("operator-label".into());
    ep.path = Some(PathType::Direct);
    let doc = document(vec![ep]);
    let back = ProvisioningDocument::from_json(&doc.to_json().unwrap()).unwrap();
    let ep = &back.endpoints[0];
    assert_eq!(ep.failure_domain.as_deref(), Some("eu2"));
    assert_eq!(ep.region.as_deref(), Some("nl"));
    assert_eq!(ep.provider.as_deref(), Some("provider-b"));
    assert_eq!(ep.asn.as_deref(), Some("operator-label"));
    assert_eq!(ep.path, Some(PathType::Direct));
}

#[test]
fn an_unknown_path_value_round_trips_rather_than_failing_the_parse() {
    let mut ep = reality_endpoint("reality-1", "Reality", "vpn.example.com");
    ep.path = Some(PathType::Other("relay-via-c".into()));
    let doc = document(vec![ep]);
    let back = ProvisioningDocument::from_json(&doc.to_json().unwrap()).unwrap();
    assert_eq!(
        back.endpoints[0].path,
        Some(PathType::Other("relay-via-c".into())),
        "a v1 consumer must skip unknown values, not reject the document"
    );
}

// ---------------------------------------------------------------------
// 2. The embedded config's structural audit
// ---------------------------------------------------------------------

#[test]
fn insecure_false_is_accepted_because_it_asserts_verification_is_on() {
    // The whole reason the embedded config cannot use the envelope's
    // substring audit: `"insecure": false` is security-POSITIVE, and a
    // scan for the word "insecure" cannot tell it from the opt-out.
    let mut cfg = singbox_config_for(&["Reality"]);
    cfg["outbounds"][0]["tls"]["insecure"] = json!(false);
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Reality",
        "vpn.example.com",
    )])
    .with_singbox_config(cfg);
    doc.validate()
        .expect("insecure:false must be accepted — it is the correct value");
}

#[test]
fn insecure_true_is_rejected_anywhere_in_the_embedded_config() {
    let mut cfg = singbox_config_for(&["Reality"]);
    cfg["outbounds"][0]["tls"]["insecure"] = json!(true);
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Reality",
        "vpn.example.com",
    )])
    .with_singbox_config(cfg);
    let err = doc
        .validate()
        .expect_err("certificate-verification opt-out");
    assert!(
        matches!(err, ContractError::EmbeddedConfigInvalid { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_private_key_anywhere_in_the_embedded_config_is_rejected() {
    let mut cfg = singbox_config_for(&["Reality"]);
    cfg["outbounds"][0]["tls"]["reality"]["private_key"] = json!("deadbeef");
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Reality",
        "vpn.example.com",
    )])
    .with_singbox_config(cfg);
    assert!(
        doc.validate().is_err(),
        "a peer or local private key must never reach a client"
    );
}

#[test]
fn client_owned_policy_blocks_are_rejected_by_the_top_level_allowlist() {
    for forbidden_key in ["dns", "inbounds", "experimental", "log"] {
        let mut cfg = singbox_config_for(&["Reality"]);
        cfg[forbidden_key] = json!({});
        let doc = document(vec![reality_endpoint(
            "reality-1",
            "Reality",
            "vpn.example.com",
        )])
        .with_singbox_config(cfg);
        let err = doc
            .validate()
            .unwrap_err_or_else_message(&format!("{forbidden_key} must be rejected"));
        assert!(
            matches!(err, ContractError::EmbeddedConfigInvalid { .. }),
            "{forbidden_key}: got {err:?}"
        );
    }
}

#[test]
fn a_server_filesystem_path_in_the_embedded_config_is_rejected() {
    let mut cfg = singbox_config_for(&["Reality"]);
    cfg["outbounds"][0]["tls"]["certificate_path"] = json!("/etc/vpn/cert.pem");
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Reality",
        "vpn.example.com",
    )])
    .with_singbox_config(cfg);
    assert!(doc.validate().is_err());
}

#[test]
fn the_envelope_substring_audit_still_rejects_forbidden_content_outside_the_config() {
    // Regression guard: splitting the audit must not create a hole in the
    // envelope's own coverage.
    let mut ep = reality_endpoint("reality-1", "Reality", "vpn.example.com");
    ep.region = Some("/etc/secret".into());
    let doc = document(vec![ep]);
    let err = doc.validate().expect_err("envelope audit still applies");
    assert!(
        matches!(err, ContractError::ForbiddenContent { .. }),
        "got {err:?}"
    );
}

// ---------------------------------------------------------------------
// 3. Cross-validation between catalog and embedded config
// ---------------------------------------------------------------------

#[test]
fn every_endpoint_tag_must_have_a_matching_core_outbound() {
    // Two endpoints in the catalog, but the config only renders one.
    let doc = document(vec![
        reality_endpoint("reality-1", "Europe 1", "vpn.example.com"),
        reality_endpoint("eu2-reality", "Europe 2", "vpn2.example.net"),
    ])
    .with_singbox_config(singbox_config_for(&["Europe 1"]));
    let err = doc.validate().expect_err("catalog/config disagreement");
    assert!(
        matches!(err, ContractError::EndpointTagMissingOutbound { .. }),
        "got {err:?}"
    );
}

#[test]
fn the_selector_must_list_exactly_the_endpoint_tags_plus_auto() {
    let mut cfg = singbox_config_for(&["Europe 1"]);
    // Selector advertises a tag that is not an endpoint in the catalog.
    cfg["outbounds"][2]["outbounds"] = json!(["Europe 1", "Ghost", "auto"]);
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Europe 1",
        "vpn.example.com",
    )])
    .with_singbox_config(cfg);
    let err = doc.validate().expect_err("selector/catalog disagreement");
    assert!(
        matches!(err, ContractError::SelectorOptionsMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_selector_missing_an_endpoint_tag_is_rejected() {
    let mut cfg = singbox_config_for(&["Europe 1", "Europe 2"]);
    cfg["outbounds"][3]["outbounds"] = json!(["Europe 1", "auto"]);
    let doc = document(vec![
        reality_endpoint("reality-1", "Europe 1", "vpn.example.com"),
        reality_endpoint("eu2-reality", "Europe 2", "vpn2.example.net"),
    ])
    .with_singbox_config(cfg);
    assert!(
        matches!(
            doc.validate(),
            Err(ContractError::SelectorOptionsMismatch { .. })
        ),
        "an endpoint the selector cannot reach is unusable"
    );
}

#[test]
fn route_final_must_name_the_selector_group() {
    let mut cfg = singbox_config_for(&["Europe 1"]);
    cfg["route"]["final"] = json!("direct");
    let doc = document(vec![reality_endpoint(
        "reality-1",
        "Europe 1",
        "vpn.example.com",
    )])
    .with_singbox_config(cfg);
    let err = doc
        .validate()
        .expect_err("route.final must reach the selector");
    assert!(
        matches!(err, ContractError::RouteFinalMismatch { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_well_formed_two_domain_document_validates() {
    let mut a = reality_endpoint("reality-1", "Europe 1", "vpn.example.com");
    a.failure_domain = Some("eu1".into());
    let mut b = reality_endpoint("eu2-reality", "Europe 2", "vpn2.example.net");
    b.failure_domain = Some("eu2".into());
    b.provider = Some("provider-b".into());
    let doc =
        document(vec![a, b]).with_singbox_config(singbox_config_for(&["Europe 1", "Europe 2"]));
    doc.validate().expect("a correct multi-domain document");
}

/// Small readability helper so the allowlist loop can report which key it
/// was checking when an expected rejection does not happen.
trait ExpectErrWithMessage {
    type Err;
    fn unwrap_err_or_else_message(self, msg: &str) -> Self::Err;
}

impl<T, E> ExpectErrWithMessage for Result<T, E> {
    type Err = E;
    fn unwrap_err_or_else_message(self, msg: &str) -> E {
        match self {
            Ok(_) => panic!("{msg}"),
            Err(e) => e,
        }
    }
}
