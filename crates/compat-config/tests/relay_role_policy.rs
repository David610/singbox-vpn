//! Relay role: deployment validation, fail-closed server policy, and the
//! first-party provisioning path, exercised through the same public
//! functions `vpn-admin` and `vpn-subscription` call in production.
//!
//! Level 1/2 evidence (CODE-VERIFIED / CI-VERIFIED). The loopback system
//! tests that drive real sing-box processes through these documents live
//! in `two_hop_system.rs`.

mod common;

use compat_config::contract::{
    infrastructure_endpoint_ids, provisioning_document_with_mode_and_access_paths, DiagnosticMode,
};
use compat_config::deployment::{
    default_node_id_for_host, migrate_deployment_toml, migrate_deployment_toml_text,
    validate_node_id, DeploymentConfig, DeploymentMigrationOutcome, NodeRole, RelayTarget,
};
use compat_config::model::{
    CompatUser, Hysteria2ServerParams, PeerCredential, RealityServerParams,
};
use compat_config::render::{render_uri_list, share_link_endpoints};
use compat_config::secret::SecretString;
use compat_config::server::{
    render_server_config_for_deployment, render_singbox_server_config, ServerPorts,
};
use compat_config::CompatError;

const EXIT_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
const RELAY_UUID: &str = "11111111-1111-4111-8111-111111111111";
const EXIT_UUID: &str = "22222222-2222-4222-8222-222222222222";

fn base(role: &str) -> String {
    format!(
        r#"schema_version = 2
node_id = "{node}"
role = "{role}"
public_host = "{node}.example.test"
subscription_host = "{node}.example.test"

[reality]
listen_port = 443
handshake_server = "www.example.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        node = if role == "relay" { "ru1" } else { "de1" }
    )
}

const RELAY_INGRESS: &str = r#"
[[access_paths]]
id = "via-ru1"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]
"#;

fn exit_peer(id: &str, tag: &str, host: &str, path: &str, credential_ref: Option<&str>) -> String {
    let credential_ref = credential_ref
        .map(|value| format!("credential_ref = \"{value}\"\n"))
        .unwrap_or_default();
    format!(
        r#"
[[peer_endpoints]]
id = "{id}"
tag = "{tag}"
host = "{host}"
port = 443
transport = "vless_reality"
server_name = "www.example.com"
reality_public_key = "{EXIT_KEY}"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "{path}"
{credential_ref}"#
    )
}

fn paired_relay_toml() -> String {
    format!(
        "{}{RELAY_INGRESS}{}{}",
        base("relay"),
        exit_peer(
            "de1-direct",
            "Germany · Direct",
            "de1.example.test",
            "direct",
            None
        ),
        exit_peer(
            "de1-via-ru1",
            "Germany · via Russia",
            "de1.example.test",
            "via-ru1",
            Some("de1-direct")
        ),
    )
}

fn load(text: &str) -> Result<DeploymentConfig, CompatError> {
    let cfg: DeploymentConfig =
        toml::from_str(text).map_err(|e| CompatError::Parse(e.to_string()))?;
    cfg.validate()?;
    Ok(cfg)
}

fn expect_err(text: &str, needle: &str) {
    match load(text) {
        Ok(_) => panic!("expected a validation failure containing {needle:?}"),
        Err(e) => assert!(
            e.to_string().contains(needle),
            "error {e} does not mention {needle:?}"
        ),
    }
}

fn user() -> CompatUser {
    let mut peer_credentials = std::collections::BTreeMap::new();
    peer_credentials.insert(
        "de1-direct".to_string(),
        PeerCredential::VlessReality {
            uuid: EXIT_UUID.to_string(),
        },
    );
    CompatUser {
        id: "u1".into(),
        name: "relay-user".into(),
        enabled: true,
        vless_uuid: RELAY_UUID.into(),
        hysteria2_password: SecretString::new("relay-hy2-password"),
        subscription_token_hash_hex: "unused".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        google_egress_hairpin: false,
        peer_credentials,
        is_reserved_probe: false,
        credentials: Vec::new(),
    }
}

fn reality() -> RealityServerParams {
    RealityServerParams {
        private_key_hex: SecretString::new("SYNTHETIC-RELAY-PRIVATE-KEY"),
        public_key_hex: EXIT_KEY.into(),
        short_ids: vec!["0a1b2c3d".into()],
        handshake_server: "www.example.com".into(),
        handshake_port: 443,
        google_egress_hairpin_uuid: None,
    }
}

fn hysteria() -> Hysteria2ServerParams {
    Hysteria2ServerParams {
        tls_cert_path: "/nonexistent/cert.pem".into(),
        tls_key_path: "/nonexistent/key.pem".into(),
        obfs_password: None,
        masquerade_dir_path: None,
        up_mbps: None,
        down_mbps: None,
    }
}

fn render(cfg: &DeploymentConfig, users: &[CompatUser]) -> serde_json::Value {
    render_server_config_for_deployment(cfg, users, &reality(), &hysteria(), 1_000).unwrap()
}

// ----------------------------------------------------------------------
// NodeRole / node identity
// ----------------------------------------------------------------------

