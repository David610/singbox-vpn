//! End-to-end test of the `vpn-admin` binary against a real (temp-dir)
//! deployment config and user store — no real `sing-box` binary is
//! required because `regenerate_singbox_config` degrades to a warning
//! when the configured binary path doesn't exist (see main.rs), so these
//! assertions focus on user-store correctness, which is what `vpn-admin`
//! actually owns.

use assert_cmd::Command;
use std::path::Path;

const REALITY_PRIVATE_A: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
const REALITY_PUBLIC_A: &str = "pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk";
const REALITY_PRIVATE_B: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI";
const REALITY_PUBLIC_B: &str = "zo060cy2M-x7cMF4FKXHbs0CloUFDTRHRboFhw5YfVk";
const REALITY_PRIVATE_C: &str = "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM";
const REALITY_PUBLIC_C: &str = "Xf7dO2vUf2-ijuFdlp1bsOpTd01Ii9r53xxuASSz7yI";

fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

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
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        state = toml_path(&state_dir),
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
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        state = toml_path(&state_dir),
        singbox = toml_path(singbox_binary),
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
    if [ -e "$0.generated-once" ]; then
      echo "PrivateKey: {private_c}"
      echo "PublicKey: {public_c}"
    else
      : > "$0.generated-once"
      echo "PrivateKey: {private_b}"
      echo "PublicKey: {public_b}"
    fi
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
"#,
        private_b = REALITY_PRIVATE_B,
        public_b = REALITY_PUBLIC_B,
        private_c = REALITY_PRIVATE_C,
        public_c = REALITY_PUBLIC_C,
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(not(unix))]
fn fake_singbox(dir: &Path, fail_check: bool) -> std::path::PathBuf {
    let path = dir.join("fake-sing-box.cmd");
    let check_exit = if fail_check { 1 } else { 0 };
    let script = format!(
        r#"@echo off
if "%1"=="generate" (
  if exist "%~f0.generated-once" (
    echo PrivateKey: {private_c}
    echo PublicKey: {public_c}
  ) else (
    type nul > "%~f0.generated-once"
    echo PrivateKey: {private_b}
    echo PublicKey: {public_b}
  )
  exit /b 0
)
if "%1"=="check" exit /b {check_exit}
if "%1"=="version" (
  echo sing-box test-fake 1.0.0
  exit /b 0
)
exit /b 1
"#,
        private_b = REALITY_PRIVATE_B,
        public_b = REALITY_PUBLIC_B,
        private_c = REALITY_PRIVATE_C,
        public_c = REALITY_PUBLIC_C,
    );
    std::fs::write(&path, script).unwrap();
    path
}

/// A fake `systemctl` that logs every invocation's verb+unit to
/// `$SYSTEMCTL_LOG` (one line per call) and always reports units as
/// installed/active, so `regenerate_singbox_config`'s and
/// `cmd_reality_rotate`'s/`cmd_restore`'s reload/restart paths run for
/// real (rather than degrading to a "systemctl not available" warning)
/// without a real systemd host.
#[cfg(unix)]
fn fake_systemctl(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("systemctl");
    let script = r#"#!/usr/bin/env bash
echo "$1 $2" >> "$SYSTEMCTL_LOG"
case "$1" in
  --version) exit 0 ;;
  show) echo "loaded"; exit 0 ;;
  reload-or-restart) exit 0 ;;
  is-active) exit 0 ;;
esac
exit 1
"#;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn admin(dir: &Path, cfg_path: &Path) -> Command {
    let mut cmd = Command::cargo_bin("vpn-admin").unwrap();
    cmd.arg("--config").arg(cfg_path);
    cmd.current_dir(dir);
    cmd.env("VPN1_ALLOW_OFFLINE_MUTATION", "1");
    cmd
}

