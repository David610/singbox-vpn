//! Real interoperability tests for Hysteria2 config generation — the
//! Hysteria2 counterpart to `reality_interop.rs`. Builds the exact server
//! config (`server::render_singbox_server_config`) and client config
//! (`render::render_singbox_client_subscription`) this codebase would
//! actually deploy/serve, drives a REAL `sing-box` binary as both server
//! and client over loopback (UDP/QUIC), and proves a real password-
//! authenticated tunnel actually relays traffic to a local (non-public)
//! HTTP target.
//!
//! Skipped (not failed) when no usable `sing-box` binary is available —
//! see `reality_interop.rs`'s module doc for the same `SING_BOX_BIN`/CI
//! wiring notes; this file follows the identical convention.

mod common;

use common::{free_port, socks5_http_get_is_200, spawn_local_http_target, wait_for_port};
use compat_config::model::{Hysteria2ServerParams, RealityServerParams};
use compat_config::secret::SecretString;
use compat_config::server::{render_singbox_server_config, ServerPorts};
use compat_config::{render, CompatUser};
use std::time::Duration;

/// Generates a throwaway self-signed EC certificate via the `openssl`
/// CLI (already a hard runtime dependency of this project — see
/// `deploy/almalinux/install.sh`'s `openssl x509` cert-expiry checks —
/// so no new Rust dependency is pulled in just for test fixtures).
/// Skips (does not fail) the calling test if `openssl` isn't on PATH,
/// same "environment can't run this, don't fake a result" policy as the
/// `sing-box`-availability gate.
fn generate_self_signed_cert(
    dir: &std::path::Path,
    cn: &str,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    if std::process::Command::new("openssl")
        .arg("version")
        .output()
        .is_err()
    {
        return None;
    }
    let key = dir.join("hysteria2-test-key.pem");
    let cert = dir.join("hysteria2-test-cert.pem");
    // `-addext subjectAltName=...` is required: modern Go (sing-box's
    // client TLS verification) rejects a certificate that only has a
    // legacy CN with no SAN ("x509: certificate relies on legacy Common
    // Name field, use SANs instead") — a bare `-subj "/CN=..."` alone
    // produces exactly that rejected shape.
    let status = std::process::Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "ed25519", "-days", "1", "-nodes", "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .args(["-subj", &format!("/CN={cn}")])
        .args(["-addext", &format!("subjectAltName=DNS:{cn}")])
        .status()
        .expect("run openssl to generate a throwaway test certificate");
    if !status.success() {
        return None;
    }
    Some((cert, key))
}

fn test_user(hysteria2_password: &str) -> CompatUser {
    CompatUser {
        id: "u1".into(),
        name: "interop-test".into(),
        enabled: true,
        vless_uuid: "841cac4a-efe4-48ac-92b8-d11f4c98c45e".into(),
        hysteria2_password: SecretString::new(hysteria2_password.to_string()),
        subscription_token_hash_hex: "unused".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
    }
}

