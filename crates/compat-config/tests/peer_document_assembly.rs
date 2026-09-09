//! Assembling a provisioning document that contains peer endpoints.
//!
//! The load-bearing behaviours: a peer the user has no credential for is
//! ABSENT rather than broken, two users get genuinely independent peer
//! credentials, a credential is never coerced across transports, and a
//! deployment with no peers is untouched.

use compat_config::contract::{
    provisioning_document, provisioning_document_with_mode, DiagnosticMode,
};
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatEndpoint, CompatUser, PeerCredential};
use compat_config::render::standard_endpoints;
use compat_config::secret::SecretString;
use provisioning_contract as contract;

/// A 32-byte X25519 public key in base64url-without-padding. Test-only
/// material: it is a fixed byte range, not a real key.
const FAKE_PEER_PUBLIC_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

const DEPLOYMENT_WITH_PEER: &str = r#"
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
"#;

fn peer_endpoint() -> CompatEndpoint {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("deployment.toml");
    std::fs::write(&p, DEPLOYMENT_WITH_PEER).unwrap();
    DeploymentConfig::load(&p).unwrap().peer_endpoints[0]
        .to_compat_endpoint()
        .unwrap()
}

fn local_endpoints() -> Vec<CompatEndpoint> {
    standard_endpoints(
        "vpn.example.com",
        443,
        443,
        FAKE_PEER_PUBLIC_KEY,
        "0a1b2c3d",
        "www.example-decoy.com",
        None,
    )
}

fn user(id: &str, uuid: &str) -> CompatUser {
    CompatUser {
        id: id.into(),
        name: id.into(),
        enabled: true,
        vless_uuid: uuid.into(),
        hysteria2_password: SecretString::new("local-hy2-password"),
        subscription_token_hash_hex: "hash".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        peer_credentials: Default::default(),
    }
}

fn with_peer_uuid(mut u: CompatUser, uuid: &str) -> CompatUser {
    u.peer_credentials.insert(
        "eu2-reality".into(),
        PeerCredential::VlessReality { uuid: uuid.into() },
    );
    u
}

fn endpoints_with_peer() -> Vec<CompatEndpoint> {
    let mut e = local_endpoints();
    e.push(peer_endpoint());
    e
}

fn peer_uuid_in(doc: &contract::ProvisioningDocument) -> String {
    let peer = doc
        .endpoints
        .iter()
        .find(|e| e.id == "eu2-reality")
        .expect("peer endpoint present");
    match &peer.params {
        contract::TransportParams::VlessReality { uuid, .. } => uuid.clone(),
        other => panic!("unexpected params {other:?}"),
    }
}

#[test]
fn a_peer_is_omitted_for_a_user_with_no_credential_for_it() {
    let doc = provisioning_document(
        &user("u1", "11111111-1111-4111-8111-111111111111"),
        &endpoints_with_peer(),
    )
    .expect("a user without a peer credential is normal, not an error");
    assert!(
        !doc.endpoints.iter().any(|e| e.id == "eu2-reality"),
        "an endpoint the user cannot authenticate to must be absent, not served broken"
    );
    assert_eq!(doc.endpoints.len(), 2, "the two local endpoints remain");
}

#[test]
fn a_peer_appears_once_the_operator_sets_that_users_credential() {
    let u = with_peer_uuid(
        user("u1", "11111111-1111-4111-8111-111111111111"),
        "22222222-2222-4222-8222-222222222222",
    );
    let doc = provisioning_document(&u, &endpoints_with_peer()).unwrap();
    let peer = doc
        .endpoints
        .iter()
        .find(|e| e.id == "eu2-reality")
        .unwrap();
    assert_eq!(peer.host, "vpn2.example.net");
    assert_eq!(peer.port, 8443);
    assert_eq!(peer.tag, "Europe 2");
    assert_eq!(peer.failure_domain.as_deref(), Some("eu2"));
    assert_eq!(peer.provider.as_deref(), Some("provider-b"));
    assert_eq!(peer.path, Some(contract::PathType::Direct));
    assert_eq!(
        peer_uuid_in(&doc),
        "22222222-2222-4222-8222-222222222222",
        "the peer must carry the peer credential, never this server's local one"
    );
}

#[test]
fn two_users_receive_independent_peer_credentials() {
    let alice = with_peer_uuid(
        user("alice", "11111111-1111-4111-8111-111111111111"),
        "22222222-2222-4222-8222-222222222222",
    );
    let bob = with_peer_uuid(
        user("bob", "33333333-3333-4333-8333-333333333333"),
        "44444444-4444-4444-8444-444444444444",
    );
    let eps = endpoints_with_peer();
    let a = peer_uuid_in(&provisioning_document(&alice, &eps).unwrap());
    let b = peer_uuid_in(&provisioning_document(&bob, &eps).unwrap());
    assert_ne!(
        a, b,
        "no shared team credential: per-user revocation depends on these differing"
    );
}

