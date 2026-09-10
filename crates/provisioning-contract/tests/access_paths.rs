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