/// Builds the server sing-box config (via the crate's OWN production
/// renderer) for a single Hysteria2 inbound, plus the client-side
/// outbound config (via the crate's OWN production subscription
/// renderer) that a real user would receive from `GET /sub/{token}`.
/// Test-only scaffolding added ON TOP of the production output (never
/// replacing it): a local `mixed` inbound so a plain client can drive
/// the tunnel via SOCKS, a `route` override, and — since the test cert
/// is self-signed and the production renderer correctly never sets
/// `insecure: true` — a client-side `certificate_path` pin to trust
/// THIS SPECIFIC throwaway cert, which is the standard way to test
/// against a private CA without weakening the renderer under test.
#[allow(clippy::too_many_arguments)]
fn build_configs(
    hysteria_port: u16,
    mixed_port: u16,
    password: &str,
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
    sni: &str,
    server_obfs_password: Option<&str>,
    client_obfs_password: Option<&str>,
) -> (serde_json::Value, serde_json::Value) {
    // REALITY params are required by the renderer signature but
    // irrelevant to this Hysteria2-only test.
    let reality = RealityServerParams {
        private_key_hex: SecretString::new("unused".to_string()),
        public_key_hex: "unused".into(),
        short_ids: vec!["00000000".into()],
        handshake_server: "www.google.com".into(),
        handshake_port: 443,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: cert_path.display().to_string(),
        tls_key_path: key_path.display().to_string(),
        obfs_password: server_obfs_password.map(|s| SecretString::new(s.to_string())),
        masquerade_dir_path: None,
        up_mbps: None,
        down_mbps: None,
    };
    let users = vec![test_user(password)];
    let mut server_cfg = render_singbox_server_config(
        &users,
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: free_port(), // unused, just needs to bind somewhere
            hysteria2_port: hysteria_port,
        },
        0,
    );
    // Drop the vless-reality inbound entirely — this test only needs
    // Hysteria2 to start, and REALITY needs real handshake-target
    // egress this test doesn't want as a dependency.
    server_cfg["inbounds"]
        .as_array_mut()
        .unwrap()
        .retain(|ib| ib["tag"] == "hysteria2-in");

    let endpoint = compat_config::model::CompatEndpoint {
        id: "hysteria2-1".into(),
        transport: compat_config::model::CompatTransport::Hysteria2,
        host: "127.0.0.1".into(),
        port: hysteria_port,
        server_name: Some(sni.into()),
        label: "Hysteria2".into(),
        public_parameters: compat_config::model::PublicParameters::Hysteria2 {
            obfs_password: client_obfs_password.map(|s| s.to_string()),
        },
    };
    let mut client_cfg = render::render_singbox_client_subscription(
        &test_user(password),
        std::slice::from_ref(&endpoint),
    )
    .expect("render client subscription");
    client_cfg["inbounds"] = serde_json::json!([{
        "type": "mixed",
        "tag": "mixed-in",
        "listen": "127.0.0.1",
        "listen_port": mixed_port,
    }]);
    // Test-only trust pin — see this function's doc comment. Must be
    // applied before the urltest/direct outbounds are dropped below,
    // since it addresses outbounds[0] by its production-rendered index.
    client_cfg["outbounds"][0]["tls"]["certificate_path"] =
        serde_json::json!(cert_path.display().to_string());
    client_cfg["route"] = serde_json::json!({ "final": "Hysteria2" });
    // Same fix as reality_interop.rs's build_configs: drop the
    // production renderer's automatic `urltest` "auto" selector, which
    // otherwise fires its own immediate, uncontrolled health-check
    // connection through this outbound at sing-box startup — confirmed
    // as a real CI race for the REALITY counterpart of this test, and
    // not something this test needs, so removed here too as defense in
    // depth rather than waiting to observe the same flake here.
    client_cfg["outbounds"]
        .as_array_mut()
        .unwrap()
        .retain(|ob| ob["tag"] == "Hysteria2");

    (server_cfg, client_cfg)
}

fn write_json(dir: &std::path::Path, name: &str, value: &serde_json::Value) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

/// Shared skip policy — see `reality_interop.rs`. A missing `sing-box` or
/// `openssl` must not block a contributor, but an early `return` is reported
/// by Rust's harness as a PASS, so the pipeline that gates merges sets
/// `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1` and this turns every skip into a failure.
/// A skipped test is not a pass.
fn skip_or_fail(reason: &str) {
    if std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok() {
        panic!(
            "SINGBOX_VPN_REQUIRE_REAL_INTEROP is set, so this suite must really run, but: {reason}. \
             Refusing to report a skip as a pass."
        );
    }
    eprintln!("skipping: {reason}");
}

/// Real end-to-end Hysteria2: generate a throwaway TLS cert, render
/// server+client config through the crate's OWN production renderers
/// with a matching password, run a real sing-box server and client over
/// UDP/QUIC, and prove traffic actually flows to a local HTTP target.
#[test]
fn hysteria2_handshake_succeeds_with_matched_password() {
    let Some(sb) = common::SingBox::find() else {
        skip_or_fail("no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some((cert, key)) = generate_self_signed_cert(dir.path(), "hysteria2-test.invalid") else {
        skip_or_fail("openssl not available to generate a test certificate");
        return;
    };

    let hysteria_port = free_port();
    let mixed_port = free_port();
    let password = "test-password-not-a-real-secret";
    let (server_cfg, client_cfg) = build_configs(
        hysteria_port,
        mixed_port,
        password,
        &cert,
        &key,
        "hysteria2-test.invalid",
        None,
        None,
    );
    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &client_cfg);
    let target = spawn_local_http_target();
    let server_log = dir.path().join("server.log");
    let client_log = dir.path().join("client.log");

    let _server = common::Guard(sb.run_logged(&server_path, &server_log));
    // Hysteria2 is UDP/QUIC — `wait_for_port`'s TCP connect can't
    // observe it coming up, so give sing-box a moment to bind instead.
    std::thread::sleep(Duration::from_millis(500));
    let _client = common::Guard(sb.run_logged(&client_path, &client_log));
    assert!(
        wait_for_port(mixed_port, Duration::from_secs(5)),
        "client never bound its local SOCKS port. client log:\n{}",
        common::read_log(&client_log)
    );

    assert!(
        socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port),
        "Hysteria2 handshake/traffic failed through a config produced by this crate's own \
         production renderers with a matched password and pinned test certificate.\n\
         --- server log ---\n{}\n--- client log ---\n{}",
        common::read_log(&server_log),
        common::read_log(&client_log)
    );
}