#[test]
fn a_local_credential_is_never_used_as_a_fallback_on_a_peer() {
    let local_uuid = "11111111-1111-4111-8111-111111111111";
    let u = with_peer_uuid(
        user("u1", local_uuid),
        "22222222-2222-4222-8222-222222222222",
    );
    let doc = provisioning_document(&u, &endpoints_with_peer()).unwrap();
    assert_ne!(
        peer_uuid_in(&doc),
        local_uuid,
        "this server's UUID is meaningless on a server it does not control"
    );
}

#[test]
fn a_credential_is_never_coerced_across_transports() {
    let mut u = user("carol", "11111111-1111-4111-8111-111111111111");
    u.peer_credentials.insert(
        "eu2-reality".into(),
        PeerCredential::Hysteria2 {
            password: SecretString::new("wrong-shape"),
        },
    );
    let err = provisioning_document(&u, &endpoints_with_peer())
        .expect_err("an operator error must surface, not be reshaped into something dialable");
    let msg = format!("{err}");
    assert!(
        msg.contains("carol") && msg.contains("eu2-reality"),
        "the error must name the user and endpoint so it is actionable; got {msg}"
    );
}

#[test]
fn a_document_with_a_peer_embeds_a_config_that_can_actually_select_it() {
    // The whole failover design rests on the client being able to hand
    // the peer's tag to Core's selector.
    let u = with_peer_uuid(
        user("u1", "11111111-1111-4111-8111-111111111111"),
        "22222222-2222-4222-8222-222222222222",
    );
    let doc = provisioning_document(&u, &endpoints_with_peer()).unwrap();
    let config = doc.singbox_config.as_ref().expect("config embedded");
    let selector = config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["tag"] == contract::SELECTOR_GROUP_TAG)
        .expect("selector group");
    let options: Vec<&str> = selector["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        options.contains(&"Europe 2"),
        "the peer must be selectable inside the running Core; got {options:?}"
    );
    assert_eq!(config["route"]["final"], contract::SELECTOR_GROUP_TAG);
}

#[test]
fn the_peers_own_outbound_carries_the_peers_host_and_key_not_this_servers() {
    let u = with_peer_uuid(
        user("u1", "11111111-1111-4111-8111-111111111111"),
        "22222222-2222-4222-8222-222222222222",
    );
    let doc = provisioning_document(&u, &endpoints_with_peer()).unwrap();
    let config = doc.singbox_config.as_ref().unwrap();
    let peer_ob = config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["tag"] == "Europe 2")
        .expect("peer outbound");
    assert_eq!(peer_ob["server"], "vpn2.example.net");
    assert_eq!(peer_ob["server_port"], 8443);
    assert_eq!(peer_ob["uuid"], "22222222-2222-4222-8222-222222222222");
}

#[test]
fn a_local_only_document_is_unchanged_by_the_peer_feature_existing() {
    // No operator is forced into multi-VPS mode.
    let doc = provisioning_document(
        &user("u1", "11111111-1111-4111-8111-111111111111"),
        &local_endpoints(),
    )
    .unwrap();
    assert_eq!(doc.endpoints.len(), 2);
    for ep in &doc.endpoints {
        assert!(
            ep.failure_domain.is_none(),
            "a local endpoint declares no domain; the client derives it from the host"
        );
        assert!(ep.region.is_none() && ep.provider.is_none() && ep.asn.is_none());
        assert!(ep.path.is_none());
    }
    let mut envelope: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
    envelope.as_object_mut().unwrap().remove("singbox_config");
    let text = envelope.to_string();
    for absent in ["failure_domain", "region", "provider", "asn", "origin"] {
        assert!(
            !text.contains(absent),
            "{absent} leaked into a local-only document: {text}"
        );
    }
}

#[test]
fn diagnostic_modes_still_behave_with_a_peer_present() {
    let u = with_peer_uuid(
        user("u1", "11111111-1111-4111-8111-111111111111"),
        "22222222-2222-4222-8222-222222222222",
    );
    let doc = provisioning_document_with_mode(&u, &endpoints_with_peer(), DiagnosticMode::TcpOnly)
        .unwrap();
    assert!(
        !doc.endpoints
            .iter()
            .any(|e| e.transport() == contract::Transport::Hysteria2),
        "tcp-only must still drop every UDP-carrying endpoint"
    );
    assert!(
        doc.endpoints.iter().any(|e| e.id == "eu2-reality"),
        "a REALITY peer survives tcp-only, like any other REALITY endpoint"
    );
}

#[test]
fn a_peer_and_a_local_endpoint_share_no_credential_material() {
    let u = with_peer_uuid(
        user("u1", "11111111-1111-4111-8111-111111111111"),
        "22222222-2222-4222-8222-222222222222",
    );
    let doc = provisioning_document(&u, &endpoints_with_peer()).unwrap();
    let local = doc.endpoints.iter().find(|e| e.id == "reality-1").unwrap();
    let peer = doc
        .endpoints
        .iter()
        .find(|e| e.id == "eu2-reality")
        .unwrap();
    let uuid_of = |e: &contract::Endpoint| match &e.params {
        contract::TransportParams::VlessReality { uuid, .. } => uuid.clone(),
        other => panic!("unexpected {other:?}"),
    };
    assert_ne!(
        uuid_of(local),
        uuid_of(peer),
        "compromising one endpoint must not expose the credential for the other"
    );
}