/// A probably-free localhost port, picked by binding to port 0 and
/// releasing it immediately. Small TOCTOU race is acceptable for a test
/// harness. Used so tests that spawn the REAL `subscription` service
/// binary never collide with each other or with any other test's
/// hardcoded port when run in parallel (`cargo test`'s default).
#[cfg(unix)]
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[cfg(unix)]
fn write_deployment_toml_with_singbox_and_sub_port(
    dir: &Path,
    singbox_binary: &Path,
    sub_port: u16,
) -> std::path::PathBuf {
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
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = {sub_port}
"#,
        state = toml_path(&state_dir),
        singbox = toml_path(singbox_binary),
    );
    std::fs::write(&cfg_path, toml).unwrap();
    cfg_path
}

#[cfg(unix)]
fn wait_for_local_port(port: u16, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

/// Kills the process on drop — a test that panics partway through must
/// not leave a real `subscription` server bound to a port forever.
#[cfg(unix)]
struct KillOnDrop(std::process::Child);
#[cfg(unix)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawns the REAL, compiled `subscription` service binary (not a mock)
/// against `cfg_path`, as a genuine long-lived background process.
/// `assert_cmd::Command::cargo_bin` is used only to resolve the correct
/// workspace binary path (it deliberately hides `spawn()` — it's built
/// for one-shot `.assert()`-style invocations — so the resolved program
/// path is re-wrapped in a plain `std::process::Command` here to get a
/// real, killable `Child`).
#[cfg(unix)]
fn spawn_subscription_binary(cfg_path: &Path) -> std::process::Child {
    let exe = if cfg!(windows) {
        "subscription.exe"
    } else {
        "subscription"
    };
    let program = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent()?.parent().map(|target| target.join(exe)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| {
            Command::cargo_bin("subscription")
                .expect("locate the subscription workspace binary")
                .get_program()
                .into()
        });
    std::process::Command::new(program)
        .arg("--config")
        .arg(cfg_path)
        .spawn()
        .expect("spawn real subscription binary")
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
            .env("VPN1_ALLOW_OFFLINE_MUTATION", "1")
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
    assert!(stdout.contains("User ID:"));
    assert!(stdout.contains("NOT your subscription token"));
    assert!(stdout.contains("https://sub.example.com:8443/sub/"));

    let user_id = stdout
        .lines()
        .skip_while(|l| *l != "User ID:")
        .nth(1)
        .unwrap()
        .trim()
        .to_string();
    assert!(user_id.starts_with("user_"));

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
    assert!(stdout.contains("New Hiddify subscription URL for"));

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
        .stderr(predicates::str::contains(
            "live state and transaction backups were not changed",
        ));

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
        .skip_while(|l| *l != "User ID:")
        .nth(1)
        .unwrap()
        .trim()
        .to_string();

    let output = admin(dir.path(), &cfg_path)
        .args(["user", "qr", &user_id])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("mints a fresh one"));
    assert!(stdout.contains("New Hiddify subscription URL for"));
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

/// Public key confidentiality is not required, but its exact installed
/// owner/group/mode is operationally required: vpn-subscription must read
/// it and unrelated principals need no local access.
#[test]
#[cfg(unix)]
fn doctor_rejects_wrong_reality_public_key_install_policy() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();

    // Force the public key world-readable regardless of this test
    // process's own umask, so the assertion below doesn't depend on
    // the environment's umask happening to already be permissive.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let pub_key = dir.path().join("state/reality/public.key");
        std::fs::set_permissions(&pub_key, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let output = admin(dir.path(), &cfg_path)
        .arg("doctor")
        .assert()
        .failure();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("REALITY public key policy invalid"),
        "doctor must enforce the installed root:vpn-subscription 0640 contract:\n{stdout}"
    );
}

