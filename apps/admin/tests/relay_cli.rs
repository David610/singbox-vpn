//! `vpn-admin` against relay and exit deployments: persisted state in, CLI
//! behaviour out. Level 2 (integration) evidence; the real-process traffic
//! proof lives in `crates/compat-config/tests/two_hop_system.rs`.

#![cfg(unix)]

use assert_cmd::Command;
use std::path::{Path, PathBuf};

mod support;

const REALITY_PRIVATE: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
const REALITY_PUBLIC: &str = "pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk";
const EXIT_PUBLIC: &str = "zo060cy2M-x7cMF4FKXHbs0CloUFDTRHRboFhw5YfVk";
/// Synthetic credential B, issued "by the exit". Must never be echoed by
/// any read-only command.
const EXIT_UUID: &str = "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1";

fn toml_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn fake_singbox(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-sing-box.sh");
    let script = format!(
        "#!/usr/bin/env bash\ncase \"$1\" in\n  generate) echo \"PrivateKey: {REALITY_PRIVATE}\"; echo \"PublicKey: {REALITY_PUBLIC}\"; exit 0 ;;\n  check) exit 0 ;;\n  version) echo 'sing-box test-fake 1.0.0'; exit 0 ;;\nesac\nexit 1\n"
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    LegacyExit,
    Exit,
    UnpairedRelay,
    PairedRelay,
}

fn write_deployment(dir: &Path, shape: Shape) -> PathBuf {
    let state = dir.join("state");
    let singbox = fake_singbox(dir);
    let (identity, host) = match shape {
        Shape::LegacyExit => ("", "de1.example.test"),
        Shape::Exit => (
            "schema_version = 2\nnode_id = \"de1\"\nrole = \"exit\"\n",
            "de1.example.test",
        ),
        Shape::UnpairedRelay | Shape::PairedRelay => (
            "schema_version = 2\nnode_id = \"ru1\"\nrole = \"relay\"\n",
            "ru1.example.test",
        ),
    };
    let mut text = format!(
        r#"{identity}public_host = "{host}"
subscription_host = "{host}"
state_dir = "{state}"
singbox_binary = "{singbox}"

[reality]
listen_port = 443
handshake_server = "www.example.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        state = toml_path(&state),
        singbox = toml_path(&singbox),
    );
    if matches!(shape, Shape::UnpairedRelay | Shape::PairedRelay) {
        text.push_str(
            "\n[[access_paths]]\nid = \"via-ru1\"\nkind = \"relay\"\nvia_endpoint_id = \"reality-1\"\ncapabilities = [\"tcp\"]\n",
        );
    }
    if shape == Shape::PairedRelay {
        for (id, tag, path, credential_ref) in [
            ("de1-direct", "Germany · Direct", "direct", ""),
            (
                "de1-via-ru1",
                "Germany · via Russia",
                "via-ru1",
                "credential_ref = \"de1-direct\"\n",
            ),
        ] {
            text.push_str(&format!(
                "\n[[peer_endpoints]]\nid = \"{id}\"\ntag = \"{tag}\"\nhost = \"de1.example.test\"\nport = 443\ntransport = \"vless_reality\"\nserver_name = \"www.example.com\"\nreality_public_key = \"{EXIT_PUBLIC}\"\nreality_short_id = \"0a1b2c3d\"\nfailure_domain = \"exit:de1\"\npath = \"{path}\"\n{credential_ref}"
            ));
        }
    }
    let cfg = dir.join("deployment.toml");
    std::fs::write(&cfg, text).unwrap();
    cfg
}

fn admin(dir: &Path, cfg: &Path) -> Command {
    let mut cmd = support::vpn_admin();
    cmd.arg("--config").arg(cfg);
    cmd.current_dir(dir);
    cmd.env("SINGBOX_VPN_ALLOW_OFFLINE_MUTATION", "1");
    cmd.env("SINGBOX_VPN_LOCK_PATH", dir.join("state.lock"));
    cmd
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// init + one user + (for a paired relay) that user's exit credential.
/// Returns the user id and the user's local VLESS UUID (credential A).
fn provision(dir: &Path, cfg: &Path, shape: Shape) -> (String, String) {
    admin(dir, cfg).arg("init").assert().success();
    let out = stdout(
        &admin(dir, cfg)
            .args(["user", "create", "--name", "alice", "--json"])
            .assert()
            .success(),
    );
    let id = serde_json::from_str::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    if shape == Shape::PairedRelay {
        admin(dir, cfg)
            .args([
                "user",
                "peer",
                "set",
                &id,
                "de1-direct",
                "--credential-stdin",
            ])
            .write_stdin(format!("{EXIT_UUID}\n"))
            .assert()
            .success();
    }
    let users: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("state/users/users.json")).unwrap())
            .unwrap();
    let local_uuid = users["users"][0]["vless_uuid"]
        .as_str()
        .unwrap()
        .to_string();
    (id, local_uuid)
}

