use compat_config::contract::{
    provisioning_document_with_mode_and_access_paths_and_options, DiagnosticMode,
};
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, PeerCredential};
use compat_config::render::{standard_endpoints, CompatibilityMode, SelectionProfile};
use compat_config::secret::SecretString;
use provisioning_contract::AccessPath;

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

fn render(mode: CompatibilityMode) -> serde_json::Value {
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

    provisioning_document_with_mode_and_access_paths_and_options(
        &user(),
        &endpoints,
        DiagnosticMode::None,
        &paths,
        SelectionProfile::default(),
        mode,
    )
    .unwrap()
    .singbox_config
    .unwrap()
}

#[test]
fn hiddify_pinned_relay_keeps_xudp_available_to_application_udp() {
    let config = render(CompatibilityMode::HiddifyPinned);
    let exit = config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outbound| outbound["tag"] == "Germany · via Russia")
        .expect("pinned relayed exit");

    assert_eq!(exit["type"], "vless");
    assert!(exit["detour"].is_string(), "relay detour must remain intact");
    assert!(
        exit.get("network").is_none(),
        "network=tcp filters application UDP before VLESS can encode it as XUDP"
    );
    assert_eq!(exit["flow"], "xtls-rprx-vision");
}

#[test]
fn normal_relay_profile_is_unchanged_by_the_hiddify_specific_fix() {
    let config = render(CompatibilityMode::Normal);
    let exit = config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outbound| outbound["tag"] == "Germany · via Russia")
        .expect("normal relayed exit");

    assert_eq!(exit["network"], "tcp");
}
