use compat_config::contract::{
    provisioning_document_with_mode_and_access_paths,
    provisioning_document_with_mode_and_access_paths_and_options, DiagnosticMode,
};
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, PeerCredential};
use compat_config::render::{standard_endpoints, CompatibilityMode, SelectionProfile};
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
        amneziawg: None,
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

// ----------------------------------------------------------------------
// D3 (real two-VPS acceptance, 2026-09-13): a `urltest` group dials every
// member itself — at Core start and on every interval — whatever the user
// picked in the selector. A group holding "Germany · Direct" next to
// "Germany · via Russia" therefore made an idle Privacy+ client connect
// straight from its own IP to the exit it must stay hidden from. These
// tests walk the rendered routing graph rather than matching tag names.
// ----------------------------------------------------------------------

const SECOND_EXIT: &str = r#"
[[peer_endpoints]]
id = "nl1-direct"
tag = "Netherlands · Direct"
host = "nl1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:nl1"
path = "direct"

[[peer_endpoints]]
id = "nl1-via-ru1"
tag = "Netherlands · via Russia"
host = "nl1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:nl1"
path = "relay-ru1"
credential_ref = "nl1-direct"
"#;

fn served_config(deployment: &str, profile: SelectionProfile) -> serde_json::Value {
    let cfg: DeploymentConfig = toml::from_str(deployment).unwrap();
    cfg.validate().unwrap();
    let endpoints = cfg
        .served_endpoints(
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
            "11223344",
            None,
        )
        .unwrap();
    let mut u = user();
    u.peer_credentials.insert(
        "nl1-direct".to_string(),
        PeerCredential::VlessReality {
            uuid: "33333333-3333-4333-8333-333333333333".to_string(),
        },
    );
    let doc = provisioning_document_with_mode_and_access_paths_and_options(
        &u,
        &endpoints,
        DiagnosticMode::None,
        &cfg.contract_access_paths().unwrap(),
        profile,
        CompatibilityMode::Normal,
    )
    .unwrap();
    doc.validate().unwrap();
    doc.singbox_config.unwrap()
}

fn outbound<'a>(config: &'a serde_json::Value, tag: &str) -> &'a serde_json::Value {
    config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["tag"] == tag)
        .unwrap_or_else(|| panic!("no outbound tagged {tag:?}"))
}

fn outbounds_of_type<'a>(config: &'a serde_json::Value, kind: &str) -> Vec<&'a serde_json::Value> {
    config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["type"] == kind)
        .collect()
}