fn rendered(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("state/sing-box/config.json")).unwrap()).unwrap()
}

#[test]
fn production_code_never_bypasses_the_role_aware_renderer() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["apps/admin/src", "services/subscription/src"] {
        for entry in std::fs::read_dir(workspace.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("render_singbox_server_config("),
                "{path:?} calls the transport-only renderer directly; every production render \
                 must go through render_server_config_for_deployment so a relay can never be \
                 rendered as an unrestricted exit"
            );
        }
    }
}

#[test]
fn relay_render_config_applies_a_fail_closed_document() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    provision(dir.path(), &cfg, Shape::PairedRelay);
    admin(dir.path(), &cfg)
        .arg("render-config")
        .assert()
        .success();
    let doc = rendered(dir.path());
    let rules = doc["route"]["rules"]
        .as_array()
        .expect("relay route rules applied");
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[1]["domain"], serde_json::json!(["de1.example.test"]));
    assert_eq!(rules.last().unwrap()["action"], "reject");
    assert!(!serde_json::to_string(&doc).unwrap().contains(EXIT_UUID));

    // Repeating render-config (the expiry timer, a repair) never loosens it.
    let _ = admin(dir.path(), &cfg)
        .args(["render-config", "--require-applied"])
        .assert();
    assert_eq!(rendered(dir.path()), doc);
}

#[test]
fn unpaired_relay_renders_reject_all_and_doctor_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::UnpairedRelay);
    provision(dir.path(), &cfg, Shape::UnpairedRelay);
    admin(dir.path(), &cfg)
        .arg("render-config")
        .assert()
        .success();
    assert_eq!(
        rendered(dir.path())["route"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let out = stdout(&admin(dir.path(), &cfg).args(["doctor", "--json"]).assert());
    let report: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let checks = report["checks"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["status"] == "ok"
        && c["message"]
            .as_str()
            .unwrap()
            .contains("relay forwarding policy is fail-closed")));
    assert!(checks
        .iter()
        .any(|c| c["status"] == "warn"
            && c["message"].as_str().unwrap().contains("relay is UNPAIRED")));
}

#[test]
fn exit_render_is_unchanged_across_migration_to_explicit_identity() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::LegacyExit);
    provision(dir.path(), &cfg, Shape::LegacyExit);
    admin(dir.path(), &cfg)
        .arg("render-config")
        .assert()
        .success();
    let before = std::fs::read(dir.path().join("state/sing-box/config.json")).unwrap();
    assert!(rendered(dir.path()).get("route").is_none());

    admin(dir.path(), &cfg)
        .args(["config", "validate"])
        .assert()
        .code(2);
    admin(dir.path(), &cfg)
        .args(["config", "migrate"])
        .assert()
        .success();
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.starts_with("schema_version = 2\nnode_id = \"de1\"\nrole = \"exit\"\n"));
    admin(dir.path(), &cfg)
        .args(["config", "validate"])
        .assert()
        .success();

    admin(dir.path(), &cfg)
        .arg("render-config")
        .assert()
        .success();
    assert_eq!(
        std::fs::read(dir.path().join("state/sing-box/config.json")).unwrap(),
        before,
        "an exit's live config is byte-identical after gaining explicit identity"
    );
}