/// L4 subscription-coherence checks pass once `init` and `render-config`
/// have together produced a coherent REALITY key file set and a
/// matching on-disk sing-box config — this is the "everything is fine"
/// baseline the drift test below deliberately breaks.
#[test]
fn doctor_l4_coherence_passes_after_init_and_render() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    // Not asserting overall `.success()` here: an unrelated L2 check
    // (world-readable key file permissions) is environment-dependent
    // (umask of the process creating the temp dir), which is not what
    // this test is about — it only cares about the new L4 lines.
    let output = admin(dir.path(), &cfg_path).arg("doctor").assert();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("[L4"),
        "doctor output must tag L4 checks:\n{stdout}"
    );
    assert!(
        stdout.contains("subscription render coherence") && stdout.contains("[OK]"),
        "L4 render coherence must pass on a freshly-initialized deployment:\n{stdout}"
    );
    assert!(
        stdout.contains("on-disk sing-box config.json exactly matches"),
        "L4 on-disk drift check must report no drift right after render-config:\n{stdout}"
    );
    // Never a hard requirement of `doctor` overall unless `--protocol` is
    // passed.
    assert!(stdout.contains("[L5-6]") && stdout.contains("not run"));
}

/// The exact incident class this check exists for: the sing-box
/// config.json actually on disk (what a running sing-box would have
/// last reloaded) no longer matches what the CURRENT REALITY key files
/// would render — e.g. because someone hand-edited a key file, or a
/// `render-config` was skipped after a manual key change. `doctor` must
/// catch this from file contents alone, as a hard `[FAIL]`, without any
/// network access.
#[test]
fn doctor_l4_detects_on_disk_config_drift_from_current_key_files() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    // Simulate exactly the incident: the REALITY public key file changes
    // (as it would after a manual edit, or a partially-applied rotation)
    // but the previously-rendered sing-box config.json is left stale.
    let short_id_path = dir.path().join("state/reality/short_id.txt");
    std::fs::write(&short_id_path, "deadbeef").unwrap();

    let output = admin(dir.path(), &cfg_path).arg("doctor").assert();
    let output = output.failure();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("does NOT exactly match") && stdout.contains("[L4"),
        "doctor must FAIL the L4 on-disk drift check once the short_id file diverges from \
         the last-rendered config.json:\n{stdout}"
    );
}

/// The strongest coherence check `doctor` has: it doesn't just re-read
/// files (which a stale process would also "pass" against, since it
/// never touches the process) — it asks the REAL, already-running
/// `subscription` service binary for its own live-state fingerprint over
/// HTTP and compares that against a fresh disk read. Spawns the actual
/// compiled `subscription` binary (not a mock) so this is a genuine
/// integration test of `apps/admin/src/main.rs`'s
/// `check_l4_live_subscription_process_state` against
/// `services/subscription/src/lib.rs`'s `/internal/state-fingerprint`
/// route — the exact two sides of the split-brain this whole mechanism
/// exists to catch.
#[test]
#[cfg(unix)]
fn doctor_l4_live_check_passes_when_subscription_process_is_freshly_started() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let sub_port = free_port();
    let cfg_path = write_deployment_toml_with_singbox_and_sub_port(dir.path(), &singbox, sub_port);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    let child = spawn_subscription_binary(&cfg_path);
    let _guard = KillOnDrop(child);
    assert!(
        wait_for_local_port(sub_port, std::time::Duration::from_secs(5)),
        "real subscription binary never bound 127.0.0.1:{sub_port}"
    );

    let output = admin(dir.path(), &cfg_path).arg("doctor").assert();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("ALREADY-RUNNING vpn-subscription process's live in-memory state matches"),
        "doctor's live L4 check must pass against a subscription process that just started from \
         the current key files:\n{stdout}"
    );
}

