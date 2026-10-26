//! Exit principals on a relay (bidirectional deployments, `docs/BIDIRECTIONAL.md`).
//!
//! A relay that opts in with `[exit_principals] enabled = true` may also be the EXIT of another relay's route:
//! users flagged `exit_principal` leave through the relay's own `direct` outbound under the exit egress policy,
//! everything else keeps the relay's forward-or-reject behaviour. These tests pin the renderer and the shared
//! verifier `doctor` uses (CODE-VERIFIED); the real-sing-box bidirectional scenarios are in `two_hop_system.rs`.

mod common;

use compat_config::deployment::DeploymentConfig;
use compat_config::model::{
    CompatUser, Hysteria2ServerParams, PeerCredential, RealityServerParams,
};
use compat_config::secret::SecretString;
use compat_config::server::{
    exit_principal_ids, exit_principal_rules, relay_policy_facts,
    render_server_config_for_deployment,
};
use compat_config::CompatError;
use serde_json::{json, Value};

const PEER_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
const RELAY_UUID: &str = "11111111-1111-4111-8111-111111111111";
const EXIT_UUID: &str = "22222222-2222-4222-8222-222222222222";
const PRINCIPAL_UUID: &str = "33333333-3333-4333-8333-333333333333";
const NOW: i64 = 1_000;

fn toml_text(enable: Option<bool>) -> String {
    let section = match enable {
        Some(v) => format!("\n[exit_principals]\nenabled = {v}\n"),
        None => String::new(),
    };
    format!(
        r#"schema_version = 2
node_id = "ru1"
role = "relay"
public_host = "ru1.example.test"
subscription_host = "ru1.example.test"
public_ipv4 = "203.0.113.7"

[reality]
listen_port = 443
handshake_server = "www.example.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
{section}
[[access_paths]]
id = "relay-ingress"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]

[[peer_endpoints]]
id = "de1-direct"
tag = "Germany · Direct"
host = "de1.example.test"
port = 443
transport = "vless_reality"
server_name = "www.example.com"
reality_public_key = "{PEER_KEY}"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"

[[peer_endpoints]]
id = "de1-via-ru1"
tag = "Germany · via Russia"
host = "de1.example.test"
port = 443
transport = "vless_reality"
server_name = "www.example.com"
reality_public_key = "{PEER_KEY}"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "relay-ingress"
credential_ref = "de1-direct"
"#
    )
}

fn load(text: &str) -> Result<DeploymentConfig, CompatError> {
    let cfg: DeploymentConfig =
        toml::from_str(text).map_err(|e| CompatError::Parse(e.to_string()))?;
    cfg.validate()?;
    Ok(cfg)
}

fn base_user(id: &str, name: &str, uuid: &str) -> CompatUser {
    CompatUser {
        id: id.into(),
        name: name.into(),
        enabled: true,
        vless_uuid: uuid.into(),
        hysteria2_password: SecretString::new("pw"),
        subscription_token_hash_hex: "unused".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        google_egress_hairpin: false,
        exit_principal: false,
        peer_credentials: Default::default(),
        is_reserved_probe: false,
        credentials: Vec::new(),
    }
}

fn client() -> CompatUser {
    let mut u = base_user("u1", "u1", RELAY_UUID);
    u.peer_credentials.insert(
        "de1-direct".into(),
        PeerCredential::VlessReality {
            uuid: EXIT_UUID.into(),
        },
    );
    u
}

fn principal() -> CompatUser {
    let mut u = base_user("u2", "u2", PRINCIPAL_UUID);
    u.exit_principal = true;
    u
}