#[test]
fn schema_v1_is_migration_required_and_migration_keeps_the_relay_role() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let v1 = std::fs::read_to_string(&cfg).unwrap().replace(
        "schema_version = 2\nnode_id = \"ru1\"\n",
        "schema_version = 1\n",
    );
    std::fs::write(&cfg, v1).unwrap();

    let out = stdout(
        &admin(dir.path(), &cfg)
            .args(["config", "validate"])
            .assert()
            .code(2),
    );
    assert!(out.contains("OUTDATED"), "{out}");
    admin(dir.path(), &cfg)
        .args(["config", "migrate"])
        .assert()
        .success();
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("role = \"relay\""));
    assert!(!text.contains("role = \"exit\""));
    assert!(text.contains("node_id = \"ru1\""));
    let out = stdout(
        &admin(dir.path(), &cfg)
            .args(["config", "validate"])
            .assert()
            .success(),
    );
    assert!(out.contains("role relay"), "{out}");
    assert!(
        out.contains(&format!(
            "{} (node role: relay)",
            compat_config::deployment::RELAY_ENFORCEMENT_CAPABILITY
        )),
        "update.sh relies on this marker to refuse unsafe relay downgrades:\n{out}"
    );
}

#[test]
fn invalid_role_fails_every_command_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::Exit);
    let text = std::fs::read_to_string(&cfg)
        .unwrap()
        .replace("role = \"exit\"", "role = \"entry\"");
    std::fs::write(&cfg, text).unwrap();
    for args in [
        vec!["status"],
        vec!["render-config"],
        vec!["config", "validate"],
    ] {
        admin(dir.path(), &cfg)
            .args(&args)
            .assert()
            .failure()
            .stderr(predicates::str::contains("entry"));
    }
}

#[test]
fn status_reports_identity_role_and_pairing_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let (_, local_uuid) = provision(dir.path(), &cfg, Shape::PairedRelay);
    let out = stdout(&admin(dir.path(), &cfg).arg("status").assert().success());
    assert!(out.contains("Node:                  ru1"), "{out}");
    assert!(out.contains("Role:                  relay"), "{out}");
    assert!(out.contains("1 declared exit target(s)"), "{out}");
    for secret in [local_uuid.as_str(), EXIT_UUID, REALITY_PRIVATE] {
        assert!(!out.contains(secret), "status leaked a credential:\n{out}");
    }
}

#[test]
fn read_only_commands_never_print_either_credential_scope() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let (id, local_uuid) = provision(dir.path(), &cfg, Shape::PairedRelay);
    admin(dir.path(), &cfg)
        .arg("render-config")
        .assert()
        .success();
    for args in [
        vec!["status".to_string()],
        vec!["user".into(), "list".into()],
        vec!["user".into(), "peer".into(), "list".into(), id.clone()],
        vec!["user".into(), "subscription".into(), id.clone()],
        vec!["config".into(), "validate".into()],
        vec!["doctor".into(), "--json".into()],
        vec!["doctor".into(), "--report".into()],
    ] {
        let assert = admin(dir.path(), &cfg).args(&args).assert();
        let output = assert.get_output();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for secret in [local_uuid.as_str(), EXIT_UUID, REALITY_PRIVATE] {
            assert!(
                !text.contains(secret),
                "`{}` leaked a credential:\n{text}",
                args.join(" ")
            );
        }
    }
}

#[test]
fn user_links_on_a_relay_never_emit_the_first_hop_or_downgrade_a_relay_route() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let (id, local_uuid) = provision(dir.path(), &cfg, Shape::PairedRelay);
    let out = stdout(
        &admin(dir.path(), &cfg)
            .args(["user", "links", &id])
            .assert()
            .success(),
    );
    let links: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("vless://") || l.starts_with("hysteria2://"))
        .collect();
    assert_eq!(
        links.len(),
        1,
        "only the direct exit route is representable:\n{out}"
    );
    assert!(links[0].starts_with(&format!("vless://{EXIT_UUID}@de1.example.test:443")));
    assert!(out.contains("Germany · via Russia: omitted"), "{out}");
    assert!(
        !out.contains("ru1.example.test"),
        "the relay first hop is never a link:\n{out}"
    );
    assert!(!out.contains(&local_uuid));
}