/// The scenario the whole check exists for: REALITY key material on
/// disk changes (rotation, restore, a manual edit — doesn't matter which
/// one), but the ALREADY-RUNNING `subscription` process was never
/// restarted, so it keeps serving what it cached at its own startup.
/// Every prior check in `doctor` (which only re-reads files) would be
/// blind to this; only a live query against the actual running process
/// can catch it.
#[test]
#[cfg(unix)]
fn doctor_l4_live_check_fails_when_running_subscription_process_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let sub_port = free_port();
    let cfg_path = write_deployment_toml_with_singbox_and_sub_port(dir.path(), &singbox, sub_port);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    let child = spawn_subscription_binary(&cfg_path);
    let _guard = KillOnDrop(child);
    assert!(
        wait_for_local_port(sub_port, std::time::Duration::from_secs(5)),
        "real subscription binary never bound 127.0.0.1:{sub_port}"
    );

    // Mutate the REALITY key material on disk WITHOUT restarting the
    // already-running subscription process above — this is deliberately
    // NOT going through `vpn-admin init --rotate` (which would try, and
    // in a real deployment succeed, to restart vpn-subscription via
    // systemd): the whole point is to reproduce a process that stays up
    // across a key change, whatever the cause, and prove `doctor` can
    // still catch it purely by asking the live process.
    // Write a VALID but different X25519 keypair (not garbage) to trigger
    // the "keys don't match" detection in the live subscription process,
    // not a parse error in doctor's X25519 validation.
    std::fs::write(
        dir.path().join("state/reality/public.key"),
        REALITY_PUBLIC_B, // Valid but different from what init() generated (A)
    )
    .unwrap();
    std::fs::write(dir.path().join("state/reality/short_id.txt"), "deadbeef").unwrap();

    let output = admin(dir.path(), &cfg_path).arg("doctor").assert();
    let output = output.failure();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("RUNNING vpn-subscription process is serving STALE state"),
        "doctor's live L4 check must FAIL once the on-disk keys diverge from what the still-\
         running subscription process was started with:\n{stdout}"
    );
}

/// `public_host` in the default test fixture (`vpn.example.com`) does
/// not resolve — this is deterministic in both a networked and a fully
/// offline sandbox (either NXDOMAIN or "no network"), and either way it
/// must surface as an explicit `[FAIL]`, not be silently skipped.
#[test]
fn doctor_fails_on_unresolvable_public_hostname() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let output = admin(dir.path(), &cfg_path)
        .arg("doctor")
        .assert()
        .failure();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("does not resolve"),
        "doctor must fail closed on an unresolvable public hostname:\n{stdout}"
    );
}

/// A raw IPv4 literal as `public_host` resolves to exactly that one
/// IPv4 address and nothing else — `to_socket_addrs` parses a literal
/// address directly, without consulting DNS or `/etc/hosts`, so this is
/// deterministic across sandboxes/CI runners unlike `"localhost"` (which
/// resolves to IPv4-only on some hosts but IPv4+`::1` on others,
/// depending on that host's own `/etc/hosts` — this test used
/// `"localhost"` originally and was flaky in CI for exactly that
/// reason). The IPv6-policy check must report an address with no AAAA
/// as an explicit `[INFO]` (no AAAA record => no leak risk, but
/// IPv6-only clients cannot reach this host), not silently omit any
/// mention of IPv6 at all.
#[test]
fn doctor_reports_ipv4_only_hostname_as_info_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let cfg_path = dir.path().join("deployment.toml");
    let toml = format!(
        r#"
public_host = "127.0.0.1"
subscription_host = "sub.example.com"
state_dir = "{state}"
singbox_binary = "{state}/nonexistent-sing-box"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#,
        state = toml_path(&state_dir),
    );
    std::fs::write(&cfg_path, toml).unwrap();

    let output = admin(dir.path(), &cfg_path).arg("doctor").assert();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("no AAAA/IPv6 record"),
        "an A-only hostname must be called out explicitly, not left ambiguous:\n{stdout}"
    );
    assert!(
        stdout.contains("[INFO]"),
        "a missing AAAA record is informational, not a failure on its own:\n{stdout}"
    );
}

/// The `auto`/`urltest` scope-limitation reminder must always be present
/// in `doctor` output — this is the exact disclaimer the Telegram-
/// resilience investigation asked for so operators never mistake a
/// passing `doctor` run for proof that Telegram (or any specific app)
/// works.
#[test]
fn doctor_always_prints_urltest_scope_disclaimer() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let output = admin(dir.path(), &cfg_path).arg("doctor").assert();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("not Telegram-specific behavior"),
        "doctor must always remind operators that urltest/auto is not a Telegram test:\n{stdout}"
    );
}

