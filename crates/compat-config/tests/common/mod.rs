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
/// (unlike dialing `example.com` or any other external host), proving
/// bytes flow end to end while staying self-contained here so these
/// tests don't need a cross-crate dependency or a tokio runtime just for
/// one static response.
pub fn spawn_local_http_target() -> LocalHttpTarget {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test HTTP target");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = b"hello from singbox-vpn interop test target";
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
///     OpenSSL does this by default.
///
/// The old 1.13.x pin additionally required every TLS record to stay under
/// an 8192-byte REALITY budget. The 1.14.1 interop suite deliberately keeps
/// a second, much larger certificate fixture to prove that limitation does
/// not regress.
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

/// How large a certificate the decoy should present. The large shape
/// reproduces the certificate flight that broke the historical 1.13.x path.
pub enum DecoyCertSize {
    /// A minimal single self-signed cert — every record stays well under
    /// the 8192-byte budget.
    Small,
    /// Inflated with hundreds of SANs so the Certificate record exceeds
    /// the historical 8192-byte limit. Current sing-box must still carry it.
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
        // push the Certificate record past the historical 8192-byte budget.
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

/// A port for a process the harness is about to start: unused on the TCP and
/// UDP wildcards, outside the kernel's ephemeral port range, and never handed
/// out twice by one test process.
///
/// "Bind port 0, read the number, drop the socket" returned a port the kernel
/// was free to assign again immediately — to the next such probe (the exit and
/// the relay were given the same port) or to a harness socket bound to port 0
/// on a specific loopback address (the exit tap, an HTTP target). The sing-box
/// that later bound it exited with "address already in use" while the other
/// socket held the port, and the scenario failed much later as a route that
/// never came up. Port-0 binds and outgoing connections both draw from the
/// ephemeral range, so a port chosen outside it can only be claimed by the
/// process it was handed to.
pub fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CURSOR: AtomicU32 = AtomicU32::new(u32::MAX);
    let (low, high) = harness_port_range();
    let span = high - low + 1;
    // Different test processes start at different offsets.
    let _ = CURSOR.compare_exchange(
        u32::MAX,
        std::process::id() % span,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    for _ in 0..span {
        let port = (low + CURSOR.fetch_add(1, Ordering::SeqCst) % span) as u16;
        if port_unused(port) {
            return port;
        }
    }
    panic!("no unused port left in the harness range {low}-{high}");
}

/// The widest block of ports above 10000 that the kernel never assigns on its
/// own (Linux: `ip_local_port_range`; elsewhere the Linux default is assumed,
/// which lies outside the BSD/macOS ephemeral range too).
fn harness_port_range() -> (u32, u32) {
    let (ephemeral_low, ephemeral_high) =
        std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
            .ok()
            .and_then(|text| {
                let mut bounds = text.split_whitespace().map(str::parse::<u32>);
                Some((bounds.next()?.ok()?, bounds.next()?.ok()?))
            })
            .unwrap_or((32768, 60999));
    let below: (u32, u32) = (10_000, ephemeral_low.saturating_sub(1));
    let above: (u32, u32) = (ephemeral_high + 1, 65_535);
    let (low, high) = if below.1.saturating_sub(below.0) >= above.1.saturating_sub(above.0) {
        below
    } else {
        above
    };
    assert!(
        high >= low + 999,
        "ephemeral port range {ephemeral_low}-{ephemeral_high} leaves no block for harness ports"
    );
    (low, high)
}

fn port_unused(port: u16) -> bool {
    use std::net::{Ipv4Addr, Ipv6Addr, UdpSocket};
    let free = |bound: std::io::Result<()>| !matches!(bound, Err(e) if e.kind() == std::io::ErrorKind::AddrInUse);
    free(TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).map(drop))
        && free(TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)).map(drop))
        && free(UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).map(drop))
        && free(UdpSocket::bind((Ipv6Addr::UNSPECIFIED, port)).map(drop))
}

/// Blocks until `child` itself holds a socket on every inbound port of the
/// sing-box config it runs (UDP for QUIC inbounds, a LISTEN socket otherwise).
/// Fails as soon as the process exits, or after `timeout`.
///
/// Ownership is read from `/proc`: "some socket listens on that port" is not
/// the prerequisite a scenario depends on — another process can hold the port
/// while this sing-box has already exited on a bind error. Nothing connects to
/// the port either: a connect-and-drop probe against a REALITY listener
/// produces exactly the "processed invalid connection" noise the privacy
/// assertions must not see.
pub fn wait_until_serving(
    child: &mut std::process::Child,
    config: &std::path::Path,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let text = std::fs::read_to_string(config).map_err(|e| format!("read {config:?}: {e}"))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {config:?}: {e}"))?;
    let inbounds: Vec<(u16, bool)> = doc["inbounds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|inbound| {
            let port = u16::try_from(inbound["listen_port"].as_u64()?).ok()?;
            let udp = matches!(
                inbound["type"].as_str(),
                Some("hysteria2" | "hysteria" | "tuic")
            );
            Some((port, udp))
        })
        .collect();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("process exited ({status}) before serving"));
        }
        let owned = socket_inodes(child.id());
        let missing: Vec<u16> = inbounds
            .iter()
            .filter(|(port, udp)| !socket_bound(&owned, *port, *udp))
            .map(|(port, _)| *port)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("not serving on {missing:?} after {timeout:?}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn socket_inodes(pid: u32) -> std::collections::HashSet<String> {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|fd| std::fs::read_link(fd.path()).ok())
        .filter_map(|target| {
            let target = target.to_str()?;
            Some(
                target
                    .strip_prefix("socket:[")?
                    .strip_suffix(']')?
                    .to_string(),
            )
        })
        .collect()
}

/// `/proc/net/{tcp,udp}{,6}` rows: local address in column 1, TCP state in
/// column 3 (`0A` = LISTEN), socket inode in column 9.
fn socket_bound(owned: &std::collections::HashSet<String>, port: u16, udp: bool) -> bool {
    let suffix = format!(":{port:04X}");
    let tables = if udp {
        ["/proc/net/udp", "/proc/net/udp6"]
    } else {
        ["/proc/net/tcp", "/proc/net/tcp6"]
    };
    tables.iter().any(|table| {
        std::fs::read_to_string(table).is_ok_and(|text| {
            text.lines().skip(1).any(|row| {
                let columns: Vec<&str> = row.split_whitespace().collect();
                columns.len() > 9
                    && columns[1].ends_with(&suffix)
                    && (udp || columns[3] == "0A")
                    && owned.contains(columns[9])
            })
        })
    })
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

    /// Runs `sing-box check -c <config_path>` against the real pinned
    /// binary and returns the captured output. Cheaper and faster than
    /// `run`/`run_logged` for tests that only need to prove a generated
    /// config is syntactically/schema valid to sing-box, not drive a
    /// live handshake — no socket, no background process, no need for a
    /// `Guard` to clean it up.
    pub fn check(&self, config_path: &std::path::Path) -> std::process::Output {
        std::process::Command::new(&self.path)
            .arg("check")
            .arg("-c")
            .arg(config_path)
            .output()
            .expect("run sing-box check")
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