#[test]
fn exit_user_links_are_unchanged_by_relay_support() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::Exit);
    let (id, local_uuid) = provision(dir.path(), &cfg, Shape::Exit);
    let out = stdout(
        &admin(dir.path(), &cfg)
            .args(["user", "links", &id])
            .assert()
            .success(),
    );
    assert!(out.contains(&format!("vless://{local_uuid}@de1.example.test:443")));
    assert!(out.contains("hysteria2://"));
}

fn backup(dir: &Path, cfg: &Path) -> PathBuf {
    let path = dir.join("backup.tar");
    admin(dir, cfg)
        .args(["backup", "--output"])
        .arg(&path)
        .assert()
        .success();
    path
}

#[test]
fn restore_refuses_a_backup_from_another_role_or_node_and_changes_nothing() {
    let exit_dir = tempfile::tempdir().unwrap();
    let exit_cfg = write_deployment(exit_dir.path(), Shape::Exit);
    provision(exit_dir.path(), &exit_cfg, Shape::Exit);
    let exit_backup = backup(exit_dir.path(), &exit_cfg);

    let relay_dir = tempfile::tempdir().unwrap();
    let relay_cfg = write_deployment(relay_dir.path(), Shape::PairedRelay);
    provision(relay_dir.path(), &relay_cfg, Shape::PairedRelay);
    let users_before = std::fs::read(relay_dir.path().join("state/users/users.json")).unwrap();
    admin(relay_dir.path(), &relay_cfg)
        .arg("restore")
        .arg(&exit_backup)
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "taken on a node with role exit, but this node's role is relay",
        ));
    assert_eq!(
        std::fs::read(relay_dir.path().join("state/users/users.json")).unwrap(),
        users_before
    );

    // Same role, different node identity.
    let other_dir = tempfile::tempdir().unwrap();
    let other_cfg = write_deployment(other_dir.path(), Shape::Exit);
    let renamed = std::fs::read_to_string(&other_cfg)
        .unwrap()
        .replace("node_id = \"de1\"", "node_id = \"de2\"");
    std::fs::write(&other_cfg, renamed).unwrap();
    provision(other_dir.path(), &other_cfg, Shape::Exit);
    admin(other_dir.path(), &other_cfg)
        .arg("restore")
        .arg(&exit_backup)
        .assert()
        .failure()
        .stderr(predicates::str::contains("belongs to node \"de1\""));
}

#[test]
fn relay_backup_restores_onto_the_same_relay_and_keeps_its_restrictions() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    provision(dir.path(), &cfg, Shape::PairedRelay);
    admin(dir.path(), &cfg)
        .arg("render-config")
        .assert()
        .success();
    let restricted = rendered(dir.path());
    let archive = backup(dir.path(), &cfg);

    std::fs::remove_file(dir.path().join("state/users/users.json")).unwrap();
    admin(dir.path(), &cfg)
        .arg("restore")
        .arg(&archive)
        .assert()
        .success();
    let restored = rendered(dir.path());
    assert_eq!(restored["route"], restricted["route"]);
    assert_eq!(
        restored["inbounds"][0]["users"],
        restricted["inbounds"][0]["users"]
    );
    let users = std::fs::read_to_string(dir.path().join("state/users/users.json")).unwrap();
    assert!(
        users.contains(EXIT_UUID),
        "restore keeps the operator-supplied exit credential"
    );
}

// ----------------------------------------------------------------------
// O5 (real two-VPS acceptance): `user peer set|rotate --uuid <secret>`
// exposes the exit credential in process listings and shell history.
// ----------------------------------------------------------------------