#[test]
fn doctor_telegram_prints_disclaimer_and_never_claims_russian_verification() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    let output = admin(dir.path(), &cfg_path)
        .args(["doctor", "--telegram"])
        .assert();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Telegram-oriented summary"));
    assert!(
        stdout.contains("does NOT verify") && stdout.contains("Russian DPI compatibility"),
        "doctor --telegram must never claim to verify Russian censorship compatibility:\n{stdout}"
    );
    assert!(stdout.contains("docs/TELEGRAM_TROUBLESHOOTING.md"));
}

#[test]
fn doctor_client_prints_interactive_checklist_and_never_claims_to_probe_the_device() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    let output = admin(dir.path(), &cfg_path)
        .args(["doctor", "--client"])
        .assert();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Client acceptance checklist"));
    assert!(
        stdout.contains("does NOT by itself prove"),
        "doctor --client must lead with the connected-in-app-vs-VPN-routed distinction:\n{stdout}"
    );
    assert!(stdout.contains("Proxy Only"));
    assert!(stdout.contains("docs/clients/HIDDIFY_IOS.md"));
    // This command cannot reach into a phone — it must read as a checklist
    // to fill in by hand, never a claim of automated device inspection.
    assert!(stdout.contains("[ ]"));
}

#[test]
fn doctor_report_redacts_secrets_and_includes_expected_sections() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();
    admin(dir.path(), &cfg_path)
        .arg("render-config")
        .assert()
        .success();

    let output = admin(dir.path(), &cfg_path)
        .args(["doctor", "--report"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[system]"));
    assert!(stdout.contains("[services]"));
    assert!(stdout.contains("[listeners]"));
    assert!(stdout.contains("[hostname resolution]"));
    assert!(stdout.contains("[transport configuration]"));
    assert!(stdout.contains("[selected configuration]"));
    // Never leak the REALITY private key generated by `init` above.
    assert!(!stdout.contains(REALITY_PRIVATE_A));
    assert!(!stdout.contains("private_key"));
}

#[test]
fn doctor_report_output_writes_mode_0600_file() {
    let dir = tempfile::tempdir().unwrap();
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    admin(dir.path(), &cfg_path).arg("init").assert().success();

    let report_path = dir.path().join("report.txt");
    admin(dir.path(), &cfg_path)
        .args(["doctor", "--report", "--report-output"])
        .arg(&report_path)
        .assert()
        .success();
    assert!(report_path.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&report_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "report file must not be group/other readable");
    }
}

#[test]
fn backup_then_restore_round_trips_users() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let state_dir = dir.path().join("state");

    // `backup`/`restore` require a REALITY private key to exist (a real
    // deployment always has one after `vpn-admin init`); `init` itself
    // needs a real `sing-box` binary this test environment doesn't have,
    // so write a deterministic valid X25519 pair directly.
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();
    std::fs::write(state_dir.join("reality/private.key"), REALITY_PRIVATE_A).unwrap();
    std::fs::write(state_dir.join("reality/public.key"), REALITY_PUBLIC_A).unwrap();
    // `init` writes all THREE files; a fixture with only two is not what a
    // real deployment looks like, and `restore` now (correctly) refuses a
    // partial REALITY keyset because restoring one would split-brain the
    // server against the subscription service.
    std::fs::write(state_dir.join("reality/short_id.txt"), "deadbeef").unwrap();

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

#[test]
fn backup_refuses_to_clobber_a_preexisting_destination() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let destination = dir.path().join("backup.tar");
    std::fs::write(&destination, b"operator-owned-sentinel").unwrap();

    admin(dir.path(), &cfg_path)
        .args(["backup", "--output"])
        .arg(&destination)
        .assert()
        .failure()
        .stderr(predicates::str::contains("must not already exist"));
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"operator-owned-sentinel",
        "a refused backup must never remove or truncate the pre-existing path"
    );
}

