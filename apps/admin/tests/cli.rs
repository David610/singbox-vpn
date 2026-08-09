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
