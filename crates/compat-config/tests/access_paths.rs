use compat_config::deployment::DeploymentConfig;
use provisioning_contract::{AccessPathKind, PathType};

const BASE: &str = r#"
public_host = "vpn.example.com"
subscription_host = "vpn.example.com"

[reality]
listen_port = 443
handshake_server = "www.example-decoy.com"
handshake_port = 443

[hysteria2]
listen_port = 443

[subscription]
listen_port = 8080
public_port = 443
"#;

fn load(text: &str) -> Result<DeploymentConfig, compat_config::CompatError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    std::fs::write(&path, text).unwrap();
    DeploymentConfig::load(&path)
}

fn relay() -> &'static str {
    r#"
[[access_paths]]
id = "relay-r1"
kind = "relay"
failure_domain = "relay:r1"
region = "test-region"
provider = "operator"
capabilities = ["tcp"]
"#
}

fn peer(path: &str) -> String {
    format!(
        r#"
[[peer_endpoints]]
id = "eu2-reality"
tag = "Europe 2"
host = "vpn2.example.net"
port = 8443
transport = "vless_reality"
server_name = "www.example-decoy-two.org"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "9f8e7d6c"
failure_domain = "eu2"
path = "{path}"
"#
    )
}

#[test]
fn absent_access_paths_keep_existing_deployment_compatible() {
    let cfg = load(BASE).unwrap();
    assert!(cfg.access_paths.is_empty());
}

#[test]
fn relay_metadata_maps_to_non_secret_contract_type() {
    let cfg = load(&format!("{BASE}{}", relay())).unwrap();
    let path = cfg.access_paths[0].to_contract_access_path().unwrap();
    assert_eq!(path.id, "relay-r1");
    assert_eq!(path.kind, AccessPathKind::Relay);
    assert_eq!(path.failure_domain.as_deref(), Some("relay:r1"));
    assert_eq!(path.capabilities, vec!["tcp"]);
}

#[test]
fn credential_shaped_access_path_keys_fail_closed() {
    let text = format!("{BASE}{}password = \"must-never-live-here\"\n", relay());
    let err = load(&text).unwrap_err();
    assert!(format!("{err}").to_ascii_lowercase().contains("credential"));
}

#[test]
fn duplicate_path_ids_are_rejected() {
    assert!(load(&format!("{BASE}{}{}", relay(), relay())).is_err());
}

#[test]
fn opted_in_path_set_rejects_unknown_peer_path() {
    let err = load(&format!("{BASE}{}{}", relay(), peer("relay-missing"))).unwrap_err();
    assert!(format!("{err}").contains("no matching [[access_paths]]"));
}

#[test]
fn legacy_opaque_peer_path_without_access_paths_still_loads() {
    let cfg = load(&format!("{BASE}{}", peer("future-path"))).unwrap();
    let ep = cfg.peer_endpoints[0].to_compat_endpoint().unwrap();
    assert_eq!(ep.path.as_deref(), Some("future-path"));
    assert_eq!(
        PathType::from_wire(ep.path.as_deref().unwrap()),
        PathType::Other("future-path".into())
    );
}