#[test]
fn authorization_mutation_is_fail_closed_without_live_singbox() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let mut command = Command::cargo_bin("vpn-admin").unwrap();
    command
        .arg("--config")
        .arg(&cfg_path)
        .current_dir(dir.path())
        .args(["user", "create", "--name", "must-not-commit"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("refusing to commit"));
    assert!(
        !dir.path().join("state/users/users.json").exists(),
        "failed live authorization apply must not publish users.json"
    );
}

#[cfg(unix)]
#[test]
fn restore_rejects_cryptographically_mismatched_reality_keys() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let staging = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(staging.path().join("users")).unwrap();
    std::fs::create_dir_all(staging.path().join("reality")).unwrap();
    std::fs::write(staging.path().join("users/users.json"), "[]").unwrap();
    std::fs::write(
        staging.path().join("reality/private.key"),
        REALITY_PRIVATE_A,
    )
    .unwrap();
    std::fs::write(staging.path().join("reality/public.key"), REALITY_PUBLIC_B).unwrap();
    std::fs::write(staging.path().join("reality/short_id.txt"), "deadbeef").unwrap();
    let archive = dir.path().join("mismatched.tar");
    assert!(std::process::Command::new("tar")
        .arg("-cf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(".")
        .status()
        .unwrap()
        .success());

    admin(dir.path(), &cfg_path)
        .arg("restore")
        .arg(&archive)
        .assert()
        .failure()
        .stderr(predicates::str::contains("do not form one X25519 keypair"));
    assert!(!dir.path().join("state/reality/private.key").exists());
}

#[test]
#[cfg(unix)]
fn restore_rejects_archive_containing_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();
    std::fs::write(state_dir.join("reality/private.key"), "test-private-key").unwrap();
    std::fs::write(state_dir.join("reality/public.key"), "test-public-key").unwrap();
    // `init` writes all THREE files; a fixture with only two is not what a
    // real deployment looks like, and `restore` now (correctly) refuses a
    // partial REALITY keyset because restoring one would split-brain the
    // server against the subscription service.
    std::fs::write(state_dir.join("reality/short_id.txt"), "deadbeef").unwrap();

    // Hand-craft a malicious archive: a valid users.json plus a
    // `reality/private.key` that is a *symlink* to a file outside the
    // archive. If restore ever followed it, it would read/copy whatever
    // that target happens to contain instead of real key material.
    let staging = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(staging.path().join("users")).unwrap();
    std::fs::write(staging.path().join("users/users.json"), "[]").unwrap();
    std::fs::create_dir_all(staging.path().join("reality")).unwrap();
    let outside_secret = dir.path().join("outside-secret.txt");
    std::fs::write(&outside_secret, "not archive content").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_secret, staging.path().join("reality/private.key"))
        .unwrap();

    let archive_path = dir.path().join("malicious-backup.tar");
    let status = std::process::Command::new("tar")
        .arg("-cf")
        .arg(&archive_path)
        .arg("-C")
        .arg(staging.path())
        .arg(".")
        .status()
        .unwrap();
    assert!(status.success());

    admin(dir.path(), &cfg_path)
        .arg("restore")
        .arg(&archive_path)
        .assert()
        .failure()
        .stderr(predicates::str::contains("symlink"));
}