/// The Hysteria2 counterpart to `reality_handshake_fails_with_mismatched_public_key`:
/// a client configured with the WRONG password (e.g. what a stale
/// subscription would serve after a `rotate-hysteria` the running
/// subscription process never picked up) must fail closed — no traffic
/// should flow.
#[test]
fn hysteria2_handshake_fails_with_wrong_password() {
    let Some(sb) = common::SingBox::find() else {
        skip_or_fail("no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some((cert, key)) = generate_self_signed_cert(dir.path(), "hysteria2-test.invalid") else {
        skip_or_fail("openssl not available to generate a test certificate");
        return;
    };

    let hysteria_port = free_port();
    let mixed_port = free_port();
    // Server keeps "correct-password"; only the CLIENT side is rebuilt
    // with a different password — simulating a stale/mismatched
    // credential (e.g. a subscription served after `rotate-hysteria`
    // that the running process never picked up), not a config typo.
    let (server_cfg, _matched_client_cfg) = build_configs(
        hysteria_port,
        mixed_port,
        "correct-password",
        &cert,
        &key,
        "hysteria2-test.invalid",
        None,
        None,
    );
    let (_unused_server_cfg, mismatched_client_cfg) = build_configs(
        hysteria_port,
        mixed_port,
        "a-completely-different-password",
        &cert,
        &key,
        "hysteria2-test.invalid",
        None,
        None,
    );

    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &mismatched_client_cfg);
    let target = spawn_local_http_target();

    let _server = common::Guard(sb.run(&server_path));
    std::thread::sleep(Duration::from_millis(500));
    let _client = common::Guard(sb.run(&client_path));
    assert!(
        wait_for_port(mixed_port, Duration::from_secs(5)),
        "client never bound its local SOCKS port"
    );

    assert!(
        !socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port),
        "a mismatched Hysteria2 password must never be able to pass traffic through the \
         tunnel — if this assertion fails, Hysteria2 auth is not actually being enforced"
    );
}

/// Real end-to-end Hysteria2 WITH salamander obfuscation enabled on both
/// sides — the production default for a fresh `vpn-admin init` (see
/// `apps/admin/src/main.rs::cmd_init`) and the mechanism `vpn-admin
/// hysteria-obfs-rotate` enables on an existing deployment. Proves the
/// obfuscation layer this crate's own production renderers wire up
/// (`server.rs`'s `tls.reality`-sibling `obfs` block, `render.rs`'s
/// `obfs=salamander` URI param / native `obfs` JSON) actually
/// interoperates with a real sing-box binary end to end, not just that
/// the JSON shape looks plausible.
#[test]
fn hysteria2_obfuscated_handshake_succeeds_with_matched_obfs_password() {
    let Some(sb) = common::SingBox::find() else {
        skip_or_fail("no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some((cert, key)) = generate_self_signed_cert(dir.path(), "hysteria2-test.invalid") else {
        skip_or_fail("openssl not available to generate a test certificate");
        return;
    };

    let hysteria_port = free_port();
    let mixed_port = free_port();
    let password = "test-password-not-a-real-secret";
    let obfs_password = "test-obfs-password-not-a-real-secret";
    let (server_cfg, client_cfg) = build_configs(
        hysteria_port,
        mixed_port,
        password,
        &cert,
        &key,
        "hysteria2-test.invalid",
        Some(obfs_password),
        Some(obfs_password),
    );
    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &client_cfg);
    let target = spawn_local_http_target();
    let server_log = dir.path().join("server.log");
    let client_log = dir.path().join("client.log");

    let _server = common::Guard(sb.run_logged(&server_path, &server_log));
    std::thread::sleep(Duration::from_millis(500));
    let _client = common::Guard(sb.run_logged(&client_path, &client_log));
    assert!(
        wait_for_port(mixed_port, Duration::from_secs(5)),
        "client never bound its local SOCKS port. client log:\n{}",
        common::read_log(&client_log)
    );

    assert!(
        socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port),
        "obfuscated Hysteria2 handshake/traffic failed through a config produced by this \
         crate's own production renderers with matched auth AND obfs passwords.\n\
         --- server log ---\n{}\n--- client log ---\n{}",
        common::read_log(&server_log),
        common::read_log(&client_log)
    );
}

