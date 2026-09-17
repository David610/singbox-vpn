use compat_config::contract::provisioning_document_with_mode_and_access_paths;
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, PeerCredential};
use compat_config::render::standard_endpoints;
use compat_config::secret::SecretString;
use provisioning_contract::{AccessPath, AccessPathKind};

const DEPLOYMENT: &str = r#"
schema_version = 2
node_id = "ru1"
role = "relay"
public_host = "ru1.example.com"
subscription_host = "ru1.example.com"

[reality]
listen_port = 443
handshake_server = "www.cloudflare.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100

[[access_paths]]
id = "relay-ru1"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]

[[peer_endpoints]]
id = "de1-direct"
tag = "Germany · Direct"
host = "de1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "direct"

[[peer_endpoints]]
id = "de1-via-ru1"
tag = "Germany · via Russia"
host = "de1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "relay-ru1"
credential_ref = "de1-direct"
"#;

fn user() -> CompatUser {
    let mut peer_credentials = std::collections::BTreeMap::new();
    peer_credentials.insert(
        "de1-direct".to_string(),
        PeerCredential::VlessReality {
            uuid: "22222222-2222-4222-8222-222222222222".to_string(),
        },
    );
    CompatUser {
        id: "u1".into(),
        name: "test".into(),
        enabled: true,
        vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
        hysteria2_password: SecretString::new("hy2"),
        subscription_token_hash_hex: "hash".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        peer_credentials,
    }
}

#[test]
fn relay_path_is_real_detour_and_first_hop_is_not_selectable() {
    let cfg: DeploymentConfig = toml::from_str(DEPLOYMENT).unwrap();
    cfg.validate().unwrap();
    let mut endpoints = standard_endpoints(
        &cfg.public_host,
        443,
        443,
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
        "11223344",
        "www.cloudflare.com",
        None,
    );
    endpoints.retain(|endpoint| endpoint.id == "reality-1");
    endpoints.extend(
        cfg.peer_endpoints
            .iter()
            .map(|peer| peer.to_compat_endpoint().unwrap()),
    );
    let paths: Vec<AccessPath> = cfg
        .access_paths
        .iter()
        .map(|path| path.to_contract_access_path().unwrap())
        .collect();

    let doc = provisioning_document_with_mode_and_access_paths(
        &user(),
        &endpoints,
        compat_config::contract::DiagnosticMode::None,
        &paths,
    )
    .unwrap();

    let ids: Vec<_> = doc
        .endpoints
        .iter()
        .map(|endpoint| endpoint.id.as_str())
        .collect();
    assert_eq!(ids, vec!["de1-direct", "de1-via-ru1"]);
    let config = doc.singbox_config.as_ref().unwrap();
    let outbounds = config["outbounds"].as_array().unwrap();
    let relay = outbounds
        .iter()
        .find(|outbound| outbound["server"] == "ru1.example.com")
        .expect("hidden RU first-hop outbound");
    let via = outbounds
        .iter()
        .find(|outbound| outbound["tag"] == "Germany · via Russia")
        .expect("via-RU exit outbound");
    assert_eq!(via["detour"], relay["tag"]);
    assert_eq!(via["network"], "tcp");
    assert_eq!(via["uuid"], "22222222-2222-4222-8222-222222222222");

    let selector = outbounds
        .iter()
        .find(|outbound| outbound["tag"] == "select")
        .unwrap();
    let options: Vec<_> = selector["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(options.contains(&"Germany · Direct"));
    assert!(options.contains(&"Germany · via Russia"));
    assert!(!options.contains(&relay["tag"].as_str().unwrap()));
}

#[test]
fn relay_path_without_via_endpoint_fails_closed() {
    let endpoints = standard_endpoints(
        "ru1.example.com",
        443,
        443,
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
        "11223344",
        "www.cloudflare.com",
        None,
    );
    let path = AccessPath::new("relay-broken", AccessPathKind::Relay, vec!["tcp".into()]);
    let mut exit = endpoints[0].clone();
    exit.id = "exit".into();
    exit.label = "Exit via broken relay".into();
    exit.path = Some("relay-broken".into());
    let err = provisioning_document_with_mode_and_access_paths(
        &user(),
        &[endpoints[0].clone(), exit],
        compat_config::contract::DiagnosticMode::None,
        &[path],
    )
    .unwrap_err();
    assert!(err.to_string().contains("without via_endpoint_id"));
}