#[test]
fn node_role_parser_is_strict() {
    assert_eq!(NodeRole::parse("exit").unwrap(), NodeRole::Exit);
    assert_eq!(NodeRole::parse("relay").unwrap(), NodeRole::Relay);
    for bad in ["", "Exit", "RELAY", "entry", "exit ", "both"] {
        assert!(NodeRole::parse(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn invalid_role_in_deployment_toml_fails_to_load() {
    let text = base("relay").replace("role = \"relay\"", "role = \"gateway\"");
    let err = load(&text).unwrap_err().to_string();
    assert!(err.contains("gateway"), "{err}");
}

#[test]
fn absent_role_means_exit_for_backward_compatibility() {
    let text = base("exit").replace("role = \"exit\"\n", "");
    assert_eq!(load(&text).unwrap().role, NodeRole::Exit);
}

#[test]
fn schema_v2_requires_a_valid_node_id() {
    expect_err(&base("exit").replace("node_id = \"de1\"\n", ""), "node_id");
    expect_err(
        &base("exit").replace("\"de1\"", "\"de 1\""),
        "unsupported characters",
    );
    expect_err(&base("exit").replace("\"de1\"", "\"-de1\""), "must start");
    assert!(validate_node_id(&"a".repeat(63)).is_ok());
    assert!(validate_node_id(&"a".repeat(64)).is_err());
}

#[test]
fn default_node_id_rule_matches_the_shared_installer_fixture_table() {
    let table = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/lib/tests/fixtures/node-id-cases.tsv"),
    )
    .unwrap();
    let mut cases = 0;
    for line in table
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let (host, expected) = line.split_once('\t').expect("host<TAB>expected");
        assert_eq!(default_node_id_for_host(host), expected, "host {host:?}");
        assert!(
            validate_node_id(expected).is_ok(),
            "{expected:?} must itself be valid"
        );
        cases += 1;
    }
    assert!(cases >= 10);
}

// ----------------------------------------------------------------------
// Relay declaration validation (fail closed)
// ----------------------------------------------------------------------

#[test]
fn relay_role_requires_a_local_ingress_declaration() {
    expect_err(&base("relay"), "requires an [[access_paths]] entry");
}

#[test]
fn exit_must_never_declare_its_own_listener_as_relay_infrastructure() {
    expect_err(
        &format!("{}{RELAY_INGRESS}", base("exit")),
        "requires role = \"relay\"",
    );
}

#[test]
fn unpaired_relay_loads_and_has_no_targets() {
    let cfg = load(&format!("{}{RELAY_INGRESS}", base("relay"))).unwrap();
    assert_eq!(cfg.role, NodeRole::Relay);
    assert!(cfg.relay_targets().is_empty());
}

#[test]
fn relay_targets_come_only_from_declared_relay_routes() {
    let cfg = load(&paired_relay_toml()).unwrap();
    assert_eq!(
        cfg.relay_targets(),
        vec![RelayTarget {
            host: "de1.example.test".into(),
            port: 443
        }]
    );
}

#[test]
fn an_exit_has_no_relay_targets_even_with_peers() {
    let text = format!(
        "{}{}",
        base("exit"),
        exit_peer(
            "de2-direct",
            "Germany 2",
            "de2.example.test",
            "direct",
            None
        )
    );
    assert!(load(&text).unwrap().relay_targets().is_empty());
}

#[test]
fn dangling_via_endpoint_is_refused_even_when_unused() {
    let text = format!(
        "{}{}",
        base("exit"),
        RELAY_INGRESS.replace("reality-1", "ghost-endpoint")
    );
    expect_err(
        &text,
        "neither local reality-1 nor a declared peer endpoint",
    );
}

#[test]
fn relay_route_without_via_endpoint_is_refused() {
    let text = format!(
        "{}{}{}",
        base("exit"),
        RELAY_INGRESS.replace("via_endpoint_id = \"reality-1\"\n", ""),
        exit_peer(
            "de1-via",
            "via nothing",
            "de1.example.test",
            "via-ru1",
            None
        )
    );
    expect_err(&text, "has no via_endpoint_id");
}

#[test]
fn udp_relay_capability_is_refused() {
    let text = format!(
        "{}{}",
        base("relay"),
        RELAY_INGRESS.replace("[\"tcp\"]", "[\"tcp\", \"udp\"]")
    );
    expect_err(&text, "capability \"udp\"");
}

#[test]
fn hysteria2_first_hop_is_refused() {
    let text = format!(
        "{}{}",
        base("relay"),
        RELAY_INGRESS.replace("reality-1", "hysteria2-1")
    );
    expect_err(&text, "local Hysteria2 listener");
}

#[test]
fn hysteria2_route_over_relay_is_refused() {
    let peer = r#"
[[peer_endpoints]]
id = "de1-hy2-via"
tag = "Germany hy2 via Russia"
host = "de1.example.test"
port = 443
transport = "hysteria2"
failure_domain = "exit:de1"
path = "via-ru1"
"#;
    expect_err(
        &format!("{}{RELAY_INGRESS}{peer}", base("relay")),
        "TCP/VLESS-only",
    );
}

#[test]
fn invalid_relay_destinations_are_refused() {
    for host in [
        "0.0.0.0",
        "::",
        "224.0.0.1",
        "255.255.255.255",
        "DE1.example.test",
        "de1..test",
    ] {
        let text = format!(
            "{}{RELAY_INGRESS}{}",
            base("relay"),
            exit_peer("de1-via", "Germany via", host, "via-ru1", None)
        );
        expect_err(&text, "invalid relay destination");
    }
}

#[test]
fn dangling_credential_ref_is_refused() {
    let text = format!(
        "{}{RELAY_INGRESS}{}",
        base("relay"),
        exit_peer(
            "de1-via",
            "Germany via",
            "de1.example.test",
            "via-ru1",
            Some("missing")
        )
    );
    expect_err(&text, "does not name a declared peer endpoint");
}

#[test]
fn credential_ref_to_a_different_server_is_refused() {
    let text = format!(
        "{}{RELAY_INGRESS}{}{}",
        base("relay"),
        exit_peer(
            "de1-direct",
            "Germany · Direct",
            "de1.example.test",
            "direct",
            None
        ),
        exit_peer(
            "nl1-via",
            "NL via",
            "nl1.example.test",
            "via-ru1",
            Some("de1-direct")
        )
    );
    expect_err(&text, "names a different server");
}

#[test]
fn credential_ref_transport_mismatch_is_refused() {
    let hy2 = r#"
[[peer_endpoints]]
id = "de1-hy2"
tag = "Germany hy2"
host = "de1.example.test"
port = 443
transport = "hysteria2"
failure_domain = "exit:de1"
"#;
    let text = format!(
        "{}{RELAY_INGRESS}{hy2}{}",
        base("relay"),
        exit_peer(
            "de1-via",
            "Germany via",
            "de1.example.test",
            "via-ru1",
            Some("de1-hy2")
        )
    );
    expect_err(&text, "credentials cannot cross transports");
}

#[test]
fn duplicate_endpoint_ids_and_tags_are_refused() {
    let dup_id = format!(
        "{}{}{}",
        base("exit"),
        exit_peer("de2", "A", "de2.example.test", "direct", None),
        exit_peer("de2", "B", "de2.example.test", "direct", None)
    );
    expect_err(&dup_id, "duplicate [[peer_endpoints]] id");
    let dup_tag = format!(
        "{}{}{}",
        base("exit"),
        exit_peer("de2", "Same", "de2.example.test", "direct", None),
        exit_peer("de3", "Same", "de3.example.test", "direct", None)
    );
    expect_err(&dup_tag, "reserved or already used");
    let reserved = format!(
        "{}{}",
        base("exit"),
        exit_peer("de2", "select", "de2.example.test", "direct", None)
    );
    expect_err(&reserved, "reserved or already used");
}

#[test]
fn duplicate_access_path_ids_are_refused() {
    expect_err(
        &format!("{}{RELAY_INGRESS}{RELAY_INGRESS}", base("relay")),
        "duplicate [[access_paths]] id",
    );
}

#[test]
fn future_schema_is_refused() {
    let text = paired_relay_toml().replace("schema_version = 2", "schema_version = 3");
    assert!(matches!(
        load(&text),
        Err(CompatError::UnsupportedSchema { found: 3, .. })
    ));
}

#[test]
fn truncated_deployment_state_fails_to_load() {
    let full = paired_relay_toml();
    for cut in [10, full.len() / 3, full.len() / 2] {
        assert!(
            load(&full[..cut]).is_err(),
            "a deployment.toml truncated at byte {cut} must not load"
        );
    }
}

// ----------------------------------------------------------------------
// Server rendering: one canonical, role-aware path
// ----------------------------------------------------------------------

#[test]
fn exit_rendering_keeps_inbounds_outbounds_identical_and_adds_mandatory_c16_route() {
    // Historically the exit renderer added nothing to the raw
    // inbounds/outbounds document (this test used to assert byte
    // identity including an absent `route`). Since the C-16 egress
    // isolation fix, every exit ALWAYS carries the mandatory
    // deny-private/metadata/loopback route policy — see
    // `server::apply_c16_egress_policy`. inbounds/outbounds/log are
    // still untouched.
    let cfg = load(&base("exit")).unwrap();
    let users = vec![user()];
    let historical = render_singbox_server_config(
        &users,
        &reality(),
        &hysteria(),
        ServerPorts {
            vless_reality_port: 443,
            hysteria2_port: 443,
        },
        1_000,
    );
    let role_aware = render(&cfg, &users);
    for key in ["inbounds", "outbounds", "log"] {
        assert_eq!(historical[key], role_aware[key], "{key} must be unchanged");
    }
    let rules = role_aware["route"]["rules"].as_array().unwrap();
    assert_eq!(
        rules.len(),
        4,
        "resolve + ipv4 deny + ipv6 deny + tcp/25 deny"
    );
    assert_eq!(rules[0]["action"], "resolve");
    assert_eq!(
        rules[1]["ip_cidr"],
        serde_json::json!(compat_config::model::C16_DENY_IPV4_CIDRS)
    );
    assert_eq!(rules[1]["action"], "reject");
    assert_eq!(
        rules[2]["ip_cidr"],
        serde_json::json!(compat_config::model::C16_DENY_IPV6_CIDRS)
    );
    assert_eq!(rules[2]["action"], "reject");
    assert_eq!(rules[3]["port"], 25);
    assert_eq!(rules[3]["action"], "reject");
    assert_eq!(role_aware["route"]["final"], "direct");
}

#[test]
fn exit_denies_customer_tunnel_to_its_own_public_ipv4() {
    // Area 6 (own-public-IP egress gap): a customer must not be able to
    // use the tunnel to reach this node's own publicly-bound services by
    // targeting its public IPv4 literal directly.
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"203.0.113.9\"",
    );
    let cfg = load(&text).unwrap();
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(
        rules.len(),
        5,
        "4 mandatory C-16 rules + 1 own-public-IPv4 deny"
    );
    assert_eq!(rules[4]["ip_cidr"], serde_json::json!(["203.0.113.9/32"]));
    assert_eq!(rules[4]["action"], "reject");
    assert_eq!(compat_config::server::c16_egress_policy_rule_count(&doc), 5);
}

#[test]
fn exit_denies_customer_tunnel_to_its_own_public_ipv6() {
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"2001:db8::9\"",
    );
    let cfg = load(&text).unwrap();
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(rules.len(), 5);
    assert_eq!(rules[4]["ip_cidr"], serde_json::json!(["2001:db8::9/128"]));
    assert_eq!(rules[4]["action"], "reject");
}

