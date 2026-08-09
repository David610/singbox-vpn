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

fn write_deployment_toml_with_singbox(dir: &Path, singbox_binary: &Path) -> std::path::PathBuf {
    let cfg_path = dir.join("deployment.toml");
    let state_dir = dir.join("state");
    let toml = format!(
        r#"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"
state_dir = "{state}"
singbox_binary = "{singbox}"

[reality]
listen_port = 443
handshake_server = "www.microsoft.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        state = state_dir.display(),
        singbox = singbox_binary.display(),
    );
    std::fs::write(&cfg_path, toml).unwrap();
    cfg_path
}

/// A fake `sing-box` binary supporting just enough subcommands to drive
/// the REALITY rotation flow: `generate reality-keypair` (returns a
/// fresh random-looking keypair each call, so rotation is observable),
/// `check -c <path>` (exit 0 unless `fail_check` is set), and `version`.
#[cfg(unix)]
fn fake_singbox(dir: &Path, fail_check: bool) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-sing-box.sh");
    let check_exit = if fail_check { 1 } else { 0 };
    let script = format!(
        r#"#!/usr/bin/env bash
case "$1" in
  generate)
    n=$(date +%s%N)
    echo "PrivateKey: priv-$n-$$"
    echo "PublicKey: pub-$n-$$"
    exit 0
    ;;
  check)
    if [ {check_exit} -ne 0 ]; then
      echo "fake sing-box: candidate config rejected" >&2
    fi
    exit {check_exit}
    ;;
  version)
    echo "sing-box test-fake 1.0.0"
    exit 0
    ;;
esac
exit 1
"#
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn admin(dir: &Path, cfg_path: &Path) -> Command {
    let mut cmd = Command::cargo_bin("vpn-admin").unwrap();
    cmd.arg("--config").arg(cfg_path);
    cmd.current_dir(dir);
    cmd
}

/// docs/FINAL_PRODUCTION_AUDIT.md P0-4: two concurrent `vpn-admin user
/// create` invocations against the same state dir must both succeed and
/// both end up persisted — the state lock (apps/admin/src/lock.rs) must
/// serialize their load-mutate-persist sequences rather than letting the
/// second writer's `users.json` overwrite the first writer's user out of
/// existence. Uses a dedicated `VPN1_LOCK_PATH` so this test never
/// contends with other tests or a real host's `/run/lock/vpn1.lock`.
#[test]
fn concurrent_user_creates_do_not_lose_an_update() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let lock_path = dir.path().join("vpn1-test.lock");

    let spawn_create = |name: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_vpn-admin"))
            .arg("--config")
            .arg(&cfg_path)
            .args(["user", "create", "--name", name])
            .env("VPN1_LOCK_PATH", &lock_path)
            .current_dir(dir.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };

    let mut child_a = spawn_create("alice");
    let mut child_b = spawn_create("bob");
    let status_a = child_a.wait().unwrap();
    let status_b = child_b.wait().unwrap();
    assert!(status_a.success(), "first concurrent create must succeed");
    assert!(status_b.success(), "second concurrent create must succeed");

    let list = admin(dir.path(), &cfg_path)
        .args(["user", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8(list.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("alice"),
        "alice must be present, got:\n{stdout}"
    );
    assert!(
        stdout.contains("bob"),
        "bob must be present (must not have been lost to a racing write), got:\n{stdout}"
    );
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

/// docs/FINAL_PRODUCTION_AUDIT.md P0-5: `vpn-admin init --rotate` must
/// actually replace the REALITY public key on disk (proving the
/// coordinated rotate path ran, not the old bare-overwrite code), and
/// must succeed end-to-end (config re-render + validate against the
/// real, if fake, sing-box binary).
#[test]
fn reality_rotate_replaces_public_key_and_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);

    admin(dir.path(), &cfg_path).arg("init").assert().success();
    let pub_key_path = dir.path().join("state/reality/public.key");
    let before = std::fs::read_to_string(&pub_key_path).unwrap();

    admin(dir.path(), &cfg_path)
        .args(["init", "--rotate"])
        .assert()
        .success()
        .stdout(predicates::str::contains("REALITY key rotated"));

    let after = std::fs::read_to_string(&pub_key_path).unwrap();
    assert_ne!(before, after, "public key must change after rotation");
    // no leftover rotate-bak/rotate-tmp files after a successful rotation
    let leftover: Vec<_> = std::fs::read_dir(dir.path().join("state/reality"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.contains("rotate-bak") || n.contains("rotate-tmp")
        })
        .collect();
    assert!(
        leftover.is_empty(),
        "leftover rotation temp files: {leftover:?}"
    );
}

/// docs/FINAL_PRODUCTION_AUDIT.md P0-5: if the candidate config fails
/// `sing-box check`, rotation must fail LOUDLY and leave the previous
/// REALITY key material completely unchanged — never half-rotated.
#[test]
fn reality_rotate_rolls_back_key_material_on_validation_failure() {
    let dir = tempfile::tempdir().unwrap();
    let good_singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &good_singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();

    let pub_key_path = dir.path().join("state/reality/public.key");
    let priv_key_path = dir.path().join("state/reality/private.key");
    let sid_path = dir.path().join("state/reality/short_id.txt");
    let pub_before = std::fs::read_to_string(&pub_key_path).unwrap();
    let priv_before = std::fs::read_to_string(&priv_key_path).unwrap();
    let sid_before = std::fs::read_to_string(&sid_path).unwrap();

    // Swap in a sing-box binary that fails `check`, so the candidate
    // config produced by this rotate attempt is rejected.
    // `fake_singbox` always writes to the same fixed filename within
    // `dir`, so this in-place-overwrites the exact path
    // deployment.toml's singbox_binary already points at.
    let failing_singbox = fake_singbox(dir.path(), true);
    assert_eq!(failing_singbox, good_singbox);

    admin(dir.path(), &cfg_path)
        .args(["init", "--rotate"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("rotation FAILED"));

    assert_eq!(
        pub_before,
        std::fs::read_to_string(&pub_key_path).unwrap(),
        "public key must be unchanged after a failed rotation"
    );
    assert_eq!(
        priv_before,
        std::fs::read_to_string(&priv_key_path).unwrap(),
        "private key must be unchanged after a failed rotation"
    );
    assert_eq!(
        sid_before,
        std::fs::read_to_string(&sid_path).unwrap(),
        "short_id must be unchanged after a failed rotation"
    );
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