fn reality() -> RealityServerParams {
    RealityServerParams {
        private_key_hex: SecretString::new("SYNTHETIC"),
        public_key_hex: PEER_KEY.into(),
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

fn render(cfg: &DeploymentConfig, users: &[CompatUser]) -> Value {
    render_server_config_for_deployment(cfg, users, &reality(), &hysteria(), NOW).unwrap()
}

fn rules(doc: &Value) -> Vec<Value> {
    doc["route"]["rules"].as_array().unwrap().clone()
}

fn direct_routes(doc: &Value) -> Vec<Value> {
    rules(doc)
        .into_iter()
        .filter(|r| r["action"] == "route" && r["outbound"] == "direct")
        .collect()
}

#[test]
fn a_node_that_did_not_opt_in_ignores_flagged_users_and_stays_fail_closed() {
    for enable in [None, Some(false)] {
        let cfg = load(&toml_text(enable)).unwrap();
        let with = render(&cfg, &[client(), principal()]);
        let without = render(&cfg, &[client()]);
        assert_eq!(
            rules(&with),
            rules(&without),
            "the flag alone must add no rule"
        );
        assert!(direct_routes(&with)
            .iter()
            .all(|r| r.get("auth_user").is_none()));
        let facts = relay_policy_facts(&cfg, &with);
        assert!(facts.fail_closed);
        assert_eq!(facts.exit_principal_rules, 0);
    }
}

#[test]
fn an_opted_in_relay_renders_one_scoped_block_that_ends_in_direct() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let doc = render(&cfg, &[client(), principal()]);
    let all = rules(&doc);
    let expected = exit_principal_rules(&cfg, &["u2".to_string()]);
    assert!(!expected.is_empty());
    let start = all
        .windows(expected.len())
        .position(|w| w == expected.as_slice())
        .expect("the exact block is present");
    assert_eq!(
        all[start + expected.len() - 1],
        json!({"inbound":["vless-reality-in"],"auth_user":["u2"],"action":"route","outbound":"direct"})
    );
    for rule in &expected {
        assert_eq!(rule["inbound"], json!(["vless-reality-in"]), "{rule}");
        assert_eq!(rule["auth_user"], json!(["u2"]), "{rule}");
    }
    // exactly one direct route in the whole document, and it is the principal's
    assert_eq!(direct_routes(&doc).len(), 1 + cfg.relay_targets().len());
    assert!(direct_routes(&doc)
        .iter()
        .filter(|r| r.get("auth_user").is_some())
        .all(|r| r["auth_user"] == json!(["u2"])));
    // the terminating reject is still last, and the forwarding rule for the declared exit follows the block
    assert_eq!(all.last().unwrap()["action"], "reject");
    assert!(start + expected.len() < all.len() - 1);
    let facts = relay_policy_facts(&cfg, &doc);
    assert!(facts.fail_closed, "{facts:?}");
    assert_eq!(facts.exit_principal_rules, expected.len());
}

#[test]
fn the_block_rejects_private_ranges_smtp_self_and_the_declared_peers_before_direct() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let block = exit_principal_rules(&cfg, &["u2".to_string()]);
    let direct_at = block.len() - 1;
    let pos = |pred: &dyn Fn(&Value) -> bool| block.iter().position(pred).expect("rule present");
    let resolve = pos(&|r| r["action"] == "resolve");
    let private = pos(&|r| {
        r["action"] == "reject"
            && r["ip_cidr"]
                .as_array()
                .is_some_and(|c| c.iter().any(|v| v == "127.0.0.0/8"))
    });
    let smtp = pos(&|r| r["action"] == "reject" && r["port"] == 25);
    let own = pos(&|r| r["ip_cidr"] == json!(["203.0.113.7/32"]));
    let peer = pos(&|r| {
        r["domain"] == json!(["de1.example.test"]) && r["port"] == 443 && r["action"] == "reject"
    });
    assert!(resolve < private && private < direct_at);
    assert!(smtp < direct_at && own < direct_at && peer < direct_at);
}

#[test]
fn only_principals_leave_directly_and_clients_never_gain_a_route() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let doc = render(&cfg, &[client(), principal()]);
    for rule in rules(&doc) {
        if let Some(users) = rule.get("auth_user") {
            assert!(
                users.as_array().unwrap().iter().all(|u| u == "u2"),
                "no rule may name the relay client: {rule}"
            );
        }
    }
}

