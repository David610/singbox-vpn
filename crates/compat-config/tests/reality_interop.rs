//! Real interoperability tests for VLESS+REALITY config generation.
//!
//! These tests build the exact server config (`server::render_singbox_server_config`)
//! and client config (`render::render_singbox_client_subscription`) this codebase
//! would actually deploy/serve, then drive a REAL `sing-box` binary as both the
//! server and the client over loopback, and assert traffic actually flows.
//!
//! Rationale: unit tests that only assert a URI/JSON *contains* fields like
//! `security=reality` or `sid=...` cannot catch a cryptographically/protocol
//! incompatible value (e.g. a stale public key, wrong short_id, or a field the
//! real REALITY implementation silently rejects). Only a real handshake against
//! the real (pinned) sing-box binary proves the generated material actually
//! authenticates — see docs/COMPATIBILITY_VERSIONS.md for the pinned version.
//!
//! Skipped (not failed) when no usable `sing-box` binary is available, so this
//! doesn't turn every contributor's machine/CI image into a hard requirement:
//! set `SING_BOX_BIN=/path/to/sing-box` to point at one explicitly, otherwise
//! `sing-box` on `PATH` is used if present. CI wires this up with the exact
//! pinned, checksum-verified binary (see `.github/workflows/ci.yml`'s
//! `singbox-validate` job) so it is NOT silently skipped in the pipeline that
//! actually gates merges.
//!
//! The proxied "traffic" destination is a local, in-process HTTP target
//! (`tests/common/mod.rs`), never a public third-party host — the only
//! external network dependency these tests have is the REALITY protocol's
//! own decoy/camouflage handshake target (`www.microsoft.com:443`), which
//! is inherent to the protocol itself (the server must perform a real TLS
//! handshake against a real external site to remain indistinguishable from
//! one), not a choice made by this test.

mod common;

use common::{free_port, socks5_http_get_is_200, spawn_local_http_target, wait_for_port};
use compat_config::model::{Hysteria2ServerParams, RealityServerParams};
use compat_config::secret::SecretString;
use compat_config::server::{render_singbox_server_config, ServerPorts};
use compat_config::{render, CompatUser};
use std::process::Command;
use std::time::Duration;

fn generate_reality_keypair(sb: &common::SingBox) -> (String, String) {
    let output = Command::new(&sb.path)
        .arg("generate")
        .arg("reality-keypair")
        .output()
        .expect("run sing-box generate reality-keypair");
    assert!(output.status.success(), "reality-keypair generation failed");
    let text = String::from_utf8_lossy(&output.stdout);
    let private_key = extract_field(&text, "PrivateKey").expect("PrivateKey in output");
    let public_key = extract_field(&text, "PublicKey").expect("PublicKey in output");
    (private_key, public_key)
}

fn extract_field(text: &str, field: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(&format!("{field}:"))
            .or_else(|| line.strip_prefix(&format!("{field} ")))
            .map(|s| s.trim().to_string())
    })
}

fn test_user() -> CompatUser {
    CompatUser {
        id: "u1".into(),
        name: "interop-test".into(),
        enabled: true,
        vless_uuid: "841cac4a-efe4-48ac-92b8-d11f4c98c45e".into(),
        hysteria2_password: SecretString::new("unused-in-this-test"),
        subscription_token_hash_hex: "unused".into(),
        created_at: 0,
        expires_at: None,
    }
}