#[test]
fn exit_with_hostname_public_host_adds_no_extra_own_ip_rule() {
    // `public_host` as a bare hostname (the common case: `base()`'s
    // default) cannot be safely turned into an IP-literal deny without
    // an extra network call at render time, so no 5th rule is added —
    // this only guards the case tested above where `public_host` is
    // already an IP literal. `public_ipv4`/`public_ipv6` are unset here
    // too (backward-compat default), so this remains the "nothing extra
    // known" baseline.
    let cfg = load(&base("exit")).unwrap();
    let doc = render(&cfg, &[user()]);
    assert_eq!(rules(&doc).len(), 4);
}

#[test]
fn exit_with_hostname_public_host_and_public_ipv4_field_denies_customer_tunnel() {
    // Area 6 follow-up: the common case is a hostname `public_host`
    // (unchanged above), but a deployment can now separately declare its
    // known public IPv4 via `public_ipv4`. That must still produce the
    // own-address deny rule even though `public_host` itself never
    // parses as an IP literal.
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"de1.example.test\"\npublic_ipv4 = \"203.0.113.9\"",
    );
    let cfg = load(&text).unwrap();
    assert_eq!(cfg.public_host, "de1.example.test");
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(
        rules.len(),
        5,
        "4 mandatory C-16 rules + 1 own-public-IPv4 deny from public_ipv4"
    );
    assert_eq!(rules[4]["ip_cidr"], serde_json::json!(["203.0.113.9/32"]));
    assert_eq!(rules[4]["action"], "reject");
    assert_eq!(compat_config::server::c16_egress_policy_rule_count(&doc), 5);
}

#[test]
fn exit_with_hostname_public_host_and_public_ipv6_field_denies_customer_tunnel() {
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"de1.example.test\"\npublic_ipv6 = \"2001:db8::9\"",
    );
    let cfg = load(&text).unwrap();
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(rules.len(), 5);
    assert_eq!(rules[4]["ip_cidr"], serde_json::json!(["2001:db8::9/128"]));
    assert_eq!(rules[4]["action"], "reject");
}

#[test]
fn exit_with_public_ipv4_and_public_ipv6_both_set_combines_into_one_rule() {
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"de1.example.test\"\npublic_ipv4 = \"203.0.113.9\"\npublic_ipv6 = \"2001:db8::9\"",
    );
    let cfg = load(&text).unwrap();
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(rules.len(), 5, "both addresses collapse into one rule");
    assert_eq!(
        rules[4]["ip_cidr"],
        serde_json::json!(["203.0.113.9/32", "2001:db8::9/128"])
    );
    assert_eq!(compat_config::server::c16_egress_policy_rule_count(&doc), 5);
}

#[test]
fn deployment_toml_without_public_ipv4_ipv6_fields_still_loads_and_validates() {
    // Backward compatibility: every existing deployment.toml predates
    // these fields. Confirm the absence of `public_ipv4`/`public_ipv6`
    // is not a load/validate regression and both default to None.
    let cfg = load(&base("exit")).unwrap();
    assert!(cfg.public_ipv4.is_none());
    assert!(cfg.public_ipv6.is_none());
    assert!(cfg.validate().is_ok());
}

#[test]
fn invalid_public_ipv4_literal_is_rejected_by_validate() {
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"de1.example.test\"\npublic_ipv4 = \"not-an-ip\"",
    );
    let err = load(&text).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("public_ipv4"), "{msg}");
}

#[test]
fn ipv6_literal_in_public_ipv4_field_is_rejected() {
    let text = base("exit").replace(
        "public_host = \"de1.example.test\"",
        "public_host = \"de1.example.test\"\npublic_ipv4 = \"2001:db8::9\"",
    );
    let err = load(&text).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("public_ipv4"), "{msg}");
}

#[test]
fn legacy_exit_without_identity_renders_identically_to_current_exit() {
    let legacy = base("exit")
        .replace("schema_version = 2\n", "")
        .replace("node_id = \"de1\"\n", "")
        .replace("role = \"exit\"\n", "");
    let users = vec![user()];
    assert_eq!(
        render(&load(&legacy).unwrap(), &users),
        render(&load(&base("exit")).unwrap(), &users)
    );
}

fn rules(doc: &serde_json::Value) -> &Vec<serde_json::Value> {
    doc["route"]["rules"].as_array().expect("relay route.rules")
}

#[test]
fn unpaired_relay_rejects_everything_when_no_probe_principal_exists() {
    // No reserved probe user exists (customer-only). The loopback
    // self-test exception (spec C-16 "Exceptions" / NEW-01 remediation)
    // must NOT appear at all in this case — there is nothing to scope it
    // to, and a generic unscoped loopback exception is exactly the
    // relay-loopback-pivot vulnerability this fixes.
    let cfg = load(&format!("{}{RELAY_INGRESS}", base("relay"))).unwrap();
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(rules.len(), 1, "final reject only, no loopback exception");
    assert_eq!(rules[0]["action"], "reject");
    assert!(doc["route"].get("final").is_none());
}

#[test]
fn unpaired_relay_loopback_selftest_is_scoped_to_the_reserved_probe_principal() {
    let cfg = load(&format!("{}{RELAY_INGRESS}", base("relay"))).unwrap();
    let doc = render(&cfg, &[user(), probe_user()]);
    let rules = rules(&doc);
    // 1 loopback exception (scoped to the reserved principal) + 3
    // probe-confinement rules + 1 final reject. The loopback rule is MORE
    // specific than the confinement rules' own blanket reject for that
    // same principal, so it is kept ahead of them, not after — see
    // `apply_probe_user_confinement`'s doc comment.
    assert_eq!(rules.len(), 5);
    let loopback = &rules[0];
    assert_eq!(loopback["auth_user"], serde_json::json!(["u-probe"]));
    assert_eq!(loopback["ip_cidr"], serde_json::json!(["127.0.0.1/32"]));
    assert_eq!(loopback["port"], 9100);
    assert_eq!(loopback["inbound"], serde_json::json!(["vless-reality-in"]));
    assert_eq!(loopback["network"], "tcp");
    assert_eq!(rules[4]["action"], "reject");
    // A customer never appears in the loopback rule's auth_user list.
    assert!(!loopback.to_string().contains("\"u1\""));
}