fn create_user(dir: &Path, cfg: &Path) -> String {
    admin(dir, cfg).arg("init").assert().success();
    let out = stdout(
        &admin(dir, cfg)
            .args(["user", "create", "--name", "carol", "--json"])
            .assert()
            .success(),
    );
    serde_json::from_str::<serde_json::Value>(&out).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn stored_peer_uuid(dir: &Path) -> Option<String> {
    let users: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("state/users/users.json")).unwrap())
            .unwrap();
    let text = users["users"][0]["peer_credentials"]["de1-direct"].to_string();
    [EXIT_UUID, ROTATED_EXIT_UUID]
        .into_iter()
        .find(|candidate| text.contains(candidate))
        .map(str::to_string)
}

const ROTATED_EXIT_UUID: &str = "b2b2b2b2-b2b2-4b2b-8b2b-b2b2b2b2b2b2";

#[test]
fn peer_credential_from_stdin_never_appears_in_the_process_arguments() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let id = create_user(dir.path(), &cfg);

    let mut child = support::vpn_admin_process()
        .arg("--config")
        .arg(&cfg)
        .args([
            "user",
            "peer",
            "set",
            &id,
            "de1-direct",
            "--credential-stdin",
        ])
        .current_dir(dir.path())
        .env("SINGBOX_VPN_ALLOW_OFFLINE_MUTATION", "1")
        .env("SINGBOX_VPN_LOCK_PATH", dir.path().join("state.lock"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    // While it waits for the credential, its argv is what `ps` shows.
    let cmdline_path = format!("/proc/{}/cmdline", child.id());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let argv = loop {
        let raw = std::fs::read(&cmdline_path).unwrap_or_default();
        let argv = String::from_utf8_lossy(&raw).replace('\0', " ");
        if argv.contains("--credential-stdin") {
            break argv;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "vpn-admin did not start"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        !argv.contains(EXIT_UUID),
        "credential visible in argv: {argv}"
    );

    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{EXIT_UUID}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!printed.contains(EXIT_UUID), "credential echoed: {printed}");
    assert_eq!(stored_peer_uuid(dir.path()).as_deref(), Some(EXIT_UUID));
}

#[test]
fn peer_credential_rotation_accepts_stdin_and_refuses_empty_or_invalid_input_without_echo() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let id = create_user(dir.path(), &cfg);
    let peer = |verb: &str, input: &str| {
        admin(dir.path(), &cfg)
            .args([
                "user",
                "peer",
                verb,
                &id,
                "de1-direct",
                "--credential-stdin",
            ])
            .write_stdin(input.to_string())
            .assert()
    };

    peer("set", &format!("{EXIT_UUID}\n")).success();
    peer("rotate", &format!("{ROTATED_EXIT_UUID}\n")).success();
    assert_eq!(
        stored_peer_uuid(dir.path()).as_deref(),
        Some(ROTATED_EXIT_UUID)
    );

    peer("rotate", "").failure();
    let near_miss = "b3b3b3b3-b3b3-4b3b-8b3b-b3b3b3b3b3bZ";
    let refused = peer("rotate", &format!("{near_miss}\n")).failure();
    let stderr = String::from_utf8_lossy(&refused.get_output().stderr).to_string();
    assert!(
        !stderr.contains(near_miss),
        "rejected value echoed: {stderr}"
    );
    assert_eq!(
        stored_peer_uuid(dir.path()).as_deref(),
        Some(ROTATED_EXIT_UUID),
        "refused input changes nothing"
    );

    admin(dir.path(), &cfg)
        .args([
            "user",
            "peer",
            "rotate",
            &id,
            "de1-direct",
            "--credential-stdin",
            "--uuid",
            EXIT_UUID,
        ])
        .assert()
        .failure();
}

#[test]
fn legacy_uuid_argument_still_works_and_warns_about_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_deployment(dir.path(), Shape::PairedRelay);
    let id = create_user(dir.path(), &cfg);
    let assert = admin(dir.path(), &cfg)
        .args([
            "user",
            "peer",
            "set",
            &id,
            "de1-direct",
            "--uuid",
            EXIT_UUID,
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(stderr.contains("--credential-stdin"), "{stderr}");
    assert!(!stderr.contains(EXIT_UUID));
    assert_eq!(stored_peer_uuid(dir.path()).as_deref(), Some(EXIT_UUID));
}