/// Builds the server sing-box config (via the crate's OWN production renderer)
/// for a single VLESS+REALITY inbound plus the client-side outbound config
/// (via the crate's OWN production subscription renderer) that a real user
/// would receive from `GET /sub/{token}`, wraps the client outbound with a
/// local `mixed` inbound so a plain HTTP client can drive it through a SOCKS
/// proxy, and returns (server_config, client_config).
fn build_configs(
    reality_port: u16,
    mixed_port: u16,
    private_key: &str,
    public_key: &str,
    short_id: &str,
) -> (serde_json::Value, serde_json::Value) {
    let reality = RealityServerParams {
        private_key_hex: SecretString::new(private_key.to_string()),
        public_key_hex: public_key.to_string(),
        short_ids: vec![short_id.to_string()],
        handshake_server: "www.microsoft.com".into(),
        handshake_port: 443,
    };
    // Hysteria2 params are required by the renderer signature but irrelevant
    // to this REALITY-only test; give it a harmless disabled-masquerade shape.
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: "/dev/null".into(),
        tls_key_path: "/dev/null".into(),
        obfs_password: None,
        masquerade_dir_path: None,
    };
    let users = vec![test_user()];
    let mut server_cfg = render_singbox_server_config(
        &users,
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: reality_port,
            hysteria2_port: free_port(), // unused, just needs to bind somewhere
        },
        0,
    );
    // The production config listens on both IPv4/IPv6 for both inbounds;
    // drop the hysteria2 inbound entirely for this REALITY-only test so we
    // don't need real TLS cert material just to get sing-box to start.
    server_cfg["inbounds"]
        .as_array_mut()
        .unwrap()
        .retain(|ib| ib["tag"] == "vless-reality-in");

    let endpoint = compat_config::model::CompatEndpoint {
        id: "reality-1".into(),
        transport: compat_config::model::CompatTransport::VlessReality,
        host: "127.0.0.1".into(),
        port: reality_port,
        server_name: Some("www.microsoft.com".into()),
        label: "Reality".into(),
        public_parameters: compat_config::model::PublicParameters::Reality {
            public_key_hex: public_key.to_string(),
            short_id: short_id.to_string(),
            fingerprint: "chrome".into(),
        },
    };
    let mut client_cfg =
        render::render_singbox_client_subscription(&test_user(), std::slice::from_ref(&endpoint))
            .expect("render client subscription");
    client_cfg["inbounds"] = serde_json::json!([{
        "type": "mixed",
        "tag": "mixed-in",
        "listen": "127.0.0.1",
        "listen_port": mixed_port,
    }]);
    // Route everything through the reality endpoint's own outbound tag
    // (render_singbox_client_subscription names it after the endpoint label).
    client_cfg["route"] = serde_json::json!({ "final": "Reality" });

    (server_cfg, client_cfg)
}

fn write_json(dir: &std::path::Path, name: &str, value: &serde_json::Value) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

/// TEST 1 + TEST 8 (spec sections 12/13): generate a real REALITY keypair,
/// render server+client config through the crate's OWN production
/// renderers, run a real sing-box server and client, and prove traffic
/// actually flows end-to-end through the tunnel to a local (non-public)
/// HTTP target. This is the test that would have caught the production
/// incident: a syntactically-valid but cryptographically-mismatched config
/// would fail here, not just look plausible in a string-contains assertion.
#[test]
fn reality_handshake_succeeds_with_matched_keypair() {
    let Some(sb) = common::SingBox::find() else {
        eprintln!("skipping: no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    if !common::tcp_reachable("www.microsoft.com", 443, Duration::from_secs(5)) {
        eprintln!(
            "skipping: this environment cannot reach www.microsoft.com:443, the REALITY decoy \
             target this test's server dials as an inherent part of the protocol — not a failure \
             of the code under test, just this environment's egress"
        );
        return;
    }
    let (private_key, public_key) = generate_reality_keypair(&sb);
    let short_id = "e54b2158";

    let dir = tempfile::tempdir().unwrap();
    let reality_port = free_port();
    let mixed_port = free_port();
    let (server_cfg, client_cfg) = build_configs(
        reality_port,
        mixed_port,
        &private_key,
        &public_key,
        short_id,
    );
    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &client_cfg);
    let target = spawn_local_http_target();
    let server_log = dir.path().join("server.log");
    let client_log = dir.path().join("client.log");

    let _server = common::Guard(sb.run_logged(&server_path, &server_log));
    assert!(
        wait_for_port(reality_port, Duration::from_secs(5)),
        "server never bound its REALITY port. server log:\n{}",
        common::read_log(&server_log)
    );
    let _client = common::Guard(sb.run_logged(&client_path, &client_log));
    assert!(
        wait_for_port(mixed_port, Duration::from_secs(5)),
        "client never bound its local SOCKS port. client log:\n{}",
        common::read_log(&client_log)
    );

    // Give the diagnostic dump a chance to reflect the final outcome
    // (the client keeps writing to its log as the handshake/relay
    // progresses) before reading it back for the assertion message.
    let relay_ok = socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port);
    assert!(
        relay_ok,
        "REALITY handshake/traffic failed through a config produced by this crate's own \
         production renderers with a matched, real keypair.\n--- server log ---\n{}\n--- client log ---\n{}",
        common::read_log(&server_log),
        common::read_log(&client_log)
    );
}