#[test]
fn hairpin_probe_inactive_and_expired_accounts_never_become_exit_principals() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let mut hairpin = principal();
    hairpin.id = "u3".into();
    hairpin.google_egress_hairpin = true;
    let mut probe = principal();
    probe.id = "u4".into();
    probe.is_reserved_probe = true;
    let mut disabled = principal();
    disabled.id = "u5".into();
    disabled.enabled = false;
    let mut expired = principal();
    expired.id = "u6".into();
    expired.expires_at = Some(NOW - 1);
    let doc = render(&cfg, &[client(), hairpin, probe, disabled, expired]);
    assert!(
        !rules(&doc).iter().any(|r| {
            r["auth_user"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "u5" || v == "u6" || v == "u4"))
                && r.as_object().is_some_and(|o| o.len() == 4)
                && r["action"] == "route"
                && r["outbound"] == "direct"
        }),
        "none of them may receive the unrestricted direct route"
    );
    assert!(relay_policy_facts(&cfg, &doc).fail_closed);
    assert_eq!(relay_policy_facts(&cfg, &doc).exit_principal_rules, 0);
}

#[test]
fn the_documents_principals_must_equal_the_authorization_state() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let mut revoked = principal();
    revoked.id = "u7".into();
    revoked.enabled = false;
    let users = [client(), principal(), revoked.clone()];
    let doc = render(&cfg, &users);
    let mut expected = exit_principal_ids(&cfg, &users, NOW);
    expected.sort();
    assert_eq!(
        expected,
        vec!["u2".to_string()],
        "only the active flagged account"
    );
    assert_eq!(relay_policy_facts(&cfg, &doc).principals, expected);

    // A stale document that still names the since-revoked account is self-consistent for the verifier, so
    // doctor compares it to the authorization state: the sets differ and the check fails.
    let stale = render(
        &cfg,
        &[client(), principal(), {
            let mut u = revoked;
            u.enabled = true;
            u
        }],
    );
    let stale_facts = relay_policy_facts(&cfg, &stale);
    assert!(stale_facts.fail_closed, "internally consistent");
    assert_ne!(
        stale_facts.principals, expected,
        "but not what the authorization state allows"
    );

    // An expired principal and a node that did not opt in contribute nobody.
    let mut expired = principal();
    expired.expires_at = Some(NOW - 1);
    assert!(exit_principal_ids(&cfg, &[expired], NOW).is_empty());
    let off = load(&toml_text(None)).unwrap();
    assert!(exit_principal_ids(&off, &[principal()], NOW).is_empty());
}

#[test]
fn the_verifier_detects_every_tampering_of_the_block() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let doc = render(&cfg, &[client(), principal()]);
    let facts = relay_policy_facts(&cfg, &doc);
    assert!(facts.fail_closed);
    let n = facts.exit_principal_rules;
    let start = rules(&doc)
        .iter()
        .position(|r| r["action"] == "resolve" && r["auth_user"] == json!(["u2"]))
        .unwrap();

    let mutate = |f: &dyn Fn(&mut Vec<Value>)| {
        let mut d = doc.clone();
        let mut r = rules(&d);
        f(&mut r);
        d["route"]["rules"] = json!(r);
        relay_policy_facts(&cfg, &d).fail_closed
    };
    assert!(
        !mutate(&|r| {
            r.remove(start + 1);
        }),
        "a dropped private-range reject"
    );
    assert!(
        !mutate(&|r| {
            r.swap(start + 1, start + 2);
        }),
        "reordered rules"
    );
    assert!(
        !mutate(&|r| {
            r[start + n - 1]["auth_user"] = json!(["u2", "u1"]);
        }),
        "the direct route widened to another user"
    );
    assert!(
        !mutate(&|r| {
            let extra = json!({"inbound":["vless-reality-in"],"auth_user":["u1"],"action":"route","outbound":"direct"});
            r.insert(start + n, extra);
        }),
        "a second direct route"
    );
    assert!(
        !mutate(&|r| {
            r.insert(0, json!({"action":"route","outbound":"direct"}));
        }),
        "an unscoped direct route"
    );
    assert!(
        !mutate(&|r| {
            let last = r.len() - 1;
            r[last]["action"] = json!("route");
        }),
        "the terminating reject replaced"
    );

    // a block in a document whose node never opted in is itself a failure
    let plain = load(&toml_text(Some(false))).unwrap();
    assert!(!relay_policy_facts(&plain, &doc).fail_closed);
}

