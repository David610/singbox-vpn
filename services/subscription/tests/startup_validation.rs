//! Regression tests for the subscription service's startup-time REALITY
//! material validation. `AppState.endpoints` is built once at process
//! start and never rereread (see `main.rs`), so a corrupt/empty
//! `public.key`/`short_id.txt` file used to be baked in for the whole
//! process lifetime instead of being caught — every subscription that
//! process ever served would then be a syntactically-valid-but-broken
//! `vless://...&pbk=&sid=...` URI at a real HTTP 200. The service must
//! now refuse to start at all in that case.

use std::path::Path;
use std::time::Duration;

fn write_deployment_toml(dir: &Path, sub_port: u16) -> std::path::PathBuf {
    let cfg_path = dir.join("deployment.toml");
    let state_dir = dir.join("state");
    let toml = format!(
        r#"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"
state_dir = "{state}"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = {sub_port}
"#,
        state = state_dir.to_string_lossy().replace('\\', "\\\\"),
    );
    std::fs::write(&cfg_path, toml).unwrap();
    cfg_path
}

/// A real, valid X25519 public key (matches the deterministic test
/// vector used elsewhere in this workspace, e.g.
/// crates/compat-config/src/credentials.rs's own unit tests).
const VALID_PUBLIC_KEY: &str = "pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk";
const VALID_SHORT_ID: &str = "deadbeef";

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn write_reality_material(dir: &Path, public_key: &str, short_id: &str) {
    let reality_dir = dir.join("state/reality");
    std::fs::create_dir_all(&reality_dir).unwrap();
    std::fs::write(reality_dir.join("public.key"), public_key).unwrap();
    std::fs::write(reality_dir.join("short_id.txt"), short_id).unwrap();
}

fn run_subscription(cfg_path: &Path, timeout: Duration) -> (Option<i32>, String, String) {
    let mut child = std::process::Command::cargo_like_bin()
        .arg("--config")
        .arg(cfg_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn subscription binary");

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait subscription binary") {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if status.is_none() {
        // Still running after the timeout — treat as "did not exit",
        // kill it so the test doesn't leak a bound port.
        let _ = child.kill();
        let _ = child.wait();
        return (None, String::new(), String::new());
    }
    let output = child.wait_with_output().expect("collect output");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Resolves the compiled `subscription` binary the same way
/// `assert_cmd::Command::cargo_bin` would, without adding a new
/// dev-dependency for one lookup — `env!("CARGO_BIN_EXE_subscription")`
/// is Cargo's own supported mechanism for exactly this.
trait CargoLikeBin {
    fn cargo_like_bin() -> std::process::Command;
}
impl CargoLikeBin for std::process::Command {
    fn cargo_like_bin() -> std::process::Command {
        std::process::Command::new(env!("CARGO_BIN_EXE_subscription"))
    }
}

#[test]
fn refuses_to_start_with_empty_public_key() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path(), free_port());
    write_reality_material(dir.path(), "", VALID_SHORT_ID);

    let (code, _stdout, stderr) = run_subscription(&cfg_path, Duration::from_secs(5));
    assert_eq!(
        code,
        Some(1),
        "must exit non-zero (not hang/bind) on an empty public key; stderr={stderr}"
    );
    assert!(
        stderr.contains("reality public key"),
        "error must name the actual problem:\n{stderr}"
    );
}

#[test]
fn refuses_to_start_with_truncated_public_key() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path(), free_port());
    write_reality_material(dir.path(), "dG9vc2hvcnQ", VALID_SHORT_ID);

    let (code, _stdout, stderr) = run_subscription(&cfg_path, Duration::from_secs(5));
    assert_eq!(
        code,
        Some(1),
        "must exit non-zero on a truncated/malformed public key; stderr={stderr}"
    );
    assert!(
        stderr.contains("reality public key"),
        "error must name the actual problem:\n{stderr}"
    );
}

#[test]
fn refuses_to_start_with_malformed_public_key() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path(), free_port());
    write_reality_material(dir.path(), "not valid base64!!!", VALID_SHORT_ID);

    let (code, _stdout, stderr) = run_subscription(&cfg_path, Duration::from_secs(5));
    assert_eq!(code, Some(1), "must exit non-zero; stderr={stderr}");
}

#[test]
fn refuses_to_start_with_empty_short_id() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path(), free_port());
    write_reality_material(dir.path(), VALID_PUBLIC_KEY, "");

    let (code, _stdout, stderr) = run_subscription(&cfg_path, Duration::from_secs(5));
    assert_eq!(
        code,
        Some(1),
        "must exit non-zero on an empty short_id; stderr={stderr}"
    );
    assert!(
        stderr.contains("short_id"),
        "error must name the actual problem:\n{stderr}"
    );
}

#[test]
fn refuses_to_start_with_non_hex_short_id() {
    let dir = tempfile::tempdir().unwrap();
    let cfg_path = write_deployment_toml(dir.path(), free_port());
    write_reality_material(dir.path(), VALID_PUBLIC_KEY, "not-hex!");

    let (code, _stdout, stderr) = run_subscription(&cfg_path, Duration::from_secs(5));
    assert_eq!(code, Some(1), "must exit non-zero; stderr={stderr}");
}

#[test]
fn starts_successfully_with_valid_reality_material() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let cfg_path = write_deployment_toml(dir.path(), port);
    write_reality_material(dir.path(), VALID_PUBLIC_KEY, VALID_SHORT_ID);

    let mut child = std::process::Command::cargo_like_bin()
        .arg("--config")
        .arg(&cfg_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn subscription binary");

    // Give it a moment to either bind successfully or exit with an error
    // — with valid material it must still be running (not have exited)
    // after a short wait.
    std::thread::sleep(Duration::from_millis(500));
    let still_running = child.try_wait().expect("try_wait").is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        still_running,
        "service must stay up (not exit) when REALITY material is valid"
    );
}