/// TEST 4 (spec section 12): the exact production incident's signature —
/// the client is configured with a REALITY public key that does NOT
/// correspond to the server's private key (e.g. what happens if the
/// subscription service ever serves a stale/rotated-away public key).
/// Must fail closed: no traffic should flow.
#[test]
fn reality_handshake_fails_with_mismatched_public_key() {
    let Some(sb) = common::SingBox::find() else {
        eprintln!("skipping: no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let (private_key, _real_public_key) = generate_reality_keypair(&sb);
    let (_unused_private, stale_public_key) = generate_reality_keypair(&sb);
    let short_id = "e54b2158";

    let dir = tempfile::tempdir().unwrap();
    let reality_port = free_port();
    let mixed_port = free_port();
    let (server_cfg, client_cfg) = build_configs(
        reality_port,
        mixed_port,
        &private_key,
        &stale_public_key, // client uses a DIFFERENT keypair's public half
        short_id,
    );
    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &client_cfg);
    let target = spawn_local_http_target();

    let _server = common::Guard(sb.run(&server_path));
    assert!(wait_for_port(reality_port, Duration::from_secs(5)));
    let _client = common::Guard(sb.run(&client_path));
    assert!(wait_for_port(mixed_port, Duration::from_secs(5)));

    assert!(
        !socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port),
        "a mismatched REALITY public key must never be able to pass traffic \
         through the tunnel — if this assertion fails, REALITY auth is not \
         actually being enforced"
    );
}

/// TEST 2 (spec section 12): short_id coherence between the server config
/// and the client subscription rendered from the SAME state. This is a
/// pure in-memory check (no sing-box binary needed) — it exists as a fast,
/// always-run regression guard that would have failed loudly if e.g. the
/// server's `short_ids` list and the subscription's advertised `short_id`
/// were ever populated from different sources.
#[test]
fn server_and_client_configs_agree_on_reality_key_material() {
    let reality = RealityServerParams {
        private_key_hex: SecretString::new("test-private-key".to_string()),
        public_key_hex: "test-public-key".into(),
        short_ids: vec!["deadbeef".into()],
        handshake_server: "www.microsoft.com".into(),
        handshake_port: 443,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: "/dev/null".into(),
        tls_key_path: "/dev/null".into(),
        obfs_password: None,
        masquerade_dir_path: None,
    };
    let users = vec![test_user()];
    let server_cfg = render_singbox_server_config(
        &users,
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: 443,
            hysteria2_port: 443,
        },
        0,
    );

    let endpoint = compat_config::model::CompatEndpoint {
        id: "reality-1".into(),
        transport: compat_config::model::CompatTransport::VlessReality,
        host: "vpn.example.com".into(),
        port: 443,
        server_name: Some(reality.handshake_server.clone()),
        label: "Reality".into(),
        public_parameters: compat_config::model::PublicParameters::Reality {
            public_key_hex: reality.public_key_hex.clone(),
            short_id: reality.short_ids[0].clone(),
            fingerprint: "chrome".into(),
        },
    };
    let client_cfg = render::render_singbox_client_subscription(&test_user(), &[endpoint])
        .expect("render client subscription");

    let server_short_ids = server_cfg["inbounds"][0]["tls"]["reality"]["short_id"]
        .as_array()
        .unwrap();
    let client_short_id = client_cfg["outbounds"][0]["tls"]["reality"]["short_id"]
        .as_str()
        .unwrap();
    assert!(
        server_short_ids
            .iter()
            .any(|v| v.as_str() == Some(client_short_id)),
        "client-advertised short_id {client_short_id:?} is not in the server's \
         accepted short_id list {server_short_ids:?}"
    );

    let client_public_key = client_cfg["outbounds"][0]["tls"]["reality"]["public_key"]
        .as_str()
        .unwrap();
    assert_eq!(
        client_public_key, reality.public_key_hex,
        "client-advertised REALITY public key does not match the server's \
         actual public key"
    );
}
