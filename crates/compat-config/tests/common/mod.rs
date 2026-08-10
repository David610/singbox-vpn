//! Shared test-only helpers for the real-`sing-box` interoperability
//! tests (`reality_interop.rs`, `hysteria2_interop.rs`). A `tests/common/
//! mod.rs` (not `tests/common.rs`) is the standard Rust convention for a
//! helper module shared across integration test binaries without being
//! compiled as its own separate test target.
//!
//! This module is compiled independently into EACH consuming test
//! binary, so an item only one of them uses is legitimately unused from
//! the other's perspective — `dead_code` warnings would fire on
//! whichever binary doesn't call it. `#![allow(dead_code)]` here is the
//! standard accommodation for a shared `tests/common` module, not a
//! blanket suppression of a real lint elsewhere in this crate.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;

/// Kills a background thread's owning process/socket by dropping the
/// listener that thread loops on — `JoinHandle` alone doesn't stop a
/// blocking-accept loop, so the shutdown signal here is closing the
/// listener's port from the OS's perspective is not directly
/// controllable from Rust for a `std::net::TcpListener`; instead the
/// thread is left detached (a `#[test]` process exits at the end of the
/// test binary regardless, taking the thread with it) — acceptable for a
/// short-lived test-only HTTP target, never something this pattern
/// would be appropriate for in production code.
pub struct LocalHttpTarget {
    pub port: u16,
}

/// A minimal, dependency-free local HTTP target: accepts a connection,
/// reads (and discards) the request, replies with a fixed 200 OK body,
/// and closes — deterministic, no public/third-party network dependency
/// (unlike dialing `example.com` or any other external host), matching
/// `services/test-service`'s own "prove bytes flow end to end" contract
/// but self-contained here so these tests don't need a cross-crate
/// dependency or a tokio runtime just for one static response.
pub fn spawn_local_http_target() -> LocalHttpTarget {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test HTTP target");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = b"hello from vpn1 interop test target";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    LocalHttpTarget { port }
}