#[test]
fn a_customer_literally_named_arcana_probe_gains_no_loopback_privilege() {
    // Structural, not name-based: a real customer who happens to pick
    // the reserved display name must NOT be confused with the reserved
    // internal principal. Their record has `is_reserved_probe: false`
    // (the default for every customer-facing creation path), so they
    // get no loopback exception and no probe-only allow rules.
    let cfg = load(&format!("{}{RELAY_INGRESS}", base("relay"))).unwrap();
    let mut impostor = user();
    impostor.id = "u-impostor".into();
    impostor.name = compat_config::model::PROBE_USER_NAME.into();
    assert!(!impostor.is_reserved_probe);
    let doc = render(&cfg, &[impostor]);
    let rules = rules(&doc);
    assert_eq!(
        rules.len(),
        1,
        "a same-named customer must not unlock the loopback exception"
    );
    assert_eq!(rules[0]["action"], "reject");
}

#[test]
fn paired_relay_allows_only_declared_exits_and_ends_in_reject_for_every_inbound() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let doc = render(&cfg, &[user()]);
    let rules = rules(&doc);
    assert_eq!(
        rules.len(),
        2,
        "one de-duplicated exit + reject (no probe principal, so no loopback exception either)"
    );
    let exit_rule = &rules[0];
    assert_eq!(exit_rule["domain"], serde_json::json!(["de1.example.test"]));
    assert_eq!(exit_rule["port"], 443);
    assert_eq!(exit_rule["network"], "tcp");
    assert_eq!(exit_rule["outbound"], "direct");
    assert_eq!(
        exit_rule["inbound"],
        serde_json::json!(["vless-reality-in"])
    );

    let last = rules.last().unwrap();
    assert_eq!(last["action"], "reject");
    let rejected: Vec<&str> = last["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag.as_str().unwrap())
        .collect();
    for inbound in doc["inbounds"].as_array().unwrap() {
        assert!(
            rejected.contains(&inbound["tag"].as_str().unwrap()),
            "inbound {} escapes the final reject",
            inbound["tag"]
        );
    }
    // Every allow rule is narrower than "any destination".
    for rule in &rules[..rules.len() - 1] {
        assert_eq!(rule["action"], "route");
        assert!(rule.get("port").is_some());
        assert!(rule.get("domain").is_some() || rule.get("ip_cidr").is_some());
    }
}

#[test]
fn ip_literal_exit_targets_become_exact_host_cidrs() {
    for (host, cidr) in [
        ("192.0.2.10", "192.0.2.10/32"),
        ("2001:db8::10", "2001:db8::10/128"),
    ] {
        let text = format!(
            "{}{RELAY_INGRESS}{}",
            base("relay"),
            exit_peer("de1-via", "Germany via", host, "via-ru1", None)
        );
        let doc = render(&load(&text).unwrap(), &[user()]);
        assert_eq!(rules(&doc)[0]["ip_cidr"], serde_json::json!([cidr]));
    }
}

#[test]
fn relay_renderer_refuses_a_malformed_in_memory_declaration() {
    let mut cfg = load(&paired_relay_toml()).unwrap();
    cfg.access_paths.clear();
    let err = render_server_config_for_deployment(&cfg, &[user()], &reality(), &hysteria(), 0)
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("requires an [[access_paths]] entry"));
}

#[test]
fn disabled_and_expired_users_are_absent_from_the_relay_document() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let mut disabled = user();
    disabled.enabled = false;
    let mut expired = user();
    expired.id = "u2".into();
    expired.vless_uuid = "33333333-3333-4333-8333-333333333333".into();
    expired.expires_at = Some(500);
    let doc = render(&cfg, &[disabled, expired]);
    assert!(doc["inbounds"][0]["users"].as_array().unwrap().is_empty());
    assert!(doc["inbounds"][1]["users"].as_array().unwrap().is_empty());
    assert_eq!(rules(&doc).last().unwrap()["action"], "reject");
}

#[test]
fn relay_document_contains_no_exit_credential_and_no_peer_private_material() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let text = serde_json::to_string(&render(&cfg, &[user()])).unwrap();
    assert!(
        text.contains(RELAY_UUID),
        "the relay authenticates its own users"
    );
    assert!(
        !text.contains(EXIT_UUID),
        "the exit credential authenticates at the exit; the relay must never hold it in its server config"
    );
    assert_eq!(text.matches("SYNTHETIC-RELAY-PRIVATE-KEY").count(), 1);
}

#[test]
fn every_rendered_relay_route_rule_is_accepted_by_real_sing_box() {
    let Some(sb) = common::SingBox::find() else {
        if std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok() {
            panic!("real sing-box is required but was not found");
        }
        eprintln!("skipping: sing-box not found");
        return;
    };
    let cfg = load(&paired_relay_toml()).unwrap();
    let production = render(&cfg, &[user()]);
    // The complete production route section, checked against inbounds of
    // the same tags (the REALITY/TLS material in this unit fixture is
    // synthetic; `two_hop_system.rs` runs the full real document).
    let minimal = serde_json::json!({
        "inbounds": [
            {"type": "mixed", "tag": "vless-reality-in", "listen": "127.0.0.1", "listen_port": common::free_port()},
            {"type": "mixed", "tag": "hysteria2-in", "listen": "127.0.0.1", "listen_port": common::free_port()}
        ],
        "outbounds": production["outbounds"].clone(),
        "route": production["route"].clone()
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay-route.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&minimal).unwrap()).unwrap();
    let out = sb.check(&path);
    assert!(
        out.status.success(),
        "sing-box rejected the relay route: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ----------------------------------------------------------------------
// First-party provisioning path and legacy share links
// ----------------------------------------------------------------------

fn served(cfg: &DeploymentConfig) -> Vec<compat_config::CompatEndpoint> {
    cfg.served_endpoints(EXIT_KEY, "11223344", None).unwrap()
}

#[test]
fn relay_serves_its_first_hop_but_never_its_hysteria2_listener() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let ids: Vec<String> = served(&cfg).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, vec!["reality-1", "de1-direct", "de1-via-ru1"]);

    let exit = load(&base("exit")).unwrap();
    let ids: Vec<String> = served(&exit).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, vec!["reality-1", "hysteria2-1"]);
}

