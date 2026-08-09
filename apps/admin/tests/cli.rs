//! End-to-end test of the `vpn-admin` binary against a real (temp-dir)
//! deployment config and user store — no real `sing-box` binary is
//! required because `regenerate_singbox_config` degrades to a warning
//! when the configured binary path doesn't exist (see main.rs), so these
//! assertions focus on user-store correctness, which is what `vpn-admin`
//! actually owns.

use assert_cmd::Command;
use std::path::Path;

fn write_deployment_toml(dir: &Path) -> std::path::PathBuf {
    let cfg_path = dir.join("deployment.toml");
    let state_dir = dir.join("state");
    let toml = format!(
        r#"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"
state_dir = "{state}"
singbox_binary = "{state}/nonexistent-sing-box"

[reality]
listen_port = 443
handshake_server = "www.microsoft.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        state = state_dir.display(),
    );
    std::fs::write(&cfg_path, toml).unwrap();
    cfg_path
}

fn admin(dir: &Path, cfg_path: &Path) -> Command {
    let mut cmd = Command::cargo_bin("vpn-admin").unwrap();
    cmd.arg("--config").arg(cfg_path);
    cmd.current_dir(dir);
    cmd
}

#[test]
fn full_user_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());

    // create prints a subscription URL exactly once.
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "david"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("User created: user_"));
    assert!(stdout.contains("https://sub.example.com:8443/sub/"));

    let user_id = stdout
        .lines()
        .find(|l| l.starts_with("User created: "))
        .unwrap()
        .trim_start_matches("User created: ")
        .to_string();

    // list never prints secrets.
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&user_id));
    assert!(stdout.contains("yes")); // enabled=yes
    assert!(!stdout.to_lowercase().contains("uuid"));

    // disable takes effect in the store.
    admin(dir.path(), &cfg_path)
        .args(["user", "disable", &user_id])
        .assert()
        .success();
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains(&format!("{user_id:<20} david            no")));

    // re-enable.
    admin(dir.path(), &cfg_path)
        .args(["user", "enable", &user_id])
        .assert()
        .success();

    // rotate-token prints a fresh URL.
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "rotate-token", &user_id])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("New subscription:"));

    // remove deletes the user.
    admin(dir.path(), &cfg_path)
        .args(["user", "remove", &user_id])
        .assert()
        .success();
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains(&user_id));

    // removing again fails cleanly.
    admin(dir.path(), &cfg_path)
        .args(["user", "remove", &user_id])
        .assert()
        .failure();
}

#[test]
fn init_without_singbox_binary_fails_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    admin(dir.path(), &cfg_path)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicates::str::contains("sing-box"));
}

#[test]
fn version_prints_own_version() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let output = admin(dir.path(), &cfg_path)
        .arg("version")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("vpn1 "));
}

#[test]
fn status_reports_user_counts_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "alice"])
        .assert()
        .success();

    let output = admin(dir.path(), &cfg_path)
        .arg("status")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("active:   1"));
    assert!(!stdout.to_lowercase().contains("uuid"));
    assert!(!stdout.contains("Bearer"));
}

#[test]
fn user_create_json_output_has_no_server_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "bob", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    // `regenerate_singbox_config` may print an informational warning line
    // before the JSON block (no sing-box binary in this test
    // environment) — the JSON itself starts at the first `{`.
    let json_start = stdout.find('{').expect("JSON object in output");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout[json_start..]).expect("valid JSON");
    assert_eq!(parsed["name"], "bob");
    assert_eq!(parsed["enabled"], true);
    assert!(parsed["subscription_url"]
        .as_str()
        .unwrap()
        .starts_with("https://sub.example.com"));
    assert!(parsed.get("vless_uuid").is_none());
    assert!(parsed.get("private_key").is_none());
}

#[test]
fn user_create_qr_prints_a_qr_code() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "carol", "--qr"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    // Terminal QR rendering uses block-drawing characters; just assert
    // there's substantially more multi-line block output than the plain
    // text path alone would produce.
    assert!(stdout.lines().count() > 15);
}

#[test]
fn user_qr_rotates_token_and_warns_it_is_new() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let output = admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "dave"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let user_id = stdout
        .lines()
        .find(|l| l.starts_with("User created: "))
        .unwrap()
        .trim_start_matches("User created: ")
        .to_string();

    let output = admin(dir.path(), &cfg_path)
        .args(["user", "qr", &user_id])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("mints a fresh one"));
    assert!(stdout.contains("New subscription:"));
}

#[test]
fn doctor_reports_missing_singbox_binary_as_failure() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    admin(dir.path(), &cfg_path)
        .arg("doctor")
        .assert()
        .failure()
        .stdout(predicates::str::contains("[FAIL]"));
}

#[test]
fn backup_then_restore_round_trips_users() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let state_dir = dir.path().join("state");

    // `backup`/`restore` require a REALITY private key to exist (a real
    // deployment always has one after `vpn-admin init`); `init` itself
    // needs a real `sing-box` binary this test environment doesn't have,
    // so write a placeholder key directly, matching what `init` would
    // have produced on disk.
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();
    std::fs::write(state_dir.join("reality/private.key"), "test-private-key").unwrap();
    std::fs::write(state_dir.join("reality/public.key"), "test-public-key").unwrap();

    admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "erin"])
        .assert()
        .success();

    let backup_path = dir.path().join("backup.tar");
    admin(dir.path(), &cfg_path)
        .args(["backup", "--output"])
        .arg(&backup_path)
        .assert()
        .success();
    assert!(backup_path.exists());

    // Simulate loss of the live user store, then restore from backup.
    std::fs::remove_file(state_dir.join("users/users.json")).unwrap();

    admin(dir.path(), &cfg_path)
        .arg("restore")
        .arg(&backup_path)
        .assert()
        .success();

    let output = admin(dir.path(), &cfg_path)
        .args(["user", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("erin"));
}
