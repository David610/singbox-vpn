//! Regression test for the REALITY decoy record-size budget — the confirmed
//! root cause of the recurring `singbox-validate` CI failure.
//!
//! ## The mechanism
//!
//! sing-box's REALITY server (via `github.com/metacubex/utls`, `reality.go`)
//! relays the client's ClientHello to the configured `handshake_server`
//! ("decoy"), then walks the decoy's TLS response record by record to decide
//! whether it can hijack the handshake. That walk is bounded:
//!
//! ```text
//! const realitySize uint16 = 8192
//! ...
//! if handshakeLen > int(realitySize) { break f }
//! ```
//!
//! `handshakeLen` is `5 + <wire record length>`, read from the 5-byte TLS
//! record header — REALITY never decrypts the decoy's flight, it only
//! measures records and names them positionally. If ANY record in the
//! decoy's response exceeds 8192 bytes, the walk aborts, `hs.handshake()` is
//! never called, `isHandshakeComplete` stays false, and the server returns:
//!
//! ```text
//! REALITY: processed invalid connection
//! ```
//!
//! ## Why this test exists
//!
//! That error string was misread three separate times as evidence of a
//! REALITY key/short_id mismatch, producing three commits each claiming a
//! different "root cause" (external egress blocked, a urltest probe racing,
//! an IPv6 decoy dial) — none of which was correct. The string is emitted
//! for ANY connection that fails to complete the hijack, including a
//! perfectly authenticated client whose decoy simply returned an oversized
//! certificate.
//!
//! So this test pins the semantics: with a decoy whose certificate flight
//! exceeds the budget, traffic must fail **while REALITY authentication
//! succeeds**. Asserting `hs.c.conn == conn: true` in the failing case is
//! the whole point — it is what makes the diagnosis unambiguous next time.
//!
//! It is also the test that would have caught the production exposure: the
//! shipped default `handshake_server` is a third-party CDN whose record
//! framing this project does not control.

mod common;

use common::{
    free_port, socks5_http_get_is_200, spawn_local_http_target, wait_for_log_line, DecoyCertSize,
};
use compat_config::model::{Hysteria2ServerParams, RealityServerParams};
use compat_config::secret::SecretString;
use compat_config::server::{render_singbox_server_config, ServerPorts};
use compat_config::{render, CompatUser};
use std::process::Command;
use std::time::Duration;

fn test_user() -> CompatUser {
    CompatUser {
        id: "u1".into(),
        name: "decoy-budget-test".into(),
        enabled: true,
        vless_uuid: "841cac4a-efe4-48ac-92b8-d11f4c98c45e".into(),
        hysteria2_password: SecretString::new("unused-in-this-test"),
        subscription_token_hash_hex: "unused".into(),
        created_at: 0,
        expires_at: None,
    }
}

fn generate_reality_keypair(sb: &common::SingBox) -> (String, String) {
    let output = Command::new(&sb.path)
        .arg("generate")
        .arg("reality-keypair")
        .output()
        .expect("run sing-box generate reality-keypair");
    assert!(output.status.success(), "reality-keypair generation failed");
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let line = line.trim();
            line.strip_prefix(&format!("{name}:"))
                .or_else(|| line.strip_prefix(&format!("{name} ")))
                .map(|s| s.trim().to_string())
        })
    };
    (
        field("PrivateKey").expect("PrivateKey"),
        field("PublicKey").expect("PublicKey"),
    )
}