/// Split-brain regression (same class of bug `cmd_reality_rotate` exists
/// to prevent, reached via `restore` instead): `restore` installs the
/// archive's REALITY key material directly onto disk and then only
/// reloads sing-box (`regenerate_singbox_config`) — but vpn-subscription
/// caches the REALITY public key/short_id in memory at startup and has
/// no reload path, so restoring an OLDER backup whose key differs from
/// what's currently live must ALSO restart vpn-subscription, or the
/// subscription service keeps advertising a stale public key to every
/// client after the restore "succeeds". Uses a fake `systemctl` (found
/// via `PATH`) that logs every invocation, so this is observable without
/// a real systemd host.
#[cfg(unix)]
#[test]
fn restore_of_differing_reality_key_restarts_subscription_service_too() {
    let dir = tempfile::tempdir().unwrap();
    // A real sing-box binary (faked) is required here: without one,
    // `regenerate_singbox_config` skips straight to a "binary not found"
    // warning and never reaches the reload path at all, which would make
    // this test vacuously pass for the wrong reason.
    let singbox = fake_singbox(dir.path(), false);
    let cfg_path = write_deployment_toml_with_singbox(dir.path(), &singbox);
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();

    let systemctl = fake_systemctl(dir.path());
    let log_path = dir.path().join("systemctl.log");

    // A backup captured while key "A" was live.
    std::fs::write(state_dir.join("reality/private.key"), REALITY_PRIVATE_A).unwrap();
    std::fs::write(state_dir.join("reality/public.key"), REALITY_PUBLIC_A).unwrap();
    std::fs::write(state_dir.join("reality/short_id.txt"), "aaaaaaaa").unwrap();
    admin(dir.path(), &cfg_path)
        .args(["user", "create", "--name", "frank"])
        .assert()
        .success();
    let backup_path = dir.path().join("backup.tar");
    admin(dir.path(), &cfg_path)
        .args(["backup", "--output"])
        .arg(&backup_path)
        .assert()
        .success();

    // The live key material is later rotated to "B" (simulating time
    // passing between the backup and an operator restoring it, e.g. onto
    // a replacement host, or rolling back after a bad rotation).
    std::fs::write(state_dir.join("reality/private.key"), REALITY_PRIVATE_B).unwrap();
    std::fs::write(state_dir.join("reality/public.key"), REALITY_PUBLIC_B).unwrap();

    // Now restore the "A" backup over the live "B" key — the exact
    // scenario where subscription's cached public key would go stale.
    let augmented_path = std::env::join_paths(
        std::iter::once(systemctl.parent().unwrap().to_path_buf()).chain(
            std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
                .unwrap_or_default(),
        ),
    )
    .unwrap();

    let mut restore_cmd = admin(dir.path(), &cfg_path);
    restore_cmd
        .arg("restore")
        .arg(&backup_path)
        .env("PATH", augmented_path)
        .env("SYSTEMCTL_LOG", &log_path)
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(state_dir.join("reality/public.key")).unwrap(),
        REALITY_PUBLIC_A,
        "restore must actually install the archive's (older) key material"
    );

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(
        log.lines()
            .any(|l| l.starts_with("reload-or-restart sing-box")),
        "restore must reload sing-box so it serves the restored key; log:\n{log}"
    );
    assert!(
        log.lines()
            .any(|l| l.starts_with("reload-or-restart vpn-subscription")),
        "restore installed REALITY key material that differs from what was live, so \
         vpn-subscription (which caches the public key/short_id at startup) must be \
         restarted too, exactly like `init --rotate` does — otherwise it keeps serving \
         the old public key after a restore that reports success. systemctl log:\n{log}"
    );
}

