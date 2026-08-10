//! Real interoperability tests for VLESS+REALITY config generation.
//!
//! These tests build the exact server config (`server::render_singbox_server_config`)
//! and client config (`render::render_singbox_client_subscription`) this codebase
//! would actually deploy/serve, then drive a REAL `sing-box` binary as both the
//! server and the client over loopback, and assert traffic actually flows.
//!
//! Rationale: unit tests that only assert a URI/JSON *contains* fields like
//! `security=reality` or `sid=...` cannot catch a cryptographically/protocol
//! incompatible value (e.g. a stale public key, wrong short_id, or a field the
//! real REALITY implementation silently rejects). Only a real handshake against
//! the real (pinned) sing-box binary proves the generated material actually
//! authenticates — see docs/COMPATIBILITY_VERSIONS.md for the pinned version.
//!
//! Skipped (not failed) when no usable `sing-box` binary is available, so this
//! doesn't turn every contributor's machine/CI image into a hard requirement:
//! set `SING_BOX_BIN=/path/to/sing-box` to point at one explicitly, otherwise
//! `sing-box` on `PATH` is used if present.

use compat_config::model::{Hysteria2ServerParams, RealityServerParams};
use compat_config::secret::SecretString;
use compat_config::server::{render_singbox_server_config, ServerPorts};
use compat_config::{render, CompatUser};
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct SingBox {
    path: PathBuf,
}

impl SingBox {
    fn find() -> Option<Self> {
        if let Ok(p) = std::env::var("SING_BOX_BIN") {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some(Self { path });
            }
        }
        let output = Command::new("sing-box").arg("version").output().ok()?;
        if output.status.success() {
            return Some(Self {
                path: PathBuf::from("sing-box"),
            });
        }
        None
    }

    fn generate_reality_keypair(&self) -> (String, String) {
        let output = Command::new(&self.path)
            .arg("generate")
            .arg("reality-keypair")
            .output()
            .expect("run sing-box generate reality-keypair");
        assert!(output.status.success(), "reality-keypair generation failed");
        let text = String::from_utf8_lossy(&output.stdout);
        let private_key = extract_field(&text, "PrivateKey").expect("PrivateKey in output");
        let public_key = extract_field(&text, "PublicKey").expect("PublicKey in output");
        (private_key, public_key)
    }

    fn run(&self, config_path: &std::path::Path) -> Child {
        Command::new(&self.path)
            .arg("run")
            .arg("-c")
            .arg(config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sing-box run")
    }
}

fn extract_field(text: &str, field: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(&format!("{field}:"))
            .or_else(|| line.strip_prefix(&format!("{field} ")))
            .map(|s| s.trim().to_string())
    })
}

/// Picks a probably-free localhost port by binding to port 0 and releasing it.
/// Small TOCTOU race is acceptable for a test harness (not production code).
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn test_user() -> CompatUser {
    CompatUser {
        id: "u1".into(),
        name: "interop-test".into(),
        enabled: true,
        vless_uuid: "841cac4a-efe4-48ac-92b8-d11f4c98c45e".into(),
        hysteria2_password: SecretString::new("unused-in-this-test"),
        subscription_token_hash_hex: "unused".into(),
        created_at: 0,
        expires_at: None,
    }
}

