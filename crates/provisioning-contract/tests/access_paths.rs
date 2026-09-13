use provisioning_contract::*;

fn endpoint(path: Option<PathType>) -> Endpoint {
    Endpoint {
        id: "reality-1".into(),
        tag: "Reality".into(),
        host: "vpn.example.com".into(),
        port: 443,
        server_name: "www.example-decoy.com".into(),
        failure_domain: None,
        region: None,
        provider: None,
        asn: None,
        path,
        params: TransportParams::VlessReality {
            uuid: "00000000-0000-4000-8000-000000000001".into(),
            flow: Some(VLESS_FLOW_VISION.into()),
            reality: RealityParams {
                public_key: "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake".into(),
                short_id: "0a1b2c3d".into(),
                fingerprint: "chrome".into(),
            },
        },
    }
}

fn doc(path: Option<PathType>) -> ProvisioningDocument {
    ProvisioningDocument::new(
        ServerInfo::current("test"),
        vec![Capability::VlessReality],
        vec![endpoint(path)],
    )
}

fn relay(id: &str) -> AccessPath {
    AccessPath::new(id, AccessPathKind::Relay, vec!["tcp".into()]).with_metadata(
        Some(format!("relay:{id}")),
        Some("test-region".into()),
        Some("operator".into()),
    )
}

#[test]
fn empty_access_paths_are_absent_and_schema_stays_one() {
    let doc = doc(None);
    let json = doc.to_json().unwrap();
    assert_eq!(doc.schema_version, 1);
    assert!(!json.contains("access_paths"));
}

#[test]
fn relay_metadata_round_trips_without_secrets() {
    let doc =
        doc(Some(PathType::Other("relay-r1".into()))).with_access_paths(vec![relay("relay-r1")]);
    let encoded = doc.to_json().unwrap();
    assert!(encoded.contains("\"access_paths\""));
    assert!(encoded.contains("\"kind\": \"relay\""));
    for forbidden in ["password", "token", "private_key", "proxy_uri"] {
        assert!(!encoded.to_ascii_lowercase().contains(forbidden));
    }
    let back = ProvisioningDocument::from_json(&encoded).unwrap();
    assert_eq!(back.access_paths, vec![relay("relay-r1")]);
}

#[test]
fn duplicate_access_path_ids_are_rejected() {
    let err = doc(None)
        .with_access_paths(vec![relay("r1"), relay("r1")])
        .validate()
        .unwrap_err();
    assert!(matches!(err, ContractError::DuplicateAccessPathId { .. }));
}

#[test]
fn declared_path_set_rejects_dangling_endpoint_reference() {
    let err = doc(Some(PathType::Other("missing".into())))
        .with_access_paths(vec![relay("r1")])
        .validate()
        .unwrap_err();
    assert!(matches!(
        err,
        ContractError::UnknownAccessPathReference { .. }
    ));
}

#[test]
fn legacy_opaque_path_without_access_paths_remains_forward_compatible() {
    let doc = doc(Some(PathType::Other("future-path".into())));
    let back = ProvisioningDocument::from_json(&doc.to_json().unwrap()).unwrap();
    assert_eq!(
        back.endpoints[0].path,
        Some(PathType::Other("future-path".into()))
    );
}

#[test]
fn empty_or_duplicate_capabilities_are_rejected() {
    assert!(doc(None)
        .with_access_paths(vec![AccessPath::new("r1", AccessPathKind::Relay, vec![])])
        .validate()
        .is_err());
    assert!(doc(None)
        .with_access_paths(vec![AccessPath::new(
            "r1",
            AccessPathKind::Relay,
            vec!["tcp".into(), "tcp".into()],
        )])
        .validate()
        .is_err());
}

/// A relay catalog (one direct and one relayed route to the same exit)
/// with the embedded config shape the server renders; `auto` is the one
/// automatic group.
fn relayed_doc(auto_members: &[&str], via_detour: Option<&str>) -> ProvisioningDocument {
    let mut direct = endpoint(Some(PathType::Direct));
    direct.id = "de1-direct".into();
    direct.tag = "Direct".into();
    let mut via = endpoint(Some(PathType::Other("relay-ru1".into())));
    via.id = "de1-via-ru1".into();
    via.tag = "Via".into();
    let mut via_outbound =
        serde_json::json!({"type": "vless", "tag": "Via", "server": "vpn.example.com"});
    if let Some(detour) = via_detour {
        via_outbound["detour"] = serde_json::json!(detour);
    }
    ProvisioningDocument::new(
        ServerInfo::current("test"),
        vec![Capability::VlessReality],
        vec![direct, via],
    )
    .with_access_paths(vec![relay("relay-ru1").with_via_endpoint_id("reality-1")])
    .with_singbox_config(serde_json::json!({
        "outbounds": [
            {"type": "vless", "tag": "Reality", "server": "ru1.example.com"},
            {"type": "vless", "tag": "Direct", "server": "vpn.example.com"},
            via_outbound,
            {"type": "urltest", "tag": "auto", "outbounds": auto_members},
            {"type": "selector", "tag": "select", "outbounds": ["Direct", "Via", "auto"], "default": "Via"},
            {"type": "direct", "tag": "direct"}
        ],
        "route": {"final": "select"}
    }))
}

#[test]
fn relayed_catalog_with_privacy_only_automatic_group_is_servable() {
    relayed_doc(&["Via"], Some("Reality")).validate().unwrap();
}

#[test]
fn automatic_group_mixing_direct_and_relayed_routes_is_rejected() {
    let err = relayed_doc(&["Direct", "Via"], Some("Reality"))
        .validate()
        .unwrap_err();
    assert!(
        matches!(
            &err,
            ContractError::AutomaticGroupCrossesPrivacyClass { group, member }
                if group == "auto" && member == "Direct"
        ),
        "{err}"
    );
}

#[test]
fn relayed_endpoint_rendered_without_detour_is_rejected() {
    let err = relayed_doc(&["Via"], None).validate().unwrap_err();
    assert!(
        matches!(&err, ContractError::RelayedEndpointNotChained { tag } if tag == "Via"),
        "{err}"
    );
}

#[test]
fn unknown_access_path_kind_round_trips() {
    let path = AccessPath::new(
        "future",
        AccessPathKind::Other("future-kind".into()),
        vec!["tcp".into()],
    );
    let doc = doc(None).with_access_paths(vec![path.clone()]);
    let back = ProvisioningDocument::from_json(&doc.to_json().unwrap()).unwrap();
    assert_eq!(back.access_paths, vec![path]);
}