/// A decoy whose TLS certificate flight exceeds REALITY's 8192-byte record
/// budget must break the tunnel — and must do so with REALITY authentication
/// having SUCCEEDED, so the failure is never again misattributed to the key
/// material.
#[test]
fn oversized_decoy_certificate_breaks_the_tunnel_even_though_reality_auth_succeeds() {
    let Some(sb) = common::SingBox::find() else {
        if std::env::var("VPN1_REQUIRE_REAL_INTEROP").is_ok() {
            panic!("VPN1_REQUIRE_REAL_INTEROP is set but no sing-box binary is available");
        }
        eprintln!("skipping: no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let Some(decoy) = common::spawn_local_tls13_decoy(DecoyCertSize::OverBudget) else {
        if std::env::var("VPN1_REQUIRE_REAL_INTEROP").is_ok() {
            panic!("VPN1_REQUIRE_REAL_INTEROP is set but the local TLS decoy would not start");
        }
        eprintln!("skipping: could not start the local TLS 1.3 decoy (openssl missing?)");
        return;
    };

    let (private_key, public_key) = generate_reality_keypair(&sb);
    let short_id = "e54b2158";
    let reality_port = free_port();
    let mixed_port = free_port();
    let target = spawn_local_http_target();

    let reality = RealityServerParams {
        private_key_hex: SecretString::new(private_key),
        public_key_hex: public_key.clone(),
        short_ids: vec![short_id.to_string()],
        handshake_server: decoy.hostname.to_string(),
        handshake_port: decoy.port,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: "/dev/null".into(),
        tls_key_path: "/dev/null".into(),
        obfs_password: None,
        masquerade_dir_path: None,
    };
    let mut server_cfg = render_singbox_server_config(
        &[test_user()],
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: reality_port,
            hysteria2_port: free_port(),
        },
        0,
    );
    server_cfg["inbounds"]
        .as_array_mut()
        .unwrap()
        .retain(|ib| ib["tag"] == "vless-reality-in");
    server_cfg["log"] = serde_json::json!({ "level": "trace", "timestamp": true });

    let endpoint = compat_config::model::CompatEndpoint {
        id: "reality-1".into(),
        transport: compat_config::model::CompatTransport::VlessReality,
        host: "127.0.0.1".into(),
        port: reality_port,
        server_name: Some(decoy.hostname.to_string()),
        label: "Reality".into(),
        public_parameters: compat_config::model::PublicParameters::Reality {
            public_key_hex: public_key,
            short_id: short_id.to_string(),
            fingerprint: "chrome".into(),
        },
    };
    let mut client_cfg =
        render::render_singbox_client_subscription(&test_user(), std::slice::from_ref(&endpoint))
            .expect("render client subscription");
    client_cfg["inbounds"] = serde_json::json!([{
        "type": "mixed", "tag": "mixed-in", "listen": "127.0.0.1", "listen_port": mixed_port,
    }]);
    let probe_url = format!("http://127.0.0.1:{}/", target.port);
    for ob in client_cfg["outbounds"].as_array_mut().unwrap() {
        if ob["type"] == "urltest" {
            ob["url"] = serde_json::json!(probe_url);
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let server_path = dir.path().join("server.json");
    let client_path = dir.path().join("client.json");
    std::fs::write(
        &server_path,
        serde_json::to_vec_pretty(&server_cfg).unwrap(),
    )
    .unwrap();
    std::fs::write(
        &client_path,
        serde_json::to_vec_pretty(&client_cfg).unwrap(),
    )
    .unwrap();
    let server_log = dir.path().join("server.log");
    let client_log = dir.path().join("client.log");

    let _server = common::Guard(sb.run_logged(&server_path, &server_log));
    assert!(wait_for_log_line(
        &server_log,
        "sing-box started",
        Duration::from_secs(10)
    ));
    let _client = common::Guard(sb.run_logged(&client_path, &client_log));
    assert!(wait_for_log_line(
        &client_log,
        "sing-box started",
        Duration::from_secs(10)
    ));

    let relay_ok = socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port);
    // sing-box writes its log asynchronously, so a single read right after
    // the relay attempt can race the trace lines this test asserts on. Wait
    // for the line instead of sampling once — the asserted condition is
    // unchanged, only the harness's observation of it is made reliable.
    let saw_auth_ok = wait_for_log_line(
        &server_log,
        "hs.c.conn == conn: true",
        Duration::from_secs(5),
    );
    let saw_invalid = wait_for_log_line(
        &server_log,
        "processed invalid connection",
        Duration::from_secs(5),
    );
    let server_log_text = common::read_log(&server_log);

    assert!(
        !relay_ok,
        "an over-budget decoy certificate was expected to break the tunnel, but traffic \
         flowed. If sing-box raised `realitySize` above 8192, this test and the \
         `DecoyUnsuitable` diagnosis in `vpn-admin doctor` both need revisiting.\n\
         --- server log ---\n{server_log_text}"
    );

    // THE POINT OF THIS TEST. The tunnel is broken, and yet REALITY
    // authentication succeeded — so `processed invalid connection` here says
    // nothing whatsoever about the key material.
    assert!(
        saw_auth_ok,
        "expected REALITY authentication to SUCCEED and the failure to come from the \
         decoy's oversized record; instead auth itself did not complete, so this test \
         is no longer reproducing the documented mechanism.\n--- server log ---\n{server_log_text}"
    );
    assert!(
        saw_invalid,
        "expected the over-budget decoy to produce REALITY's `processed invalid \
         connection`.\n--- server log ---\n{server_log_text}"
    );
}