#[test]
fn provisioning_path_builds_a_real_detour_chain_with_distinct_credentials() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let paths = cfg.contract_access_paths().unwrap();
    let doc = provisioning_document_with_mode_and_access_paths(
        &user(),
        &served(&cfg),
        DiagnosticMode::None,
        &paths,
    )
    .unwrap();

    let catalog: Vec<&str> = doc.endpoints.iter().map(|e| e.tag.as_str()).collect();
    assert_eq!(catalog, vec!["Germany · Direct", "Germany · via Russia"]);

    let config = doc.singbox_config.as_ref().unwrap();
    let outbounds = config["outbounds"].as_array().unwrap();
    let first_hop = outbounds
        .iter()
        .find(|o| o["server"] == "ru1.example.test")
        .expect("hidden first hop");
    let via = outbounds
        .iter()
        .find(|o| o["tag"] == "Germany · via Russia")
        .unwrap();
    let direct = outbounds
        .iter()
        .find(|o| o["tag"] == "Germany · Direct")
        .unwrap();
    assert_eq!(via["detour"], first_hop["tag"]);
    assert_eq!(via["server"], "de1.example.test");
    assert!(direct.get("detour").is_none());
    assert_eq!(
        first_hop["uuid"], RELAY_UUID,
        "first hop authenticates with credential A"
    );
    assert_eq!(
        via["uuid"], EXIT_UUID,
        "exit authenticates with credential B"
    );
    assert_eq!(direct["uuid"], EXIT_UUID);
    assert_ne!(first_hop["uuid"], via["uuid"]);

    let selector = outbounds.iter().find(|o| o["tag"] == "select").unwrap();
    let options: Vec<&str> = selector["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(!options.contains(&first_hop["tag"].as_str().unwrap()));
    let auto = outbounds.iter().find(|o| o["tag"] == "auto").unwrap();
    assert!(!auto["outbounds"]
        .as_array()
        .unwrap()
        .contains(&first_hop["tag"]));

    let envelope = doc.to_json().unwrap();
    let access_paths_json = serde_json::to_string(&doc.access_paths).unwrap();
    for secret in [RELAY_UUID, EXIT_UUID, "relay-hy2-password"] {
        assert!(
            !access_paths_json.contains(secret),
            "access-path metadata must never carry a credential"
        );
    }
    assert!(!envelope.contains("SYNTHETIC-RELAY-PRIVATE-KEY"));
}

#[test]
fn removing_the_exit_credential_removes_both_dependent_routes() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let mut u = user();
    u.peer_credentials.clear();
    let err = provisioning_document_with_mode_and_access_paths(
        &u,
        &served(&cfg),
        DiagnosticMode::None,
        &cfg.contract_access_paths().unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompatError::NoSelectableRoute),
        "a relay user with no exit credential has no route, and must never be handed the first hop as an exit: {err}"
    );
}

#[test]
fn unpaired_relay_provisioning_is_an_explicit_no_route_error() {
    let cfg = load(&format!("{}{RELAY_INGRESS}", base("relay"))).unwrap();
    let err = provisioning_document_with_mode_and_access_paths(
        &user(),
        &served(&cfg),
        DiagnosticMode::None,
        &cfg.contract_access_paths().unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, CompatError::NoSelectableRoute));
}

#[test]
fn share_links_never_express_a_relay_route_or_the_relay_first_hop() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let paths = cfg.contract_access_paths().unwrap();
    assert!(infrastructure_endpoint_ids(&paths).contains("reality-1"));
    let shareable = share_link_endpoints(&served(&cfg), &paths);
    let ids: Vec<&str> = shareable.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["de1-direct"]);

    let links = render_uri_list(&user(), &shareable).unwrap();
    assert_eq!(links.lines().count(), 1);
    assert!(links.contains("@de1.example.test:443"));
    assert!(links.contains(EXIT_UUID));
    assert!(!links.contains("ru1.example.test"));
    assert!(!links.contains(RELAY_UUID));
}

#[test]
fn share_links_omit_a_peer_the_user_has_no_credential_for() {
    let cfg = load(&paired_relay_toml()).unwrap();
    let paths = cfg.contract_access_paths().unwrap();
    let mut u = user();
    u.peer_credentials.clear();
    let links = render_uri_list(&u, &share_link_endpoints(&served(&cfg), &paths)).unwrap();
    assert!(links.is_empty());
}

// ----------------------------------------------------------------------
// Credential lifecycle (independent scopes)
// ----------------------------------------------------------------------

fn doc_for(u: &CompatUser) -> serde_json::Value {
    let cfg = load(&paired_relay_toml()).unwrap();
    provisioning_document_with_mode_and_access_paths(
        u,
        &served(&cfg),
        DiagnosticMode::None,
        &cfg.contract_access_paths().unwrap(),
    )
    .unwrap()
    .singbox_config
    .unwrap()
}

fn uuid_of(config: &serde_json::Value, predicate: impl Fn(&serde_json::Value) -> bool) -> String {
    config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| predicate(o))
        .unwrap()["uuid"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn rotating_the_first_hop_credential_does_not_touch_the_exit_credential() {
    let before = user();
    let mut after = user();
    after.vless_uuid = "44444444-4444-4444-8444-444444444444".into();
    let (b, a) = (doc_for(&before), doc_for(&after));
    let first_hop = |o: &serde_json::Value| o["server"] == "ru1.example.test";
    let via = |o: &serde_json::Value| o["tag"] == "Germany · via Russia";
    assert_ne!(uuid_of(&b, first_hop), uuid_of(&a, first_hop));
    assert_eq!(uuid_of(&b, via), uuid_of(&a, via));

    let cfg = load(&paired_relay_toml()).unwrap();
    let old_server = serde_json::to_string(&render(&cfg, &[before])).unwrap();
    let new_server = serde_json::to_string(&render(&cfg, &[after])).unwrap();
    assert!(old_server.contains(RELAY_UUID) && !new_server.contains(RELAY_UUID));
}

#[test]
fn rotating_the_exit_credential_does_not_touch_the_first_hop_credential() {
    let before = user();
    let mut after = user();
    after.peer_credentials.insert(
        "de1-direct".into(),
        PeerCredential::VlessReality {
            uuid: "55555555-5555-4555-8555-555555555555".into(),
        },
    );
    let (b, a) = (doc_for(&before), doc_for(&after));
    let first_hop = |o: &serde_json::Value| o["server"] == "ru1.example.test";
    let via = |o: &serde_json::Value| o["tag"] == "Germany · via Russia";
    let direct = |o: &serde_json::Value| o["tag"] == "Germany · Direct";
    assert_eq!(uuid_of(&b, first_hop), uuid_of(&a, first_hop));
    assert_ne!(uuid_of(&b, via), uuid_of(&a, via));
    assert_eq!(
        uuid_of(&a, via),
        uuid_of(&a, direct),
        "one exit credential, two routes"
    );

    let cfg = load(&paired_relay_toml()).unwrap();
    assert_eq!(
        render(&cfg, &[before]),
        render(&cfg, &[after]),
        "an exit credential lives at the exit: the relay's server config must not change"
    );
}

#[test]
fn subscription_token_is_independent_of_both_proxy_credentials() {
    let mut rotated = user();
    rotated.subscription_token_hash_hex = "different-token-hash".into();
    assert_eq!(doc_for(&user()), doc_for(&rotated));
    let cfg = load(&paired_relay_toml()).unwrap();
    assert_eq!(render(&cfg, &[user()]), render(&cfg, &[rotated]));
}

#[test]
fn re_enabling_a_user_never_fabricates_a_missing_peer_credential() {
    let mut u = user();
    u.peer_credentials.clear();
    u.enabled = false;
    u.enabled = true;
    assert!(u.peer_credential("de1-direct").is_none());
    let cfg = load(&paired_relay_toml()).unwrap();
    assert!(matches!(
        provisioning_document_with_mode_and_access_paths(
            &u,
            &served(&cfg),
            DiagnosticMode::None,
            &cfg.contract_access_paths().unwrap()
        ),
        Err(CompatError::NoSelectableRoute)
    ));
}

// ----------------------------------------------------------------------
// Migration keeps role and identity
// ----------------------------------------------------------------------

#[test]
fn migration_of_a_v1_relay_keeps_the_relay_role() {
    let v1 = paired_relay_toml()
        .replace("schema_version = 2", "schema_version = 1")
        .replace("node_id = \"ru1\"\n", "");
    let migrated = migrate_deployment_toml_text(&v1).expect("v1 needs migration");
    let cfg = load(&migrated).unwrap();
    assert_eq!(cfg.role, NodeRole::Relay);
    assert_eq!(cfg.node_id, "ru1");
    assert_eq!(migrate_deployment_toml_text(&migrated), None, "idempotent");
}

#[test]
fn migration_only_reads_top_level_keys() {
    // `role`/`node_id` inside a table are not the top-level keys, and must
    // not suppress stamping them.
    let text = "public_host = \"de9.example.test\"\nsubscription_host = \"x\"\n\n[reality]\nlisten_port = 443\nhandshake_server = \"www.example.com\"\n# role = \"relay\"\n\n[hysteria2]\nlisten_port = 443\n\n[subscription]\nlisten_port = 9100\n";
    let migrated = migrate_deployment_toml_text(text).unwrap();
    assert!(migrated.starts_with("schema_version = 2\nnode_id = \"de9\"\nrole = \"exit\"\n"));
    assert_eq!(load(&migrated).unwrap().role, NodeRole::Exit);
}

#[test]
fn migration_preserves_an_existing_node_id() {
    let text = base("exit").replace("schema_version = 2\n", "schema_version = 1\n");
    let migrated = migrate_deployment_toml_text(&text).unwrap();
    assert_eq!(migrated.matches("node_id").count(), 1);
    assert_eq!(load(&migrated).unwrap().node_id, "de1");
}

#[test]
fn failed_migration_leaves_previous_state_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deployment.toml");
    // A v1 relay whose declaration is broken cannot migrate; the original
    // file must survive byte-for-byte.
    let broken = format!(
        "{}{}",
        base("relay").replace("schema_version = 2", "schema_version = 1"),
        RELAY_INGRESS.replace("reality-1", "ghost")
    );
    std::fs::write(&path, &broken).unwrap();
    assert!(migrate_deployment_toml(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);

    let current = paired_relay_toml();
    std::fs::write(&path, &current).unwrap();
    assert_eq!(
        migrate_deployment_toml(&path).unwrap(),
        DeploymentMigrationOutcome::AlreadyCurrent
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), current);
}