fn members(group: &serde_json::Value) -> Vec<&str> {
    group["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect()
}

/// The servers the client device itself may open a connection to when
/// Core uses `tag`: an outbound's own server when it has no `detour`,
/// otherwise whatever its detour chain dials first; groups resolve to
/// every member they may use.
fn first_connections(config: &serde_json::Value, tag: &str) -> Vec<String> {
    let ob = outbound(config, tag);
    match ob["type"].as_str().unwrap() {
        "urltest" | "selector" => {
            let mut servers: Vec<String> = members(ob)
                .into_iter()
                .flat_map(|member| first_connections(config, member))
                .collect();
            servers.sort();
            servers.dedup();
            servers
        }
        _ => match ob["detour"].as_str() {
            Some(detour) => first_connections(config, detour),
            None => vec![ob["server"].as_str().unwrap_or("local").to_string()],
        },
    }
}

#[test]
fn d3_no_health_probe_group_in_a_privacy_profile_dials_an_exit_directly() {
    for deployment in [DEPLOYMENT.to_string(), format!("{DEPLOYMENT}{SECOND_EXIT}")] {
        for profile in [
            SelectionProfile::Reliability,
            SelectionProfile::Performance,
            SelectionProfile::Auto,
        ] {
            let config = served_config(&deployment, profile);
            let urltests = outbounds_of_type(&config, "urltest");
            assert!(
                !urltests.is_empty(),
                "an automatic Privacy+ group is still offered"
            );
            for group in urltests {
                assert_eq!(
                    first_connections(&config, group["tag"].as_str().unwrap()),
                    vec!["ru1.example.com".to_string()],
                    "{profile:?}: every probe of {:?} must enter through the relay, never an exit",
                    group["tag"]
                );
            }
        }
    }
}

#[test]
fn d3_direct_and_privacy_routes_never_share_an_automatic_group() {
    let config = served_config(
        &format!("{DEPLOYMENT}{SECOND_EXIT}"),
        SelectionProfile::Auto,
    );
    for group in outbounds_of_type(&config, "urltest") {
        for member in members(group) {
            assert!(
                outbound(&config, member)["detour"].is_string(),
                "automatic group {:?} contains the direct route {member:?}",
                group["tag"]
            );
        }
    }
}

#[test]
fn d3_privacy_plus_is_the_default_and_the_selector_never_switches_by_itself() {
    for profile in [SelectionProfile::Reliability, SelectionProfile::Performance] {
        let config = served_config(DEPLOYMENT, profile);
        let selector = outbound(&config, "select");
        assert_eq!(selector["type"], "selector", "a manual choice, not a race");
        assert_eq!(
            selector["default"], "Germany · via Russia",
            "{profile:?}: a freshly imported relay profile starts on Privacy+"
        );
        assert_eq!(config["route"]["final"], "select");
    }
}

#[test]
fn d3_explicit_privacy_plus_has_no_path_that_avoids_the_relay() {
    let config = served_config(DEPLOYMENT, SelectionProfile::default());
    let via = outbound(&config, "Germany · via Russia");
    assert_eq!(via["server"], "de1.example.com");
    let first_hop = outbound(&config, via["detour"].as_str().unwrap());
    assert_eq!(first_hop["server"], "ru1.example.com");
    assert!(
        first_hop.get("detour").is_none(),
        "the first hop dials the relay itself"
    );
    assert_eq!(
        first_connections(&config, "Germany · via Russia"),
        vec!["ru1.example.com".to_string()]
    );
    let first_hop_tag = first_hop["tag"].as_str().unwrap();
    for group in outbounds_of_type(&config, "urltest")
        .into_iter()
        .chain(outbounds_of_type(&config, "selector"))
    {
        assert!(
            !members(group).contains(&first_hop_tag),
            "the relay is never a route of its own, so nothing can pick RU as the exit"
        );
        if group["type"] == "urltest" && members(group).contains(&"Germany · via Russia") {
            assert!(
                !members(group).contains(&"Germany · Direct"),
                "a group a Privacy+ choice resolves through can never fall back to Direct"
            );
        }
    }
}

#[test]
fn d3_direct_stays_explicitly_selectable_and_independent_of_the_relay() {
    let config = served_config(DEPLOYMENT, SelectionProfile::default());
    assert!(members(outbound(&config, "select")).contains(&"Germany · Direct"));
    assert_eq!(
        first_connections(&config, "Germany · Direct"),
        vec!["de1.example.com".to_string()]
    );
}

#[test]
fn d3_automatic_choice_between_privacy_routes_exercises_complete_two_hop_routes() {
    let config = served_config(
        &format!("{DEPLOYMENT}{SECOND_EXIT}"),
        SelectionProfile::Auto,
    );
    let mut privacy = members(outbound(&config, "auto"));
    privacy.sort();
    assert_eq!(
        privacy,
        vec!["Germany · via Russia", "Netherlands · via Russia"]
    );
    for member in privacy {
        assert_eq!(
            first_connections(&config, member),
            vec!["ru1.example.com".to_string()]
        );
        assert_eq!(outbound(&config, member)["network"], "tcp");
    }
    assert_eq!(outbound(&config, "select")["default"], "auto");
}

#[test]
fn profiles_without_relay_routes_keep_the_single_exit_auto_group() {
    let endpoints = standard_endpoints(
        "de1.example.com",
        443,
        443,
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
        "11223344",
        "www.cloudflare.com",
        None,
    );
    let doc = provisioning_document_with_mode_and_access_paths(
        &user(),
        &endpoints,
        DiagnosticMode::None,
        &[],
    )
    .unwrap();
    let config = doc.singbox_config.unwrap();
    assert_eq!(
        members(outbound(&config, "auto")),
        vec!["Reality", "Hysteria2"]
    );
    assert_eq!(outbound(&config, "select")["default"], "Reality");
}
