//! Runtime declarative peer endpoints (ADR-0009, Option A) and the
//! per-user credentials that make them usable.
//!
//! The properties that matter here are mostly *negative*: a deployment
//! that configures no peers must be indistinguishable from today, a peer
//! the user cannot authenticate to must be absent rather than broken, and
//! this server must refuse to hold a peer's private key.

use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatTransport, CompatUser, EndpointOrigin, PeerCredential};
use compat_config::secret::SecretString;
use compat_config::store;

fn write(dir: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

const BASE_TOML: &str = r#"
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

fn peer_block() -> String {
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
region = "nl"
provider = "provider-b"
"#
    .to_string()
}

// ---------------------------------------------------------------------
// Zero-peer deployments are untouched
// ---------------------------------------------------------------------

#[test]
fn a_deployment_with_no_peer_endpoints_loads_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "deployment.toml", BASE_TOML);
    let cfg = DeploymentConfig::load(&p).expect("existing files must keep loading");
    assert!(
        cfg.peer_endpoints.is_empty(),
        "absence of the section must read as no peers, never as an error"
    );
}

#[test]
fn a_user_with_no_peer_credentials_round_trips_byte_identically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("users.json");
    let user = CompatUser {
        id: "u1".into(),
        name: "alice".into(),
        enabled: true,
        vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
        hysteria2_password: SecretString::new("hy2pass"),
        subscription_token_hash_hex: "hash".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        peer_credentials: Default::default(),
    };
    store::save_users_atomic(&path, &[user]).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("peer_credentials"),
        "an empty map must not appear on disk; got {text}"
    );
    assert_eq!(
        store::load_users(&path).unwrap().len(),
        1,
        "and it must still load"
    );
}

// ---------------------------------------------------------------------
// Peer config validation
// ---------------------------------------------------------------------

#[test]
fn a_peer_id_colliding_with_a_local_endpoint_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        "{BASE_TOML}{}",
        peer_block().replace("eu2-reality", "reality-1")
    );
    let p = write(dir.path(), "deployment.toml", &toml);
    let err = DeploymentConfig::load(&p).expect_err("id collision with a locally generated id");
    assert!(
        format!("{err}").contains("reality-1"),
        "the error must name the colliding id; got {err}"
    );
}

#[test]
fn two_peers_sharing_an_id_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let toml = format!("{BASE_TOML}{}{}", peer_block(), peer_block());
    let p = write(dir.path(), "deployment.toml", &toml);
    assert!(DeploymentConfig::load(&p).is_err());
}

#[test]
fn a_peer_private_key_is_refused_rather_than_silently_ignored() {
    // This server has no use for a peer's private key and must never
    // hold one. Silently ignoring the field would be worse than failing:
    // the operator would believe it was needed, and it would sit in a
    // config file forever.
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        "{BASE_TOML}{}reality_private_key = \"deadbeefdeadbeef\"\n",
        peer_block()
    );
    let p = write(dir.path(), "deployment.toml", &toml);
    let err = DeploymentConfig::load(&p).expect_err("a private key must be refused");
    let msg = format!("{err}").to_ascii_lowercase();
    assert!(msg.contains("private"), "the error must say why; got {err}");
}

#[test]
fn a_malformed_reality_public_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        "{BASE_TOML}{}",
        peer_block().replace(
            "reality_public_key = \"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8\"",
            "reality_public_key = \"\""
        )
    );
    let p = write(dir.path(), "deployment.toml", &toml);
    assert!(DeploymentConfig::load(&p).is_err());
}

#[test]
fn a_peer_endpoint_becomes_a_compat_endpoint_marked_as_a_peer() {
    let dir = tempfile::tempdir().unwrap();
    let toml = format!("{BASE_TOML}{}", peer_block());
    let p = write(dir.path(), "deployment.toml", &toml);
    let cfg = DeploymentConfig::load(&p).unwrap();
    let ep = cfg.peer_endpoints[0].to_compat_endpoint().unwrap();
    assert_eq!(ep.origin, EndpointOrigin::Peer);
    assert_eq!(ep.id, "eu2-reality");
    assert_eq!(ep.host, "vpn2.example.net");
    assert_eq!(ep.port, 8443);
    assert_eq!(ep.transport, CompatTransport::VlessReality);
    assert_eq!(ep.failure_domain.as_deref(), Some("eu2"));
    assert_eq!(ep.provider.as_deref(), Some("provider-b"));
}

// ---------------------------------------------------------------------
// Credential secrecy
// ---------------------------------------------------------------------

#[test]
fn debug_of_a_peer_credential_never_prints_the_secret() {
    let c = PeerCredential::Hysteria2 {
        password: SecretString::new("super-secret-peer-password"),
    };
    let rendered = format!("{c:?}");
    assert!(
        !rendered.contains("super-secret-peer-password"),
        "got {rendered}"
    );
}

#[test]
fn peer_credentials_round_trip_and_are_looked_up_by_endpoint_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("users.json");
    let mut user = CompatUser {
        id: "u1".into(),
        name: "alice".into(),
        enabled: true,
        vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
        hysteria2_password: SecretString::new("hy2pass"),
        subscription_token_hash_hex: "hash".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        peer_credentials: Default::default(),
    };
    user.peer_credentials.insert(
        "eu2-reality".into(),
        PeerCredential::VlessReality {
            uuid: "22222222-2222-4222-8222-222222222222".into(),
        },
    );
    store::save_users_atomic(&path, &[user]).unwrap();
    let back = store::load_users(&path).unwrap();
    match back[0].peer_credential("eu2-reality") {
        Some(PeerCredential::VlessReality { uuid }) => {
            assert_eq!(uuid, "22222222-2222-4222-8222-222222222222")
        }
        other => panic!("expected a vless peer credential, got {other:?}"),
    }
    assert!(
        back[0].peer_credential("does-not-exist").is_none(),
        "an absent credential must be None, never a fallback to the local one"
    );
}