// ----------------------------------------------------------------------
// Fresh-install templates (the installer's actual input)
// ----------------------------------------------------------------------

#[test]
fn fresh_install_templates_render_a_current_loadable_deployment_for_both_roles() {
    let templates =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/almalinux/templates");
    let template = std::fs::read_to_string(templates.join("deployment.toml.template")).unwrap();
    let ingress = std::fs::read_to_string(templates.join("relay-ingress.toml.template")).unwrap();
    let render = |role: &str, node: &str| {
        template
            .replace("{{PUBLIC_HOST}}", "node.example.test")
            .replace("{{SUBSCRIPTION_HOST}}", "node.example.test")
            .replace("{{SUBSCRIPTION_PORT}}", "8443")
            .replace("{{REALITY_HANDSHAKE_SERVER}}", "www.example.com")
            .replace("{{NODE_ID}}", node)
            .replace("{{NODE_ROLE}}", role)
    };

    let exit = load(&render("exit", "de1")).unwrap();
    assert_eq!(
        exit.schema_version,
        compat_config::deployment::DEPLOYMENT_SCHEMA_VERSION
    );
    assert_eq!((exit.role, exit.node_id.as_str()), (NodeRole::Exit, "de1"));
    let exit_rules =
        render_server_config_for_deployment(&exit, &[user()], &reality(), &hysteria(), 0).unwrap()
            ["route"]["rules"]
            .as_array()
            .unwrap()
            .len();
    assert_eq!(
        exit_rules, 4,
        "mandatory C-16 egress isolation, no probe/hairpin extras"
    );
    assert_eq!(
        migrate_deployment_toml_text(&render("exit", "de1")),
        None,
        "a fresh install must not need migration"
    );

    let relay_text = format!("{}{ingress}", render("relay", "ru1"));
    let relay = load(&relay_text).unwrap();
    assert_eq!(
        (relay.role, relay.node_id.as_str()),
        (NodeRole::Relay, "ru1")
    );
    assert!(relay.relay_targets().is_empty());
    let doc =
        render_server_config_for_deployment(&relay, &[user()], &reality(), &hysteria(), 0).unwrap();
    assert_eq!(
        doc["route"]["rules"].as_array().unwrap().len(),
        1,
        "final reject only: no probe principal provisioned yet, no relay targets declared"
    );
    assert_eq!(migrate_deployment_toml_text(&relay_text), None);

    // The relay template without its ingress declaration must not load.
    assert!(load(&render("relay", "ru1")).is_err());
}

// ----------------------------------------------------------------------
// Google/YouTube egress hairpin (docs/YOUTUBE_FINAL_ROOT_CAUSE.md §16):
// an EXIT can hold one relay user's credential and hairpin the Google/
// YouTube domain set through a relay with better network peering to
// Google's CDN. Never a client-facing feature — see
// `CompatUser::google_egress_hairpin`'s doc comment.

fn hairpin_user() -> CompatUser {
    let mut u = user();
    u.id = "u-hairpin".into();
    u.name = "google-egress-hairpin".into();
    u.vless_uuid = "55555555-5555-4555-8555-555555555555".into();
    u.peer_credentials = Default::default();
    u.google_egress_hairpin = true;
    u
}

#[test]
fn relay_hairpin_flag_off_leaves_relay_rendering_byte_identical() {
    let relay = load(&paired_relay_toml()).unwrap();
    let without = render(&relay, &[user()]);
    let mut off = hairpin_user();
    off.google_egress_hairpin = false;
    let with_flag_off = render(&relay, &[user(), off]);
    // The extra user still renders its own inbound entry, but the RULES
    // (the fail-closed policy under test) must be identical: the second
    // user is invisible to route generation when the flag is false.
    assert_eq!(without["route"], with_flag_off["route"]);
}

#[test]
fn relay_hairpin_user_gets_exactly_two_extra_rules_scoped_to_google_domains() {
    // A hairpin user needs its OWN sniff step: the exit only sniffs to
    // decide ITS OWN routing, then forwards whatever destination form it
    // received from its own client — often a bare IP, since an iOS TUN
    // core typically resolves DNS itself before sing-box ever sees the
    // packet. Without recovering the domain again here, `domain_suffix`
    // can never match an IP-only destination and the connection falls
    // through to reject (see docs/YOUTUBE_FINAL_ROOT_CAUSE.md §16).
    let relay = load(&paired_relay_toml()).unwrap();
    let without = render(&relay, &[user()]);
    let with_hairpin = render(&relay, &[user(), hairpin_user()]);
    let without_rules = without["route"]["rules"].as_array().unwrap();
    let with_rules = with_hairpin["route"]["rules"].as_array().unwrap();
    assert_eq!(
        with_rules.len(),
        without_rules.len() + 2,
        "the hairpin user must add exactly two rules (sniff, then route), changing nothing else"
    );
    let sniff_rule = &with_rules[0];
    assert_eq!(sniff_rule["auth_user"], serde_json::json!(["u-hairpin"]));
    assert_eq!(sniff_rule["action"], "sniff");
    let hairpin_rule = &with_rules[1];
    assert_eq!(hairpin_rule["auth_user"], serde_json::json!(["u-hairpin"]));
    assert_eq!(hairpin_rule["outbound"], "direct");
    assert_eq!(
        hairpin_rule["domain_suffix"],
        serde_json::json!(compat_config::model::GOOGLE_EGRESS_DOMAINS)
    );
    // Every other rule (declared-exit forwarding, final reject) is
    // untouched and in its original relative order, just shifted by the
    // two hairpin rules prepended ahead of it.
    for i in 0..without_rules.len() {
        assert_eq!(with_rules[i + 2], without_rules[i]);
    }
}

