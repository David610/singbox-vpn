use provisioning_contract::{
    Capability, ContractError, Endpoint, ProvisioningDocument, RealityParams, ServerInfo,
    TransportParams, VLESS_FLOW_VISION,
};

fn valid_document() -> ProvisioningDocument {
    let endpoint = Endpoint {
        id: "reality-primary".to_string(),
        tag: "VPN (REALITY)".to_string(),
        host: "vpn.example.com".to_string(),
        port: 443,
        server_name: "www.example.com".to_string(),
        params: TransportParams::VlessReality {
            uuid: "11111111-1111-4111-8111-111111111111".to_string(),
            flow: Some(VLESS_FLOW_VISION.to_string()),
            reality: RealityParams {
                public_key: "test-public-key".to_string(),
                short_id: "0a1b2c3d".to_string(),
                fingerprint: "chrome".to_string(),
            },
        },
    };

    ProvisioningDocument::new(
        ServerInfo::current("0.1.2"),
        vec![Capability::VlessReality],
        vec![endpoint],
    )
}

#[test]
fn every_strict_prefix_of_a_valid_document_is_rejected() {
    let json = valid_document().to_json().unwrap();
    assert!(ProvisioningDocument::from_json(&json).is_ok());

    for cut in 0..json.len() {
        let prefix = &json[..cut];
        assert!(
            ProvisioningDocument::from_json(prefix).is_err(),
            "truncated provisioning JSON unexpectedly parsed at byte offset {cut}/{}",
            json.len()
        );
    }
}

#[test]
fn a_future_schema_version_is_rejected_at_the_parse_boundary() {
    let mut value = serde_json::to_value(valid_document()).unwrap();
    value["schema_version"] = serde_json::json!(999);
    let encoded = serde_json::to_string(&value).unwrap();

    let err = ProvisioningDocument::from_json(&encoded).unwrap_err();
    assert!(matches!(
        err,
        ContractError::UnsupportedSchemaVersion {
            requested: 999,
            ..
        }
    ));
}

#[test]
fn forbidden_server_private_or_client_policy_markers_fail_closed() {
    let mut path_leak = valid_document();
    path_leak.server.version = "/etc/vpn/private-state".to_string();
    assert!(matches!(
        path_leak.validate(),
        Err(ContractError::ForbiddenContent { .. })
    ));

    let mut pem_leak = valid_document();
    if let TransportParams::VlessReality { reality, .. } = &mut pem_leak.endpoints[0].params {
        reality.public_key = "-----BEGIN PRIVATE KEY-----".to_string();
    }
    assert!(matches!(
        pem_leak.validate(),
        Err(ContractError::ForbiddenContent { .. })
    ));
}

#[test]
fn invalid_endpoint_host_and_zero_port_are_rejected_before_serialization() {
    let mut slash_host = valid_document();
    slash_host.endpoints[0].host = "vpn.example.com/path".to_string();
    assert!(matches!(
        slash_host.validate(),
        Err(ContractError::Invalid {
            field: "endpoint.host",
            ..
        })
    ));

    let mut zero_port = valid_document();
    zero_port.endpoints[0].port = 0;
    assert!(matches!(
        zero_port.validate(),
        Err(ContractError::Invalid {
            field: "endpoint.port",
            ..
        })
    ));
}
