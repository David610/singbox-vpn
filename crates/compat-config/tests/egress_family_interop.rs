//! Unprivileged local real-core controls. No live VPS or native-device evidence.
mod common;

#[test]
fn real_core_literal_family_and_proxy_recovery_boundaries() {
    use compat_config::deployment::DeploymentConfig;
    use compat_config::model::{Hysteria2ServerParams, RealityServerParams};
    use compat_config::secret::SecretString;
    use compat_config::server::render_server_config_for_deployment;
    let Some(sb) = common::SingBox::find() else {
        assert!(
            std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_err(),
            "real sing-box required"
        );
        eprintln!(
            "SKIP: egress_family_interop requires real sing-box and IPv6 loopback; not acceptance"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for policy in ["allow", "reject_google", "reject_all"] {
        let text = format!("public_host=\"example.test\"\nsubscription_host=\"example.test\"\n[reality]\nlisten_port=443\nhandshake_server=\"localhost\"\n[hysteria2]\nlisten_port=443\n[subscription]\nlisten_port=9100\n[egress]\nipv6_literal={policy:?}\n");
        let cfg: DeploymentConfig = toml::from_str(&text).unwrap();
        let user: compat_config::model::CompatUser = serde_json::from_value(serde_json::json!({
            "id":"lab", "name":"lab", "enabled":true,
            "vless_uuid":"11111111-1111-4111-8111-111111111111",
            "hysteria2_password":"DISPOSABLE-LOCAL-FIXTURE", "subscription_token_hash_hex":"unused", "created_at":0
        })).unwrap();
        let reality = RealityServerParams {
            private_key_hex: SecretString::new("replaced-by-lab"),
            public_key_hex: String::new(),
            short_ids: vec!["0a1b2c3d".into()],
            handshake_server: "localhost".into(),
            handshake_port: 443,
            google_egress_hairpin_uuid: None,
        };
        let hysteria = Hysteria2ServerParams {
            tls_cert_path: "replaced-by-lab".into(),
            tls_key_path: "replaced-by-lab".into(),
            obfs_password: None,
            masquerade_dir_path: None,
            up_mbps: None,
            down_mbps: None,
        };
        let doc =
            render_server_config_for_deployment(&cfg, &[user], &reality, &hysteria, 1).unwrap();
        std::fs::write(
            dir.path().join(format!("{policy}.json")),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut binary = sb.path.clone();
    // The cloud's localhost-only helper is unnecessary here: the rig supplies
    // its own controlled DNS and uses the same unmodified upstream binary.
    if binary.file_name().is_some_and(|n| n == "sing-box-local") {
        binary = binary.with_file_name("sing-box");
    }
    let python = std::env::var("YOUTUBE_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let output = std::process::Command::new(python)
        .arg(root.join("deploy/lib/egress_family_lab.py"))
        .arg("--sing-box")
        .arg(binary)
        .arg("--configs-dir")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "local real-core lab failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprint!("{}", String::from_utf8_lossy(&output.stdout));
}