/// Drives an HTTP GET through the given SOCKS5 proxy port via a raw
/// socket (no `curl`/`reqwest` dependency) to `(host, port)`, and
/// returns true iff a `200 OK` response came back — proving the tunnel
/// actually carries real application traffic end-to-end, not just that
/// the TCP/SOCKS handshake completed.
pub fn socks5_http_get_is_200(socks_port: u16, host: &str, port: u16) -> bool {
    let mut stream = match std::net::TcpStream::connect(("127.0.0.1", socks_port)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // The REALITY server dials its decoy as part of every handshake, so
    // this covers a TCP connect plus two TLS handshakes. The decoy is now
    // local (see `spawn_local_tls13_decoy`), so this is generous rather
    // than load-bearing — a genuine protocol failure fails in milliseconds,
    // well before the timeout is reached, and this bound exists only so a
    // wedged process fails the test instead of hanging it.
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();
    // SOCKS5 greeting: no auth.
    if stream.write_all(&[0x05, 0x01, 0x00]).is_err() {
        return false;
    }
    let mut buf = [0u8; 2];
    if stream.read_exact(&mut buf).is_err() || buf != [0x05, 0x00] {
        return false;
    }
    // CONNECT host:port.
    let host_bytes = host.as_bytes();
    if host_bytes.is_empty() || host_bytes.len() > 255 {
        return false;
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8];
    req.extend_from_slice(host_bytes);
    req.extend_from_slice(&port.to_be_bytes());
    if stream.write_all(&req).is_err() {
        return false;
    }
    let mut reply = [0u8; 4];
    if stream.read_exact(&mut reply).is_err() || reply[1] != 0x00 {
        return false;
    }
    let addr_len = match reply[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut l = [0u8; 1];
            if stream.read_exact(&mut l).is_err() {
                return false;
            }
            l[0] as usize
        }
        _ => return false,
    };
    let mut skip = vec![0u8; addr_len + 2];
    if stream.read_exact(&mut skip).is_err() {
        return false;
    }
    let request = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

/// A local TLS 1.3 server usable as a REALITY decoy ("handshake server").
///
/// REALITY's server genuinely must complete a TLS handshake against a real
/// TLS 1.3 endpoint — that is inherent to the protocol. What is NOT inherent
/// is that the endpoint be a third-party CDN: dialing `www.microsoft.com`
/// made a merge-gating test depend on which Akamai edge answered, and it is
/// precisely how the historical CI flake arose (see
/// `reality_decoy_budget.rs`). A locally-controlled decoy keeps the protocol
/// behaviour real and makes the outcome deterministic.
///
/// To be usable by sing-box's REALITY implementation the decoy must:
///   * negotiate TLS **1.3** (`hs.hello.supportedVersion != VersionTLS13` aborts);
///   * offer an **X25519** (or X25519MLKEM768) key share — OpenSSL's default;
///   * emit the middlebox-compat **ChangeCipherSpec of exactly 6 bytes** —
///     OpenSSL does this by default;
///   * keep every TLS record at or under metacubex/utls's hard-coded
///     `realitySize` budget of **8192 bytes**.
///
/// The SNI must be a hostname, not an IP literal (uTLS omits SNI for IPs, and
/// the REALITY server matches `config.ServerNames[clientHello.serverName]`),
/// so `localhost` is used as the decoy hostname throughout.
pub struct LocalDecoy {
    pub port: u16,
    pub hostname: &'static str,
    _dir: tempfile::TempDir,
    child: std::process::Child,
}

impl Drop for LocalDecoy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How large a certificate the decoy should present. This is the single
/// variable that decides whether sing-box's REALITY server accepts the
/// decoy's flight or aborts with "processed invalid connection".
pub enum DecoyCertSize {
    /// A minimal single self-signed cert — every record stays well under
    /// the 8192-byte budget.
    Small,
    /// Inflated with hundreds of SANs so the Certificate record exceeds
    /// 8192 bytes, reproducing the historical CI failure deterministically.
    OverBudget,
}

pub fn spawn_local_tls13_decoy(size: DecoyCertSize) -> Option<LocalDecoy> {
    let openssl_present = std::process::Command::new("openssl")
        .arg("version")
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    if !openssl_present {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let cert = dir.path().join("decoy-cert.pem");
    let key = dir.path().join("decoy-key.pem");

    let mut req = std::process::Command::new("openssl");
    req.arg("req")
        .arg("-x509")
        .arg("-newkey")
        .arg("rsa:2048")
        .arg("-days")
        .arg("1")
        .arg("-nodes")
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .arg("-subj")
        .arg("/CN=localhost");
    if let DecoyCertSize::OverBudget = size {
        // Each SAN adds ~20 bytes to the leaf certificate; enough of them
        // push the Certificate record past REALITY's 8192-byte budget.
        //
        // The count is deliberately well past the threshold rather than
        // just over it: DER encoding of the serial and signature varies by
        // a few bytes per generation, and a margin that put the record near
        // 8192 produced a test that passed only ~75% of the time (observed:
        // one run framed the Certificate at 7938 and the tunnel worked).
        // Fixed-width labels keep the size stable across runs too.
        let sans: Vec<String> = (0..800)
            .map(|i| format!("DNS:pad{i:04}.localhost"))
            .collect();
        req.arg("-addext")
            .arg(format!("subjectAltName=DNS:localhost,{}", sans.join(",")));
    } else {
        req.arg("-addext").arg("subjectAltName=DNS:localhost");
    }
    let out = req.output().ok()?;
    if !out.status.success() {
        return None;
    }

    let port = free_port();
    let child = std::process::Command::new("openssl")
        .arg("s_server")
        .arg("-accept")
        .arg(port.to_string())
        .arg("-cert")
        .arg(&cert)
        .arg("-key")
        .arg(&key)
        .arg("-tls1_3")
        .arg("-quiet")
        .arg("-naccept")
        .arg("50")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let decoy = LocalDecoy {
        port,
        hostname: "localhost",
        _dir: dir,
        child,
    };
    if !wait_for_port(port, std::time::Duration::from_secs(5)) {
        return None;
    }
    Some(decoy)
}

/// Waits until `needle` appears in the log file at `path`.
///
/// Used instead of probing the REALITY port with a bare TCP connect. A
/// connect-then-drop probe sends no ClientHello, so the REALITY server
/// correctly logs it as `REALITY: processed invalid connection` — meaning
/// the harness manufactured, on every single run, the exact error string
/// that three separate commits then tried to explain. Waiting on the
/// server's own readiness line removes that phantom connection entirely.
pub fn wait_for_log_line(
    path: &std::path::Path,
    needle: &str,
    timeout: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(contents) = std::fs::read_to_string(path) {
            if contents.contains(needle) {
                return true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    false
}

pub fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

pub fn wait_for_port(port: u16, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

pub struct SingBox {
    pub path: std::path::PathBuf,
}

impl SingBox {
    pub fn find() -> Option<Self> {
        if let Ok(p) = std::env::var("SING_BOX_BIN") {
            let path = std::path::PathBuf::from(p);
            if path.is_file() {
                return Some(Self { path });
            }
        }
        let output = std::process::Command::new("sing-box")
            .arg("version")
            .output()
            .ok()?;
        if output.status.success() {
            return Some(Self {
                path: std::path::PathBuf::from("sing-box"),
            });
        }
        None
    }

    pub fn run(&self, config_path: &std::path::Path) -> std::process::Child {
        std::process::Command::new(&self.path)
            .arg("run")
            .arg("-c")
            .arg(config_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sing-box run")
    }

    /// Like `run`, but redirects stdout+stderr to `log_path` instead of
    /// discarding them, so a CI failure can be diagnosed from the actual
    /// sing-box log instead of a bare "assertion failed" with no
    /// context — real handshake failures print exactly what stage
    /// failed (REALITY handshake, decoy dial, etc.).
    ///
    /// The trace-level server log this captures is what finally identified
    /// the real cause of the recurring CI failure — and it was none of the
    /// three things previously committed as "the root cause". See
    /// `reality_decoy_budget.rs` for the mechanism and the reproducer.
    pub fn run_logged(
        &self,
        config_path: &std::path::Path,
        log_path: &std::path::Path,
    ) -> std::process::Child {
        let log_file = std::fs::File::create(log_path).expect("create sing-box log file");
        let log_file_err = log_file.try_clone().expect("clone log file handle");
        std::process::Command::new(&self.path)
            .arg("run")
            .arg("-c")
            .arg(config_path)
            .stdout(std::process::Stdio::from(log_file))
            .stderr(std::process::Stdio::from(log_file_err))
            .spawn()
            .expect("spawn sing-box run")
    }
}

/// Reads back a log file written by `SingBox::run_logged`, for printing
/// on test failure. Never fails the calling test if the log can't be
/// read — this is diagnostic best-effort, not a correctness assertion.
pub fn read_log(log_path: &std::path::Path) -> String {
    std::fs::read_to_string(log_path)
        .unwrap_or_else(|e| format!("(could not read log at {log_path:?}: {e})"))
}

pub struct Guard(pub std::process::Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