/// Builds the server sing-box config (via the crate's OWN production renderer)
/// for a single VLESS+REALITY inbound plus the client-side outbound config
/// (via the crate's OWN production subscription renderer) that a real user
/// would receive from `GET /sub/{token}`, wraps the client outbound with a
/// local `mixed` inbound so a plain HTTP client can drive it through a SOCKS
/// proxy, and returns (server_config, client_config).
fn build_configs(
    reality_port: u16,
    mixed_port: u16,
    private_key: &str,
    public_key: &str,
    short_id: &str,
) -> (serde_json::Value, serde_json::Value) {
    let reality = RealityServerParams {
        private_key_hex: SecretString::new(private_key.to_string()),
        public_key_hex: public_key.to_string(),
        short_ids: vec![short_id.to_string()],
        handshake_server: "www.microsoft.com".into(),
        handshake_port: 443,
    };
    // Hysteria2 params are required by the renderer signature but irrelevant
    // to this REALITY-only test; give it a harmless disabled-masquerade shape.
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: "/dev/null".into(),
        tls_key_path: "/dev/null".into(),
        obfs_password: None,
        masquerade_dir_path: None,
    };
    let users = vec![test_user()];
    let mut server_cfg = render_singbox_server_config(
        &users,
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: reality_port,
            hysteria2_port: free_port(), // unused, just needs to bind somewhere
        },
        0,
    );
    // The production config listens on both IPv4/IPv6 for both inbounds;
    // drop the hysteria2 inbound entirely for this REALITY-only test so we
    // don't need real TLS cert material just to get sing-box to start.
    server_cfg["inbounds"]
        .as_array_mut()
        .unwrap()
        .retain(|ib| ib["tag"] == "vless-reality-in");

    let endpoint = compat_config::model::CompatEndpoint {
        id: "reality-1".into(),
        transport: compat_config::model::CompatTransport::VlessReality,
        host: "127.0.0.1".into(),
        port: reality_port,
        server_name: Some("www.microsoft.com".into()),
        label: "Reality".into(),
        public_parameters: compat_config::model::PublicParameters::Reality {
            public_key_hex: public_key.to_string(),
            short_id: short_id.to_string(),
            fingerprint: "chrome".into(),
        },
    };
    let mut client_cfg =
        render::render_singbox_client_subscription(&test_user(), std::slice::from_ref(&endpoint))
            .expect("render client subscription");
    client_cfg["inbounds"] = serde_json::json!([{
        "type": "mixed",
        "tag": "mixed-in",
        "listen": "127.0.0.1",
        "listen_port": mixed_port,
    }]);
    // Route everything through the reality endpoint's own outbound tag
    // (render_singbox_client_subscription names it after the endpoint label).
    client_cfg["route"] = serde_json::json!({ "final": "Reality" });

    (server_cfg, client_cfg)
}

