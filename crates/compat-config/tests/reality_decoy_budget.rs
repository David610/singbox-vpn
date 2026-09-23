//! Regression test for the REALITY decoy certificate-flight limitation that
//! affected the previous sing-box 1.13.x pin.
//!
//! On 1.13.19 a decoy whose TLS Certificate record exceeded the historical
//! 8192-byte REALITY budget authenticated the client but then aborted the
//! hijack as `REALITY: processed invalid connection`. That made ordinary
//! third-party decoy certificate growth capable of breaking every user.
//!
//! sing-box 1.14.1 no longer exhibits that failure in the real-binary interop
//! harness: the same deliberately large (~16 KiB Certificate record) decoy
//! completes the tunnel after REALITY authentication succeeds. Keep this test
//! as a forward regression guard. If a future data-plane update reintroduces
//! the old record-size failure, CI must catch it before release.

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
        vision_off_experiment: false,
        google_egress_hairpin: false,
        peer_credentials: Default::default(),
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

/// A certificate flight that broke the historical 1.13.x REALITY path must
/// remain usable on the current data plane, with authentication completing.
#[test]
fn historically_oversized_decoy_certificate_remains_usable() {
    let Some(sb) = common::SingBox::find() else {
        if std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok() {
            panic!("SINGBOX_VPN_REQUIRE_REAL_INTEROP is set but no sing-box binary is available");
        }
        eprintln!("skipping: no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let Some(decoy) = common::spawn_local_tls13_decoy(DecoyCertSize::OverBudget) else {
        if std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok() {
            panic!(
                "SINGBOX_VPN_REQUIRE_REAL_INTEROP is set but the local TLS decoy would not start"
            );
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
        google_egress_hairpin_uuid: None,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: "/dev/null".into(),
        tls_key_path: "/dev/null".into(),
        obfs_password: None,
        masquerade_dir_path: None,
        up_mbps: None,
        down_mbps: None,
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
        ..Default::default()
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
        relay_ok,
        "the historically over-budget decoy certificate regressed to breaking REALITY; \
         1.14.1 is expected to carry this large certificate flight successfully.\n\
         --- server log ---\n{server_log_text}"
    );
    assert!(
        saw_auth_ok,
        "traffic flowed without the expected REALITY-authenticated handshake marker.\n\
         --- server log ---\n{server_log_text}"
    );
    assert!(
        !saw_invalid,
        "the historically over-budget decoy unexpectedly produced \
         `processed invalid connection` again.\n--- server log ---\n{server_log_text}"
    );
}