/// A client whose obfs password doesn't match the server's (e.g. an
/// operator forgot to re-import after `hysteria-obfs-rotate`, or a
/// downgrade attack tries connecting un-obfuscated to an obfuscation-
/// enabled server) must fail closed. Salamander is applied before the
/// QUIC handshake even begins, so this is stricter than an auth failure:
/// the server should not even recognize the wire traffic as Hysteria2.
#[test]
fn hysteria2_handshake_fails_with_wrong_obfs_password() {
    let Some(sb) = common::SingBox::find() else {
        skip_or_fail("no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some((cert, key)) = generate_self_signed_cert(dir.path(), "hysteria2-test.invalid") else {
        skip_or_fail("openssl not available to generate a test certificate");
        return;
    };

    let hysteria_port = free_port();
    let mixed_port = free_port();
    let (server_cfg, _matched_client_cfg) = build_configs(
        hysteria_port,
        mixed_port,
        "correct-password",
        &cert,
        &key,
        "hysteria2-test.invalid",
        Some("server-obfs-password"),
        Some("server-obfs-password"),
    );
    let (_unused_server_cfg, mismatched_client_cfg) = build_configs(
        hysteria_port,
        mixed_port,
        "correct-password",
        &cert,
        &key,
        "hysteria2-test.invalid",
        Some("server-obfs-password"),
        Some("a-completely-different-obfs-password"),
    );

    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &mismatched_client_cfg);
    let target = spawn_local_http_target();

    let _server = common::Guard(sb.run(&server_path));
    std::thread::sleep(Duration::from_millis(500));
    let _client = common::Guard(sb.run(&client_path));
    // The client may not even bind its local port cleanly, or may bind it
    // and simply pass no traffic — either is an acceptable "fails closed"
    // outcome for a mismatched obfuscation layer, so this test asserts on
    // traffic never flowing, not on the bind step.
    std::thread::sleep(Duration::from_secs(2));

    assert!(
        !socks5_http_get_is_200(mixed_port, "127.0.0.1", target.port),
        "a mismatched Hysteria2 obfuscation password must never be able to pass traffic \
         through the tunnel — if this assertion fails, obfuscation is not actually gating \
         wire-level access"
    );
}

/// Validates the OPTIONAL Brutal fixed-bandwidth Hysteria2 configuration
/// (`up_mbps`/`down_mbps` + `ignore_client_bandwidth` — see
/// `Hysteria2ServerParams::up_mbps`/`down_mbps` in `model.rs`, and
/// `server.rs`'s rendering of them) against the REAL pinned sing-box
/// binary via `sing-box check`. `server.rs`'s own unit tests
/// (`hysteria_bandwidth_set_together_forces_ignore_client_bandwidth`)
/// only prove this crate PRODUCES the expected JSON shape; they cannot
/// prove the exact pinned sing-box version actually ACCEPTS it as a
/// valid Hysteria2 inbound — a field-name typo or a schema change in a
/// future sing-box version would still pass those unit tests but fail
/// here, which is exactly the gap this test closes.
#[test]
fn hysteria2_brutal_bandwidth_config_passes_real_sing_box_check() {
    let Some(sb) = common::SingBox::find() else {
        skip_or_fail("no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some((cert, key)) = generate_self_signed_cert(dir.path(), "hysteria2-test.invalid") else {
        skip_or_fail("openssl not available to generate a test certificate");
        return;
    };

    let reality = RealityServerParams {
        private_key_hex: SecretString::new("unused".to_string()),
        public_key_hex: "unused".into(),
        short_ids: vec!["00000000".into()],
        handshake_server: "www.google.com".into(),
        handshake_port: 443,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: cert.display().to_string(),
        tls_key_path: key.display().to_string(),
        obfs_password: None,
        masquerade_dir_path: None,
        up_mbps: Some(100),
        down_mbps: Some(80),
    };
    let users = vec![test_user("brutal-bandwidth-test-password")];
    let mut server_cfg = render_singbox_server_config(
        &users,
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: free_port(), // unused, dropped below
            hysteria2_port: free_port(),
        },
        0,
    );
    // Isolate Hysteria2, same as build_configs above — REALITY needs a
    // real handshake-target dependency `sing-box check` (a static config
    // validation, not a live dial) doesn't need to exercise here.
    server_cfg["inbounds"]
        .as_array_mut()
        .unwrap()
        .retain(|ib| ib["tag"] == "hysteria2-in");

    // Sanity check on the test's own setup: an empty/absent Brutal
    // config is ALSO valid Hysteria2, so a passing `sing-box check`
    // alone would not prove anything about Brutal specifically unless
    // this test first confirms the fields it's validating are actually
    // present in the config being checked.
    let hy_inbound = &server_cfg["inbounds"][0];
    assert_eq!(
        hy_inbound["up_mbps"], 100,
        "test setup bug: rendered config is missing up_mbps"
    );
    assert_eq!(
        hy_inbound["down_mbps"], 80,
        "test setup bug: rendered config is missing down_mbps"
    );
    assert_eq!(
        hy_inbound["ignore_client_bandwidth"], true,
        "test setup bug: rendered config is missing ignore_client_bandwidth"
    );

    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let output = sb.check(&server_path);
    assert!(
        output.status.success(),
        "real sing-box check REJECTED a Hysteria2 inbound with \
         up_mbps/down_mbps/ignore_client_bandwidth set (pinned version: see \
         deploy/almalinux/install.sh SINGBOX_VERSION).\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