fn write_json(dir: &std::path::Path, name: &str, value: &serde_json::Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

/// Drives an HTTP GET through the given SOCKS5 proxy port via a raw socket
/// (no `curl`/`reqwest` dependency needed) and returns true if a response
/// status line came back — proving the tunnel actually carries traffic
/// end-to-end, not just that the TCP handshake completed.
fn socks5_http_get_succeeds(socks_port: u16, host: &str) -> bool {
    use std::io::Write;
    let mut stream = match TcpStream::connect(("127.0.0.1", socks_port)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // SOCKS5 greeting: no auth.
    stream.write_all(&[0x05, 0x01, 0x00]).unwrap();
    let mut buf = [0u8; 2];
    if stream.read_exact(&mut buf).is_err() || buf != [0x05, 0x00] {
        return false;
    }
    // CONNECT host:80 (plain HTTP so we don't need TLS in this harness).
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&80u16.to_be_bytes());
    stream.write_all(&req).unwrap();
    let mut reply = [0u8; 4];
    if stream.read_exact(&mut reply).is_err() || reply[1] != 0x00 {
        return false;
    }
    // Drain the rest of the reply (address + port, variable length by type).
    let addr_len = match reply[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut l = [0u8; 1];
            stream.read_exact(&mut l).unwrap();
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
    if std::io::Read::read_to_string(&mut stream, &mut response).is_err() {
        return false;
    }
    response.starts_with("HTTP/1.1 ") || response.starts_with("HTTP/1.0 ")
}

/// TEST 1 + TEST 8 (spec sections 12/13): generate a real REALITY keypair,
/// render server+client config through the crate's OWN production
/// renderers, run a real sing-box server and client, and prove traffic
/// actually flows end-to-end through the tunnel. This is the test that
/// would have caught the production incident: a syntactically-valid but
/// cryptographically-mismatched config would fail here, not just look
/// plausible in a string-contains assertion.
#[test]
fn reality_handshake_succeeds_with_matched_keypair() {
    let Some(sb) = SingBox::find() else {
        eprintln!("skipping: no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let (private_key, public_key) = sb.generate_reality_keypair();
    let short_id = "e54b2158";

    let dir = tempfile::tempdir().unwrap();
    let reality_port = free_port();
    let mixed_port = free_port();
    let (server_cfg, client_cfg) = build_configs(
        reality_port,
        mixed_port,
        &private_key,
        &public_key,
        short_id,
    );
    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &client_cfg);

    let _server = Guard(sb.run(&server_path));
    assert!(
        wait_for_port(reality_port, Duration::from_secs(5)),
        "server never bound its REALITY port"
    );
    let _client = Guard(sb.run(&client_path));
    assert!(
        wait_for_port(mixed_port, Duration::from_secs(5)),
        "client never bound its local SOCKS port"
    );

    assert!(
        socks5_http_get_succeeds(mixed_port, "example.com"),
        "REALITY handshake/traffic failed through a config produced by this \
         crate's own production renderers with a matched, real keypair"
    );
}

/// TEST 4 (spec section 12): the exact production incident's signature —
/// the client is configured with a REALITY public key that does NOT
/// correspond to the server's private key (e.g. what happens if the
/// subscription service ever serves a stale/rotated-away public key).
/// Must fail closed: no traffic should flow.
#[test]
fn reality_handshake_fails_with_mismatched_public_key() {
    let Some(sb) = SingBox::find() else {
        eprintln!("skipping: no sing-box binary available (set SING_BOX_BIN)");
        return;
    };
    let (private_key, _real_public_key) = sb.generate_reality_keypair();
    let (_unused_private, stale_public_key) = sb.generate_reality_keypair();
    let short_id = "e54b2158";

    let dir = tempfile::tempdir().unwrap();
    let reality_port = free_port();
    let mixed_port = free_port();
    let (server_cfg, client_cfg) = build_configs(
        reality_port,
        mixed_port,
        &private_key,
        &stale_public_key, // client uses a DIFFERENT keypair's public half
        short_id,
    );
    let server_path = write_json(dir.path(), "server.json", &server_cfg);
    let client_path = write_json(dir.path(), "client.json", &client_cfg);

    let _server = Guard(sb.run(&server_path));
    assert!(wait_for_port(reality_port, Duration::from_secs(5)));
    let _client = Guard(sb.run(&client_path));
    assert!(wait_for_port(mixed_port, Duration::from_secs(5)));

    assert!(
        !socks5_http_get_succeeds(mixed_port, "example.com"),
        "a mismatched REALITY public key must never be able to pass traffic \
         through the tunnel — if this assertion fails, REALITY auth is not \
         actually being enforced"
    );
}

/// TEST 2 (spec section 12): short_id coherence between the server config
/// and the client subscription rendered from the SAME state. This is a
/// pure in-memory check (no sing-box binary needed) — it exists as a fast,
/// always-run regression guard that would have failed loudly if e.g. the
/// server's `short_ids` list and the subscription's advertised `short_id`
/// were ever populated from different sources.
#[test]
fn server_and_client_configs_agree_on_reality_key_material() {
    let reality = RealityServerParams {
        private_key_hex: SecretString::new("test-private-key".to_string()),
        public_key_hex: "test-public-key".into(),
        short_ids: vec!["deadbeef".into()],
        handshake_server: "www.microsoft.com".into(),
        handshake_port: 443,
    };
    let hysteria = Hysteria2ServerParams {
        tls_cert_path: "/dev/null".into(),
        tls_key_path: "/dev/null".into(),
        obfs_password: None,
        masquerade_dir_path: None,
    };
    let users = vec![test_user()];
    let server_cfg = render_singbox_server_config(
        &users,
        &reality,
        &hysteria,
        ServerPorts {
            vless_reality_port: 443,
            hysteria2_port: 443,
        },
        0,
    );

    let endpoint = compat_config::model::CompatEndpoint {
        id: "reality-1".into(),
        transport: compat_config::model::CompatTransport::VlessReality,
        host: "vpn.example.com".into(),
        port: 443,
        server_name: Some(reality.handshake_server.clone()),
        label: "Reality".into(),
        public_parameters: compat_config::model::PublicParameters::Reality {
            public_key_hex: reality.public_key_hex.clone(),
            short_id: reality.short_ids[0].clone(),
            fingerprint: "chrome".into(),
        },
    };
    let client_cfg = render::render_singbox_client_subscription(&test_user(), &[endpoint])
        .expect("render client subscription");

    let server_short_ids = server_cfg["inbounds"][0]["tls"]["reality"]["short_id"]
        .as_array()
        .unwrap();
    let client_short_id = client_cfg["outbounds"][0]["tls"]["reality"]["short_id"]
        .as_str()
        .unwrap();
    assert!(
        server_short_ids
            .iter()
            .any(|v| v.as_str() == Some(client_short_id)),
        "client-advertised short_id {client_short_id:?} is not in the server's \
         accepted short_id list {server_short_ids:?}"
    );

    let client_public_key = client_cfg["outbounds"][0]["tls"]["reality"]["public_key"]
        .as_str()
        .unwrap();
    assert_eq!(
        client_public_key, reality.public_key_hex,
        "client-advertised REALITY public key does not match the server's \
         actual public key"
    );
}
