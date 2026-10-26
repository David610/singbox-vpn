//! Regression test for the REALITY decoy certificate-flight limitation that
//! affected the previous sing-box 1.13.x pin.
//!
//! On 1.13.19 a decoy whose TLS Certificate record exceeded the historical
//! 8192-byte REALITY budget authenticated the client but then aborted the
//! hijack as `REALITY: processed invalid connection`. That made ordinary
//! third-party decoy certificate growth capable of breaking every user.
//!
//! sing-box 1.14.1 no longer exhibits that failure in the real-binary interop
//! harness: a deliberately large (~12 KiB Certificate record) decoy completes
//! the tunnel after REALITY authentication succeeds. Keep this test as a
//! forward regression guard. If a future data-plane update reintroduces the
//! old record-size failure, CI must catch it before release.
//!
//! The fixture is not larger on purpose: the current REALITY code drops a
//! connection once more than 16384 decoy bytes are buffered unparsed, so a
//! ~16 KiB Certificate record failed intermittently depending on how the
//! decoy's bytes were split across reads (see `spawn_local_tls13_decoy`).

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
        exit_principal: false,
        peer_credentials: Default::default(),
        is_reserved_probe: false,
        credentials: Vec::new(),
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

/// `loadavg` and CPU count at the moment of the failure: contention is the first thing to rule in or out for a timing-shaped flake.
fn host_load() -> String {
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_else(|_| "(unavailable)".into());
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    format!("{} cpus={cpus}", load.trim())
}

/// Diagnostic only (the assertions are unchanged): for every `processed invalid connection`, say what that
/// connection did, so a failure classifies itself instead of needing the whole trace read by hand.
/// sing-box reports ANY handshake that does not complete that way -- a client that authenticated, received the
/// whole certificate flight and then closed before sending Finished included (observed once: the `urltest` health
/// probe of the rendered subscription, abandoned 5 ms in, while the test's own relay connection succeeded).
fn diagnose_invalid_connections(log: &str) -> String {
    let mut out = Vec::new();
    for line in log
        .lines()
        .filter(|l| l.contains("processed invalid connection"))
    {
        let remote = line
            .split("process connection from ")
            .nth(1)
            .and_then(|r| r.split(':').take(2).collect::<Vec<_>>().join(":").into())
            .unwrap_or_default();
        let mine: Vec<&str> = log
            .lines()
            .filter(|l| !remote.is_empty() && l.contains(&format!("remoteAddr: {remote} ")))
            .collect();
        let has = |needle: &str| mine.iter().any(|l| l.contains(needle));
        let verdict = if has("hs.c.conn == conn: true") && has("readClientFinished() err: EOF") {
            "AUTHENTICATED, server flight sent, then the CLIENT CLOSED before Finished (client-abandoned handshake, not a server/decoy failure)"
        } else if has("hs.c.conn == conn: true") {
            "authenticated, then failed for another reason (inspect the trace)"
        } else {
            "never authenticated (wrong key/short id/time, or not a REALITY client)"
        };
        out.push(format!("connection {remote}: {verdict}"));
    }
    if out.is_empty() {
        "no `processed invalid connection` line".to_string()
    } else {
        out.join("\n")
    }
}

#[test]
fn diagnosis_recognises_a_client_abandoned_authenticated_handshake() {
    // Lines taken from the one observed failure (staging, 2026-10-21 10:40:53Z).
    let abandoned = "\
inbound/vless[vless-reality-in]: REALITY remoteAddr: 127.0.0.1:49610 hs.c.conn == conn: true
inbound/vless[vless-reality-in]: REALITY remoteAddr: 127.0.0.1:49610 hs.readClientFinished() err: EOF
ERROR [458195100 11ms] inbound/vless[vless-reality-in]: process connection from 127.0.0.1:49610: TLS handshake: REALITY: processed invalid connection";
    assert!(diagnose_invalid_connections(abandoned).contains("CLIENT CLOSED before Finished"));
    let unauth = "\
inbound/vless[vless-reality-in]: REALITY remoteAddr: 127.0.0.1:50000 hs.c.isHandshakeComplete.Load(): false
ERROR [1 1ms] inbound/vless[vless-reality-in]: process connection from 127.0.0.1:50000: TLS handshake: REALITY: processed invalid connection";
    assert!(diagnose_invalid_connections(unauth).contains("never authenticated"));
    assert!(diagnose_invalid_connections("nothing").contains("no `processed invalid connection`"));
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
    // Diagnostic only: the client's own trace log goes into the failure message, so "the client closed before Finished"
    // (I-71) can be explained from the client's side instead of guessed from the server's.
    client_cfg["log"] = serde_json::json!({ "level": "trace", "timestamp": true });
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
         `processed invalid connection` again.\n--- diagnosis ---\n{}\n--- host load (loadavg, cpus) ---\n{}\n\
         --- client log ---\n{}\n--- server log ---\n{server_log_text}",
        diagnose_invalid_connections(&server_log_text),
        host_load(),
        common::read_log(&client_log)
    );
}