/// A hostile archive must not be able to dictate the permission bits of the
/// live REALITY private key. `std::fs::copy` propagates the SOURCE file's
/// mode to the destination, and restore runs as root with tar restoring
/// archive modes verbatim — so a mode of 04777 in a tar header became the
/// mode of `/etc/vpn/compat/reality/private.key`, setuid bit and all, with
/// restore exiting 0 and printing success.
#[test]
#[cfg(unix)]
fn restore_never_widens_permissions_on_restored_secrets() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();
    std::fs::write(state_dir.join("reality/private.key"), REALITY_PRIVATE_B).unwrap();
    std::fs::write(state_dir.join("reality/public.key"), REALITY_PUBLIC_B).unwrap();
    std::fs::write(state_dir.join("reality/short_id.txt"), "deadbeef").unwrap();

    // Build an archive whose members carry hostile modes.
    let staging = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(staging.path().join("reality")).unwrap();
    std::fs::create_dir_all(staging.path().join("users")).unwrap();
    std::fs::write(staging.path().join("users/users.json"), "[]").unwrap();
    for (name, contents, mode) in [
        ("reality/private.key", REALITY_PRIVATE_A, 0o4777u32),
        ("reality/public.key", REALITY_PUBLIC_A, 0o666),
        ("reality/short_id.txt", "deadbeef", 0o666),
    ] {
        let p = staging.path().join(name);
        std::fs::write(&p, contents).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let archive = dir.path().join("hostile-modes.tar");
    assert!(std::process::Command::new("tar")
        .arg("-cf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(".")
        .status()
        .unwrap()
        .success());

    admin(dir.path(), &cfg_path)
        .arg("restore")
        .arg(&archive)
        .assert()
        .success();

    let mode = std::fs::metadata(state_dir.join("reality/private.key"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o7000,
        0,
        "restored private.key kept a setuid/setgid bit from the archive (mode {:o})",
        mode & 0o7777
    );
    assert_eq!(
        mode & 0o777,
        0o640,
        "private key must be root:sing-box readable"
    );
    // And the non-secret halves must not have been widened either.
    let pub_mode = std::fs::metadata(state_dir.join("reality/public.key"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        pub_mode & 0o7000,
        0,
        "restored public.key kept a setuid/setgid bit from the archive (mode {:o})",
        pub_mode & 0o7777
    );
}

/// An archive carrying a new REALITY private key but not the matching
/// public half must be refused outright. Restoring it installs the new
/// private key beside the host's OLD public key, so sing-box enforces one
/// key while the subscription service advertises another and every client
/// fails the handshake — the exact incident class this project exists to
/// prevent, previously reached via a command that printed "Restore applied
/// and validated" and exited 0.
#[test]
fn restore_rejects_a_partial_reality_keyset() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();
    std::fs::write(state_dir.join("reality/private.key"), "live-private-key").unwrap();
    std::fs::write(state_dir.join("reality/public.key"), "live-public-key").unwrap();
    std::fs::write(state_dir.join("reality/short_id.txt"), "deadbeef").unwrap();

    let staging = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(staging.path().join("reality")).unwrap();
    std::fs::create_dir_all(staging.path().join("users")).unwrap();
    std::fs::write(staging.path().join("users/users.json"), "[]").unwrap();
    std::fs::write(
        staging.path().join("reality/private.key"),
        "archive-private-key",
    )
    .unwrap();
    let archive = dir.path().join("partial.tar");
    assert!(std::process::Command::new("tar")
        .arg("-cf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(".")
        .status()
        .unwrap()
        .success());

    admin(dir.path(), &cfg_path)
        .arg("restore")
        .arg(&archive)
        .assert()
        .failure();

    // And it must not have touched live state on its way to refusing.
    assert_eq!(
        std::fs::read_to_string(state_dir.join("reality/private.key")).unwrap(),
        "live-private-key"
    );
    assert_eq!(
        std::fs::read_to_string(state_dir.join("reality/public.key")).unwrap(),
        "live-public-key"
    );
}

/// A FIFO in the archive previously blocked `std::fs::read` forever — while
/// holding the deployment-wide state lock, so every subsequent admin command
/// deadlocked too. It must be refused by entry type, and promptly.
#[test]
#[cfg(unix)]
fn restore_rejects_a_fifo_entry_without_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path());
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(state_dir.join("reality")).unwrap();
    std::fs::write(state_dir.join("reality/private.key"), "live-private-key").unwrap();
    std::fs::write(state_dir.join("reality/public.key"), "live-public-key").unwrap();
    std::fs::write(state_dir.join("reality/short_id.txt"), "deadbeef").unwrap();

    let staging = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(staging.path().join("users")).unwrap();
    let fifo = staging.path().join("users/users.json");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    if !status.success() {
        eprintln!("skipping: mkfifo unavailable");
        return;
    }
    let archive = dir.path().join("fifo.tar");
    assert!(std::process::Command::new("tar")
        .arg("-cf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(".")
        .status()
        .unwrap()
        .success());

    let started = std::time::Instant::now();
    admin(dir.path(), &cfg_path)
        .arg("restore")
        .arg(&archive)
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .failure();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "restore did not fail promptly on a FIFO entry"
    );
}