#[test]
fn relay_hairpin_user_still_falls_through_to_reject_for_non_google_destinations() {
    // The hairpin rule matches on domain_suffix; anything that is not in
    // the Google/YouTube set (including the declared exit's own host,
    // which the hairpin user has no reason to reach) keeps hitting this
    // relay's ordinary policy — forward-to-declared-exit if it matches,
    // reject-all otherwise. This is enforced by sing-box's own rule
    // evaluation order (first match wins), not by anything this test can
    // observe directly in the rendered JSON — so this test instead pins
    // down the one property that actually matters: the final rule is
    // still an unconditional reject covering every inbound, unchanged by
    // the hairpin user's presence.
    let relay = load(&paired_relay_toml()).unwrap();
    let with_hairpin = render(&relay, &[user(), hairpin_user()]);
    let rules = with_hairpin["route"]["rules"].as_array().unwrap();
    let last = rules.last().unwrap();
    assert_eq!(last["action"], "reject");
    assert!(last.get("domain_suffix").is_none());
    assert!(last.get("auth_user").is_none());
}

fn exit_hairpin_deployment_toml() -> String {
    format!(
        "{}{}",
        base("exit"),
        r#"
[google_egress_hairpin]
relay_host = "ru1.example.test"
relay_port = 443
relay_server_name = "www.example.com"
relay_reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
relay_reality_short_id = "0a1b2c3d"
"#
    )
}

fn hairpin_reality() -> RealityServerParams {
    let mut r = reality();
    r.google_egress_hairpin_uuid = Some(SecretString::new("66666666-6666-4666-8666-666666666666"));
    r
}

#[test]
fn exit_without_hairpin_config_renders_exactly_as_before() {
    let exit = load(&base("exit")).unwrap();
    let doc =
        render_server_config_for_deployment(&exit, &[user()], &reality(), &hysteria(), 0).unwrap();
    // Mandatory C-16 egress isolation is always present now; what "as
    // before" still means here is: no hairpin outbound/rules, no sniff.
    assert_eq!(doc["route"]["rules"].as_array().unwrap().len(), 4);
    assert!(doc["inbounds"][0].get("sniff").is_none());
    assert_eq!(doc["outbounds"].as_array().unwrap().len(), 1);
}

#[test]
fn exit_with_hairpin_config_but_no_credential_still_renders_unchanged() {
    // Public metadata alone (no secret UUID loaded) must not activate
    // anything — half-configured must fail safe to "off", not "on
    // without a credential". C-16 route rules are still mandatory and
    // present either way.
    let exit = load(&exit_hairpin_deployment_toml()).unwrap();
    let doc =
        render_server_config_for_deployment(&exit, &[user()], &reality(), &hysteria(), 0).unwrap();
    assert_eq!(doc["route"]["rules"].as_array().unwrap().len(), 4);
    assert_eq!(doc["outbounds"].as_array().unwrap().len(), 1);
}

#[test]
fn exit_with_hairpin_configured_adds_one_outbound_and_sniff_plus_route_rule() {
    let exit = load(&exit_hairpin_deployment_toml()).unwrap();
    let doc =
        render_server_config_for_deployment(&exit, &[user()], &hairpin_reality(), &hysteria(), 0)
            .unwrap();
    // sing-box 1.13 removed per-inbound `sniff` as a legacy field; the
    // unconditional sniff must instead be the first route.rules entry
    // (the same shape Hiddify's own core emits).
    for inbound in doc["inbounds"].as_array().unwrap() {
        assert!(
            inbound.get("sniff").is_none(),
            "sniff must not be a legacy inbound field"
        );
    }
    let outbounds = doc["outbounds"].as_array().unwrap();
    assert_eq!(
        outbounds.len(),
        2,
        "the original direct outbound stays, plus one hairpin outbound"
    );
    let hairpin_ob = outbounds
        .iter()
        .find(|o| o["tag"] == "google-egress-hairpin")
        .expect("hairpin outbound present");
    assert_eq!(hairpin_ob["type"], "vless");
    assert_eq!(hairpin_ob["server"], "ru1.example.test");
    assert_eq!(hairpin_ob["server_port"], 443);
    assert_eq!(hairpin_ob["uuid"], "66666666-6666-4666-8666-666666666666");
    assert_eq!(
        hairpin_ob["tls"]["reality"]["public_key"],
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
    );
    assert_eq!(hairpin_ob["tls"]["reality"]["short_id"], "0a1b2c3d");
    let rules = doc["route"]["rules"].as_array().unwrap();
    assert_eq!(
        rules.len(),
        6,
        "4 mandatory C-16 rules ahead of the hairpin's sniff + route"
    );
    assert_eq!(rules[4]["action"], "sniff");
    assert_eq!(rules[5]["outbound"], "google-egress-hairpin");
    assert_eq!(
        rules[5]["domain_suffix"],
        serde_json::json!(compat_config::model::GOOGLE_EGRESS_DOMAINS)
    );
    assert_eq!(doc["route"]["final"], "direct");
}

#[test]
fn exit_hairpin_outbound_never_carries_this_exits_own_private_key() {
    // The exit's OWN inbound legitimately embeds the exit's own private
    // key (it is the exit's own listener identity) — that is not a leak.
    // What must never happen is that key ending up in the NEW hairpin
    // OUTBOUND, which only ever needs the hairpin credential (a UUID)
    // and the relay's PUBLIC key.
    let exit = load(&exit_hairpin_deployment_toml()).unwrap();
    let doc =
        render_server_config_for_deployment(&exit, &[user()], &hairpin_reality(), &hysteria(), 0)
            .unwrap();
    let outbounds = doc["outbounds"].as_array().unwrap();
    let hairpin_ob = outbounds
        .iter()
        .find(|o| o["tag"] == "google-egress-hairpin")
        .unwrap();
    let rendered = serde_json::to_string(hairpin_ob).unwrap();
    assert!(!rendered.contains("SYNTHETIC-RELAY-PRIVATE-KEY"));
    assert!(hairpin_ob.get("private_key").is_none());
}

// ----------------------------------------------------------------------
// Reserved protocol-probe user confinement (not an open proxy)
// ----------------------------------------------------------------------

fn probe_user() -> CompatUser {
    let mut u = user();
    u.id = "u-probe".into();
    u.name = compat_config::model::PROBE_USER_NAME.into();
    u.vless_uuid = "66666666-6666-4666-8666-666666666666".into();
    u.peer_credentials = Default::default();
    u.is_reserved_probe = true;
    u
}

fn url_host(url: &str) -> &str {
    url.trim_start_matches("https://")
        .split(['/', ':'])
        .next()
        .unwrap()
}

