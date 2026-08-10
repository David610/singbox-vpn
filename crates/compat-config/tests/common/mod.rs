//! Shared test-only helpers for the real-`sing-box` interoperability
//! tests (`reality_interop.rs`, `hysteria2_interop.rs`). A `tests/common/
//! mod.rs` (not `tests/common.rs`) is the standard Rust convention for a
//! helper module shared across integration test binaries without being
//! compiled as its own separate test target.

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
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
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
}

pub struct Guard(pub std::process::Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