#[test]
fn no_principal_means_no_block_even_when_enabled() {
    let cfg = load(&toml_text(Some(true))).unwrap();
    let doc = render(&cfg, &[client()]);
    assert_eq!(
        rules(&doc),
        rules(&render(&load(&toml_text(None)).unwrap(), &[client()]))
    );
    assert!(relay_policy_facts(&cfg, &doc).fail_closed);
}

#[test]
fn validation_keeps_the_section_relay_only_and_unknown_keys_out() {
    let exit = toml_text(Some(true)).replace("role = \"relay\"", "role = \"exit\"");
    assert!(load(&exit).is_err());
    let typo = toml_text(None) + "\n[exit_principals]\nenable = true\n";
    assert!(load(&typo).is_err());
    let cfg = load(&toml_text(Some(true))).unwrap();
    let again = load(&toml::to_string(&cfg).unwrap()).unwrap();
    assert!(again.exit_principals_enabled());
}

/// The real sing-box validator must accept the hybrid document in its heaviest shape (principal-scoped
/// `resolve`/`reject`/`sniff`/`route-options` rules plus the TLS spoof). Skips without a sing-box binary; the
/// `singbox-validate` CI job provides one (SINGBOX_VPN_REQUIRE_REAL_INTEROP makes a missing binary a failure).
#[test]
fn the_real_singbox_validator_accepts_the_hybrid_document() {
    let Some(sb) = common::SingBox::find() else {
        assert!(
            std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_err(),
            "sing-box is required for this check"
        );
        eprintln!("skipping: sing-box not available");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let keys = std::process::Command::new(&sb.path)
        .args(["generate", "reality-keypair"])
        .output()
        .expect("generate reality keypair");
    let text = String::from_utf8_lossy(&keys.stdout).to_string();
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("{name}:")))
            .map(|v| v.trim().to_string())
            .expect("keypair field")
    };
    let (cert, key) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
    let made = std::process::Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "ed25519",
            "-days",
            "1",
            "-nodes",
            "-subj",
            "/CN=hybrid.test",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output()
        .expect("openssl");
    assert!(made.status.success());
    let reality = RealityServerParams {
        private_key_hex: SecretString::new(field("PrivateKey")),
        public_key_hex: field("PublicKey"),
        short_ids: vec!["0a1b2c3d".into()],
        handshake_server: "www.example.com".into(),
        handshake_port: 443,
        google_egress_hairpin_uuid: None,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: cert.to_string_lossy().into_owned(),
        tls_key_path: key.to_string_lossy().into_owned(),
        obfs_password: None,
        masquerade_dir_path: None,
        up_mbps: None,
        down_mbps: None,
    };
    for spoof in [false, true] {
        let mut toml = toml_text(Some(true));
        if spoof {
            toml.push_str("\n[google_egress_tls_spoof]\nsni = \"ya.ru\"\n");
        }
        let cfg = load(&toml).unwrap();
        let doc = render_server_config_for_deployment(
            &cfg,
            &[client(), principal()],
            &reality,
            &hysteria,
            NOW,
        )
        .unwrap();
        assert!(relay_policy_facts(&cfg, &doc).fail_closed);
        let path = dir.path().join(format!("hybrid-{spoof}.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        let out = sb.check(&path);
        assert!(
            out.status.success(),
            "sing-box rejected the hybrid document (spoof={spoof}): {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