#[test]
fn probe_urls_are_exactly_covered_by_the_probe_allowlist() {
    use compat_config::model::*;
    for url in [PROBE_TRACE_V4_URL, PROBE_DNS_URL, PROBE_V6_ONLY_URL] {
        assert!(
            url.starts_with("https://"),
            "{url} must be HTTPS (port 443)"
        );
        let host = url_host(url);
        let allowed = match host.parse::<std::net::IpAddr>() {
            Ok(ip) => PROBE_ALLOWED_IP_CIDRS.contains(&format!("{ip}/32").as_str()),
            Err(_) => PROBE_ALLOWED_DOMAINS.contains(&host),
        };
        assert!(allowed, "probe URL host {host} missing from the allowlist");
    }
    assert_eq!(PROBE_ALLOWED_PORT, 443);
    // Nothing extra either: every allowlisted entry is used by a URL.
    let hosts: Vec<&str> = [PROBE_TRACE_V4_URL, PROBE_DNS_URL, PROBE_V6_ONLY_URL]
        .into_iter()
        .map(url_host)
        .collect();
    for d in PROBE_ALLOWED_DOMAINS {
        assert!(hosts.contains(d), "unused allowlisted domain {d}");
    }
    for c in PROBE_ALLOWED_IP_CIDRS {
        assert!(
            hosts.contains(&c.trim_end_matches("/32")),
            "unused cidr {c}"
        );
    }
}

/// Asserts the 3 probe-confinement rules appear, in order, starting at
/// `rules[at]`. `at` is 0 on an exit (nothing else is ever scoped to the
/// probe identity there) and 1 on a relay that also has the loopback
/// self-test exception rule (also `auth_user`-scoped to the same
/// principal, and correctly kept ahead of this function's own blanket
/// reject — see `apply_probe_user_confinement`'s doc comment).
fn assert_probe_rules_lead(rules: &[serde_json::Value], at: usize) {
    use compat_config::model::*;
    let ids = serde_json::json!(["u-probe"]);
    assert_eq!(rules[at]["auth_user"], ids);
    assert_eq!(rules[at]["network"], "tcp");
    assert_eq!(rules[at]["port"], 443);
    assert_eq!(
        rules[at]["ip_cidr"],
        serde_json::json!(PROBE_ALLOWED_IP_CIDRS)
    );
    assert_eq!(rules[at]["outbound"], "direct");
    assert_eq!(rules[at + 1]["auth_user"], ids);
    assert_eq!(
        rules[at + 1]["domain"],
        serde_json::json!(PROBE_ALLOWED_DOMAINS)
    );
    assert!(
        rules[at + 1].get("domain_suffix").is_none(),
        "exact hosts only"
    );
    assert_eq!(rules[at + 1]["outbound"], "direct");
    assert_eq!(rules[at + 2]["auth_user"], ids);
    assert_eq!(rules[at + 2]["action"], "reject");
    assert_eq!(
        rules[at + 2].as_object().unwrap().len(),
        3,
        "unconditional for that user"
    );
}

#[test]
fn exit_confines_probe_user_and_leaves_customers_unrouted() {
    let exit = load(&base("exit")).unwrap();
    // No probe user: the mandatory C-16 route is still present (4
    // rules), but nothing probe-specific.
    let customers_only = render(&exit, &[user()]);
    assert_eq!(
        customers_only["route"]["rules"].as_array().unwrap().len(),
        4
    );
    assert_eq!(
        compat_config::server::probe_confinement_rule_count(&customers_only),
        0
    );

    let with_probe = render(&exit, &[user(), probe_user()]);
    let rules = with_probe["route"]["rules"].as_array().unwrap();
    assert_eq!(
        rules.len(),
        8,
        "1 loopback self-test exception + 3 probe rules ahead of the 4 mandatory C-16 rules; \
         customers keep default direct egress"
    );
    let loopback = &rules[0];
    assert_eq!(loopback["auth_user"], serde_json::json!(["u-probe"]));
    assert_eq!(loopback["ip_cidr"], serde_json::json!(["127.0.0.1/32"]));
    assert_eq!(loopback["action"], "route");
    assert_eq!(loopback["outbound"], "direct");
    assert_probe_rules_lead(rules, 1);
    // No customer id appears in any rule.
    assert!(!with_probe["route"].to_string().contains("\"u1\""));
    assert_eq!(with_probe["route"]["final"], "direct");
    assert_eq!(
        compat_config::server::probe_confinement_rule_count(&with_probe),
        4
    );
}

#[test]
fn disabled_probe_user_gets_no_probe_rules_but_keeps_mandatory_c16() {
    let exit = load(&base("exit")).unwrap();
    let mut p = probe_user();
    p.enabled = false;
    let doc = render(&exit, &[user(), p]);
    assert_eq!(doc["route"]["rules"].as_array().unwrap().len(), 4);
    assert_eq!(compat_config::server::probe_confinement_rule_count(&doc), 0);
}

#[test]
fn a_customer_literally_named_arcana_probe_gets_no_probe_confinement_on_exit() {
    // Same structural guarantee as the relay-side test of the same name:
    // a customer picking the reserved display name is an ordinary
    // customer (`is_reserved_probe: false`) and is never matched by
    // `apply_probe_user_confinement`.
    let exit = load(&base("exit")).unwrap();
    let mut impostor = user();
    impostor.id = "u-impostor".into();
    impostor.name = compat_config::model::PROBE_USER_NAME.into();
    assert!(!impostor.is_reserved_probe);
    let doc = render(&exit, &[impostor]);
    assert_eq!(
        compat_config::server::probe_confinement_rule_count(&doc),
        0,
        "a same-named customer must not unlock probe confinement/privilege"
    );
    assert_eq!(doc["route"]["rules"].as_array().unwrap().len(), 4);
}

#[test]
fn probe_rules_precede_exit_hairpin_rules() {
    let exit = load(&exit_hairpin_deployment_toml()).unwrap();
    let mut r = reality();
    r.google_egress_hairpin_uuid = Some(SecretString::new("77777777-7777-4777-8777-777777777777"));
    let without =
        render_server_config_for_deployment(&exit, &[user()], &r, &hysteria(), 1_000).unwrap();
    let with =
        render_server_config_for_deployment(&exit, &[user(), probe_user()], &r, &hysteria(), 1_000)
            .unwrap();
    let rules = with["route"]["rules"].as_array().unwrap();
    let loopback = &rules[0];
    assert_eq!(loopback["auth_user"], serde_json::json!(["u-probe"]));
    assert_eq!(loopback["ip_cidr"], serde_json::json!(["127.0.0.1/32"]));
    assert_probe_rules_lead(rules, 1);
    assert_eq!(
        &rules[4..],
        without["route"]["rules"].as_array().unwrap().as_slice()
    );
    assert_eq!(with["route"]["final"], "direct");
}

#[test]
fn probe_rules_precede_relay_policy_which_is_otherwise_unchanged() {
    let relay = load(&paired_relay_toml()).unwrap();
    let without = render(&relay, &[user()]);
    let with = render(&relay, &[user(), probe_user()]);
    let rules = with["route"]["rules"].as_array().unwrap();
    // A probe user's presence adds exactly one more rule beyond the 3
    // probe-confinement rules: the loopback self-test exception, now
    // scoped to that same reserved principal (spec C-16 "Exceptions").
    // It is MORE specific than this function's own blanket reject, so it
    // is kept ahead of the 3 probe-confinement rules, not after them (see
    // `apply_probe_user_confinement`'s doc comment) — index 0, then the 3
    // confinement rules at 1..4, then everything else byte-identical to
    // the no-probe case.
    let loopback = &rules[0];
    assert_eq!(loopback["auth_user"], serde_json::json!(["u-probe"]));
    assert_eq!(loopback["ip_cidr"], serde_json::json!(["127.0.0.1/32"]));
    assert_probe_rules_lead(rules, 1);
    assert_eq!(
        &rules[4..],
        without["route"]["rules"].as_array().unwrap().as_slice()
    );
}
