//! LOCAL SYSTEM TESTS (S1–S17) for the two-hop Privacy+ route.
//!
//! Evidence class: local automated system evidence (CI-VERIFIED when the
//! `singbox-validate` job runs it). Real sing-box processes, loopback only.
//! This is NOT server-verified, device-verified, or network-verified
//! evidence: every hop shares one kernel, one loopback interface and one
//! clock, so provider separation, packet-capture knowledge separation,
//! real reachability, latency and leak behaviour are out of scope here.
//!
//! Topology (distinct loopback addresses, deterministic roles):
//!
//! ```text
//! client (sing-box, SOCKS on 127.0.0.1, dials from 127.0.0.6)
//!   ├─ "Germany · Direct"      ─────────────────────► tap ─► exit 127.0.0.3 ─► target 127.0.0.4
//!   └─ "Germany · via Russia"  ─► relay 127.0.0.2 ───► tap ─► exit 127.0.0.3 ─► target 127.0.0.4
//! undeclared destinations: 127.0.0.5, other ports on 127.0.0.3/127.0.0.4
//! ```
//!
//! Every server document comes from `render_server_config_for_deployment`,
//! every client document from the first-party provisioning path
//! (`DeploymentConfig::served_endpoints` + `provisioning_document_*`), and
//! every applied config goes through `apply_config_atomically` validated by
//! the real `sing-box check`. The only test-side additions are
//! client-owned: a SOCKS inbound, the selected route, and the device's
//! source address. The tap in front of the exit records connection sources.
//!
//! The scenario functions take a [`Lab`] describing addresses, not
//! hard-coded processes, so the same scenario list is the checklist for
//! the later real-VPS acceptance phase (docs/TWO_HOP_SYSTEM_TESTS.md).
//!
//! Requires Linux (127.0.0.0/8 is routable to lo), a pinned `sing-box`
//! (`SING_BOX_BIN` or PATH) and `openssl`. Skips otherwise, unless
//! `SINGBOX_VPN_REQUIRE_REAL_INTEROP` is set, which turns a skip into a
//! failure.

#![cfg(target_os = "linux")]

mod common;

use common::{free_port, spawn_local_tls13_decoy, DecoyCertSize, LocalDecoy, SingBox};
use compat_config::contract::{
    contract_endpoint, provisioning_document_with_mode_and_access_paths, DiagnosticMode, VlessFlow,
};
use compat_config::deployment::{migrate_deployment_toml, DeploymentConfig, NodeRole};
use compat_config::model::{
    CompatUser, Hysteria2ServerParams, PeerCredential, RealityServerParams,
};
use compat_config::secret::SecretString;
use compat_config::server::{apply_config_atomically, render_server_config_for_deployment};
use compat_config::CompatError;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const RELAY_IP: &str = "127.0.0.2";
const EXIT_IP: &str = "127.0.0.3";
const TARGET_IP: &str = "127.0.0.4";
const UNDECLARED_IP: &str = "127.0.0.5";
/// Source address of every socket the client device opens itself.
const CLIENT_IP: &str = "127.0.0.6";
const DIRECT_TAG: &str = "Germany · Direct";
const VIA_TAG: &str = "Germany · via Russia";
const NEGATIVE_TIMEOUT: Duration = Duration::from_secs(6);

// Real processes and fixed loopback addresses: run scenarios one at a time
// so timing never depends on how many sing-box processes share the CPU.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ----------------------------------------------------------------------
// Harness primitives
// ----------------------------------------------------------------------

fn prerequisites() -> Option<SingBox> {
    let required = std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok();
    let openssl = std::process::Command::new("openssl")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    match (SingBox::find(), openssl) {
        (Some(sb), true) => Some(sb),
        _ if required => panic!("two-hop system tests require sing-box and openssl"),
        _ => {
            eprintln!("skipping: sing-box and/or openssl not available");
            None
        }
    }
}

/// HTTP 200 target bound to one address; counts requests it served, so a
/// scenario can prove a destination was never reached at all.
struct HttpTarget {
    port: u16,
    hits: Arc<AtomicUsize>,
}

fn spawn_http_target(bind: &str) -> HttpTarget {
    let listener = TcpListener::bind(bind).expect("bind HTTP target");
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let counter = counter.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                counter.fetch_add(1, Ordering::SeqCst);
                let body = b"two-hop-system-target";
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                );
                let _ = stream.write_all(body);
            });
        }
    });
    HttpTarget { port, hits }
}

impl HttpTarget {
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// Transparent TCP forwarder in front of the exit's REALITY listener that
/// records the source address of every connection — the loopback stand-in
/// for the exit-side packet capture of the real two-VPS acceptance (D3).
/// Client sockets dial from [`CLIENT_IP`] (see `Lab::run_client`); the relay
/// dials from its default loopback source, so `client_connections` counts exactly
/// the connections the client device opened straight to the exit.
struct Tap {
    port: u16,
    sources: Arc<Mutex<Vec<std::net::IpAddr>>>,
}

fn spawn_tcp_tap(bind_ip: &str, upstream_port: u16) -> Tap {
    let listener = TcpListener::bind(format!("{bind_ip}:0")).expect("bind tap");
    let port = listener.local_addr().unwrap().port();
    let sources = Arc::new(Mutex::new(Vec::new()));
    let recorded = sources.clone();
    let upstream = format!("{bind_ip}:{upstream_port}");
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let Ok(client) = client else { continue };
            if let Ok(peer) = client.peer_addr() {
                recorded.lock().unwrap().push(peer.ip());
            }
            let Ok(server) = TcpStream::connect(&upstream) else {
                continue;
            };
            let (mut c_read, mut s_write) =
                (client.try_clone().unwrap(), server.try_clone().unwrap());
            let (mut s_read, mut c_write) = (server, client);
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut c_read, &mut s_write);
                let _ = s_write.shutdown(std::net::Shutdown::Write);
            });
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut s_read, &mut c_write);
                let _ = c_write.shutdown(std::net::Shutdown::Write);
            });
        }
    });
    Tap { port, sources }
}

impl Tap {
    fn client_connections(&self) -> usize {
        let client: std::net::IpAddr = CLIENT_IP.parse().unwrap();
        self.sources
            .lock()
            .unwrap()
            .iter()
            .filter(|ip| **ip == client)
            .count()
    }

    fn all_connections(&self) -> usize {
        self.sources.lock().unwrap().len()
    }
}

/// True if a TCP socket is in LISTEN state on `port` (any address). Read
/// from /proc instead of probing with connect(): a connect-and-drop probe
/// against a REALITY listener produces exactly the "processed invalid
/// connection" noise the privacy assertions below must not see.
fn listening(port: u16) -> bool {
    let needle = format!(":{port:04X} ");
    ["/proc/net/tcp", "/proc/net/tcp6"].iter().any(|file| {
        std::fs::read_to_string(file).is_ok_and(|text| {
            text.lines().skip(1).any(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                fields.len() > 3
                    && format!("{} ", fields[1]).ends_with(&needle)
                    && fields[3] == "0A"
            })
        })
    })
}

fn wait_listening(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if listening(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// SOCKS5 CONNECT + HTTP GET with a bounded timeout. True only for an
/// HTTP 200 carrying the target's own body.
fn http_via_socks(socks_port: u16, host: &str, port: u16, timeout: Duration) -> bool {
    let attempt = || -> std::io::Result<bool> {
        let mut stream = TcpStream::connect(("127.0.0.1", socks_port))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.write_all(&[0x05, 0x01, 0x00])?;
        let mut greeting = [0u8; 2];
        stream.read_exact(&mut greeting)?;
        let mut request = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
        request.extend_from_slice(host.as_bytes());
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request)?;
        let mut head = [0u8; 4];
        stream.read_exact(&mut head)?;
        if head[1] != 0 {
            return Ok(false);
        }
        let skip = match head[3] {
            0x01 => 6,
            0x04 => 18,
            _ => {
                let mut len = [0u8; 1];
                stream.read_exact(&mut len)?;
                usize::from(len[0]) + 2
            }
        };
        let mut rest = vec![0u8; skip];
        stream.read_exact(&mut rest)?;
        stream.write_all(
            format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let text = String::from_utf8_lossy(&response);
        Ok(text.starts_with("HTTP/1.1 200") && text.contains("two-hop-system-target"))
    };
    attempt().unwrap_or(false)
}

/// Positive probes retry briefly: a freshly (re)started sing-box can need
/// a moment before its first proxied dial succeeds.
fn reaches(socks_port: u16, host: &str, port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if http_via_socks(socks_port, host, port, Duration::from_secs(5)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Negative probes: several independent attempts, all of which must fail.
fn refused(socks_port: u16, host: &str, port: u16) -> bool {
    (0..3).all(|_| !http_via_socks(socks_port, host, port, NEGATIVE_TIMEOUT))
}

struct Proc(Option<std::process::Child>);

impl Proc {
    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.kill();
    }
}

fn uuid(seed: u8) -> String {
    let h = format!("{seed:02x}");
    format!("{h}{h}{h}{h}-{h}{h}-4{h}0-8{h}0-{h}{h}{h}{h}{h}{h}")
}

fn generate_reality(sb: &SingBox) -> (String, String) {
    let out = std::process::Command::new(&sb.path)
        .args(["generate", "reality-keypair"])
        .output()
        .expect("generate reality keypair");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("{name}:")))
            .map(|v| v.trim().to_string())
            .expect("keypair field")
    };
    (field("PrivateKey"), field("PublicKey"))
}

// ----------------------------------------------------------------------
// Nodes
// ----------------------------------------------------------------------

struct Node {
    dir: tempfile::TempDir,
    deployment_path: PathBuf,
    reality: RealityServerParams,
    hysteria: Hysteria2ServerParams,
    reality_port: u16,
    sub_port: u16,
    log: PathBuf,
    process: Proc,
}

impl Node {
    fn new(sb: &SingBox, decoy: &LocalDecoy, short_id: &str) -> Node {
        let dir = tempfile::tempdir().unwrap();
        let (private_key, public_key) = generate_reality(sb);
        let cert = dir.path().join("hy2-cert.pem");
        let key = dir.path().join("hy2-key.pem");
        let status = std::process::Command::new("openssl")
            .args([
                "req", "-x509", "-newkey", "ed25519", "-days", "1", "-nodes", "-subj",
            ])
            .arg("/CN=two-hop.test")
            .arg("-keyout")
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .expect("openssl");
        assert!(status.status.success(), "hysteria2 test certificate");
        Node {
            deployment_path: dir.path().join("deployment.toml"),
            reality: RealityServerParams {
                private_key_hex: SecretString::new(private_key),
                public_key_hex: public_key,
                short_ids: vec![short_id.to_string()],
                handshake_server: decoy.hostname.to_string(),
                handshake_port: decoy.port,
            },
            hysteria: Hysteria2ServerParams {
                tls_cert_path: cert.to_string_lossy().into_owned(),
                tls_key_path: key.to_string_lossy().into_owned(),
                obfs_password: None,
                masquerade_dir_path: None,
                up_mbps: None,
                down_mbps: None,
            },
            reality_port: free_port(),
            sub_port: free_port(),
            log: dir.path().join("sing-box.log"),
            process: Proc(None),
            dir,
        }
    }

    fn header(&self, node_id: &str, role: &str, host: &str, decoy: &LocalDecoy) -> String {
        format!(
            r#"schema_version = 2
node_id = "{node_id}"
role = "{role}"
public_host = "{host}"
subscription_host = "{host}"
state_dir = "{state}"

[reality]
listen_port = {reality_port}
handshake_server = "{decoy_host}"
handshake_port = {decoy_port}

[hysteria2]
listen_port = {hy2_port}

[subscription]
listen_port = {sub_port}
"#,
            state = self.dir.path().join("state").display(),
            reality_port = self.reality_port,
            decoy_host = decoy.hostname,
            decoy_port = decoy.port,
            hy2_port = free_port(),
            sub_port = self.sub_port,
        )
    }

    fn deployment(&self) -> DeploymentConfig {
        DeploymentConfig::load(&self.deployment_path).expect("load deployment.toml")
    }

    fn config_path(&self) -> PathBuf {
        self.dir.path().join("state/sing-box/config.json")
    }

    /// The production apply path: role-aware render, then write-validate-
    /// swap with the real `sing-box check`.
    fn apply(&self, sb: &SingBox, users: &[CompatUser]) -> Result<serde_json::Value, CompatError> {
        let deployment = self.deployment();
        let doc = render_server_config_for_deployment(
            &deployment,
            users,
            &self.reality,
            &self.hysteria,
            unix_now(),
        )?;
        apply_config_atomically(&doc, &self.config_path(), |candidate| {
            let out = sb.check(candidate);
            if out.status.success() {
                Ok(())
            } else {
                Err(CompatError::ConfigValidationFailed(
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                ))
            }
        })?;
        Ok(doc)
    }

    /// Start (or restart) sing-box from the config currently ON DISK — the
    /// same thing systemd does after a crash or reboot.
    fn start(&mut self, sb: &SingBox) {
        self.process.kill();
        self.process = Proc(Some(sb.run_logged(&self.config_path(), &self.log)));
        assert!(
            wait_listening(self.reality_port),
            "sing-box did not start:\n{}",
            common::read_log(&self.log)
        );
    }

    fn stop(&mut self) {
        self.process.kill();
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn user(id: &str, relay_uuid: &str, exit_uuid: Option<&str>) -> CompatUser {
    let mut peer_credentials = std::collections::BTreeMap::new();
    if let Some(exit_uuid) = exit_uuid {
        peer_credentials.insert(
            "de1-direct".to_string(),
            PeerCredential::VlessReality {
                uuid: exit_uuid.to_string(),
            },
        );
    }
    CompatUser {
        id: id.into(),
        name: id.into(),
        enabled: true,
        vless_uuid: relay_uuid.into(),
        hysteria2_password: SecretString::new(format!("synthetic-hy2-{id}")),
        subscription_token_hash_hex: format!("synthetic-token-hash-{id}"),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        peer_credentials,
    }
}

// ----------------------------------------------------------------------
// The lab
// ----------------------------------------------------------------------

struct Lab {
    sb: SingBox,
    decoy: LocalDecoy,
    target: HttpTarget,
    undeclared: HttpTarget,
    exit: Node,
    relay: Node,
    /// Credential A: issued by the relay, authenticates the first hop.
    relay_user: CompatUser,
    /// Credential B lives here: issued by the exit for the same person.
    exit_user: CompatUser,
    /// In front of the exit; both declared peers point at it.
    exit_tap: Tap,
    clients: Vec<Proc>,
}

impl Lab {
    fn start() -> Option<Lab> {
        let sb = prerequisites()?;
        let decoy = spawn_local_tls13_decoy(DecoyCertSize::Small).expect("local TLS 1.3 decoy");
        let exit = Node::new(&sb, &decoy, "0a1b2c3d");
        let relay = Node::new(&sb, &decoy, "1a2b3c4d");
        let mut lab = Lab {
            target: spawn_http_target(&format!("{TARGET_IP}:0")),
            undeclared: spawn_http_target(&format!("{UNDECLARED_IP}:0")),
            relay_user: user("alice", &uuid(0xa1), Some(&uuid(0xb1))),
            exit_user: user("alice-at-exit", &uuid(0xb1), None),
            exit_tap: spawn_tcp_tap(EXIT_IP, exit.reality_port),
            exit,
            relay,
            decoy,
            sb,
            clients: Vec::new(),
        };
        lab.write_exit_deployment();
        lab.write_relay_deployment(true);
        lab.exit
            .apply(&lab.sb, std::slice::from_ref(&lab.exit_user))
            .unwrap();
        lab.relay
            .apply(&lab.sb, std::slice::from_ref(&lab.relay_user))
            .unwrap();
        lab.exit.start(&lab.sb);
        lab.relay.start(&lab.sb);
        Some(lab)
    }

    fn write_exit_deployment(&self) {
        let text = self.exit.header("de1", "exit", EXIT_IP, &self.decoy);
        std::fs::write(&self.exit.deployment_path, text).unwrap();
    }

    fn write_relay_deployment(&self, paired: bool) {
        let mut text = self.relay.header("ru1", "relay", RELAY_IP, &self.decoy);
        text.push_str(
            r#"
[[access_paths]]
id = "via-ru1"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]
"#,
        );
        if paired {
            for (id, tag, path, credential_ref) in [
                ("de1-direct", DIRECT_TAG, "direct", ""),
                (
                    "de1-via-ru1",
                    VIA_TAG,
                    "via-ru1",
                    "credential_ref = \"de1-direct\"\n",
                ),
            ] {
                text.push_str(&format!(
                    r#"
[[peer_endpoints]]
id = "{id}"
tag = "{tag}"
host = "{EXIT_IP}"
port = {port}
transport = "vless_reality"
server_name = "{sni}"
reality_public_key = "{pbk}"
reality_short_id = "{sid}"
failure_domain = "exit:de1"
path = "{path}"
{credential_ref}"#,
                    port = self.exit_tap.port,
                    sni = self.decoy.hostname,
                    pbk = self.exit.reality.public_key_hex,
                    sid = self.exit.reality.short_ids[0],
                ));
            }
        }
        std::fs::write(&self.relay.deployment_path, text).unwrap();
    }

    /// The first-party provisioning document's embedded Core config for
    /// `u`, built exactly as `GET /v1/provision/{token}` builds it on the
    /// relay.
    fn provisioned_config(&self, u: &CompatUser) -> Result<serde_json::Value, CompatError> {
        let deployment = self.relay.deployment();
        let endpoints = deployment.served_endpoints(
            &self.relay.reality.public_key_hex,
            &self.relay.reality.short_ids[0],
            None,
        )?;
        let doc = provisioning_document_with_mode_and_access_paths(
            u,
            &endpoints,
            DiagnosticMode::None,
            &deployment.contract_access_paths()?,
        )?;
        Ok(doc.singbox_config.expect("embedded Core config"))
    }

    /// A client-owned wrapper around a Core config: a SOCKS inbound and the
    /// selected route. Returns the SOCKS port.
    fn client(&mut self, core: &serde_json::Value, selected_tag: &str) -> u16 {
        let mut config = core.clone();
        config["route"]["final"] = serde_json::json!(selected_tag);
        self.run_client(config)
    }

    /// A client running the served Core config as served: `route.final`
    /// stays the selector and every group, the automatic one included,
    /// runs. `choice` is the user's pick in the selector (applied as its
    /// start-up selection); `None` keeps the served default.
    fn served_client(&mut self, core: &serde_json::Value, choice: Option<&str>) -> u16 {
        let mut config = core.clone();
        if let Some(choice) = choice {
            for outbound in config["outbounds"].as_array_mut().unwrap() {
                if outbound["tag"] == "select" {
                    outbound["default"] = serde_json::json!(choice);
                }
            }
        }
        self.run_client(config)
    }

    fn run_client(&mut self, mut config: serde_json::Value) -> u16 {
        let socks = free_port();
        // The device's own address: every outbound that opens a socket
        // itself (no `detour`) dials from CLIENT_IP, as a real device dials
        // from its public IP. Routing and groups are untouched.
        for outbound in config["outbounds"].as_array_mut().unwrap() {
            if outbound.get("server").is_some() && outbound.get("detour").is_none() {
                outbound["inet4_bind_address"] = serde_json::json!(CLIENT_IP);
            }
        }
        config["log"] = serde_json::json!({"level": "warn"});
        config["inbounds"] = serde_json::json!([
            {"type": "mixed", "tag": "socks-in", "listen": "127.0.0.1", "listen_port": socks}
        ]);
        let dir = self.relay.dir.path().join(format!("client-{socks}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("client.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let check = self.sb.check(&path);
        assert!(
            check.status.success(),
            "client config rejected: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        self.clients.push(Proc(Some(
            self.sb.run_logged(&path, &dir.join("client.log")),
        )));
        assert!(wait_listening(socks), "client did not start");
        socks
    }

    /// A client that holds only the relay's first-hop outbound, as a
    /// misbehaving client extracting it from its profile would.
    fn first_hop_only_client(&mut self, relay_uuid: &str) -> u16 {
        let deployment = self.relay.deployment();
        let endpoints = deployment
            .served_endpoints(
                &self.relay.reality.public_key_hex,
                &self.relay.reality.short_ids[0],
                None,
            )
            .unwrap();
        let first_hop = endpoints.iter().find(|e| e.id == "reality-1").unwrap();
        let built = contract_endpoint(
            &user("probe", relay_uuid, None),
            first_hop,
            VlessFlow::Vision,
            Some("first-hop"),
        )
        .unwrap();
        let core = compat_config::render::render_singbox_config_from_contract(
            &[built],
            Default::default(),
            Default::default(),
        )
        .unwrap();
        self.client(&core, "first-hop")
    }

    /// Proves a first-hop tunnel is authenticated and alive, so that a
    /// refusal of another destination through the SAME tunnel can only be
    /// the relay's forwarding policy.
    fn selftest_target_on_relay_loopback(&self) -> HttpTarget {
        spawn_http_target(&format!("127.0.0.1:{}", self.relay.sub_port))
    }
}

fn set_outbound_uuid(
    core: &mut serde_json::Value,
    pick: impl Fn(&serde_json::Value) -> bool,
    uuid: &str,
) {
    for outbound in core["outbounds"].as_array_mut().unwrap() {
        if pick(outbound) {
            outbound["uuid"] = serde_json::json!(uuid);
        }
    }
}

macro_rules! lab {
    () => {{
        let _serial = serial();
        match Lab::start() {
            Some(lab) => (lab, _serial),
            None => return,
        }
    }};
}

// ----------------------------------------------------------------------
// S1–S15
// ----------------------------------------------------------------------

#[test]
fn s01_direct_route_client_exit_target() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let socks = lab.client(&core, DIRECT_TAG);
    assert!(
        reaches(socks, TARGET_IP, lab.target.port),
        "S1: direct route must work"
    );
}

#[test]
fn s02_relay_route_client_relay_exit_target() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = core["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["tag"] == VIA_TAG)
        .unwrap()
        .clone();
    assert!(
        via["detour"].is_string(),
        "S2: the via route must be a Core detour chain"
    );
    let socks = lab.client(&core, VIA_TAG);
    assert!(
        reaches(socks, TARGET_IP, lab.target.port),
        "S2: relay route must work"
    );

    // The via route really depends on the relay: with the relay down the
    // same client, same config, same exit, cannot reach the target.
    lab.relay.stop();
    assert!(
        refused(socks, TARGET_IP, lab.target.port),
        "S2: via route must need the relay"
    );
}

#[test]
fn s03_relay_is_not_an_internet_exit() {
    let (mut lab, _serial) = lab!();
    let control = lab.selftest_target_on_relay_loopback();
    let socks = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(
        reaches(socks, "127.0.0.1", control.port),
        "control: the authenticated first-hop tunnel is alive"
    );
    let before = lab.target.hits() + lab.undeclared.hits();
    assert!(
        refused(socks, TARGET_IP, lab.target.port),
        "S3: relay must not exit to a target"
    );
    assert!(refused(socks, UNDECLARED_IP, lab.undeclared.port));
    assert_eq!(
        lab.target.hits() + lab.undeclared.hits(),
        before,
        "no byte reached a target"
    );
}

#[test]
fn s04_wrong_relay_credential_fails() {
    let (mut lab, _serial) = lab!();
    let mut core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let relay_uuid = lab.relay_user.vless_uuid.clone();
    set_outbound_uuid(&mut core, |o| o["uuid"] == relay_uuid, &uuid(0xee));
    let socks = lab.client(&core, VIA_TAG);
    assert!(
        refused(socks, TARGET_IP, lab.target.port),
        "S4: wrong first-hop credential"
    );
    assert_eq!(lab.target.hits(), 0);
}

#[test]
fn s05_wrong_exit_credential_fails_after_reaching_the_relay() {
    let (mut lab, _serial) = lab!();
    let mut core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    set_outbound_uuid(&mut core, |o| o["tag"] == VIA_TAG, &uuid(0xef));
    let via_socks = lab.client(&core, VIA_TAG);
    assert!(
        refused(via_socks, TARGET_IP, lab.target.port),
        "S5: wrong exit credential"
    );
    assert_eq!(lab.target.hits(), 0);

    let control = lab.selftest_target_on_relay_loopback();
    let first_hop = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(
        reaches(first_hop, "127.0.0.1", control.port),
        "S5: the relay itself accepted the same first-hop credential"
    );
}

#[test]
fn s06_exit_unavailable_fails_without_direct_fallback() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = lab.client(&core, VIA_TAG);
    let direct = lab.client(&core, DIRECT_TAG);
    assert!(reaches(via, TARGET_IP, lab.target.port));
    lab.exit.stop();
    let hits = lab.target.hits();
    assert!(
        refused(via, TARGET_IP, lab.target.port),
        "S6: via route must fail"
    );
    assert!(
        refused(direct, TARGET_IP, lab.target.port),
        "S6: exit is down for direct too"
    );
    assert_eq!(
        lab.target.hits(),
        hits,
        "S6: nothing fell back to another path"
    );
}

#[test]
fn s07_relay_unavailable_fails_while_direct_remains_usable() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = lab.client(&core, VIA_TAG);
    let direct = lab.client(&core, DIRECT_TAG);
    lab.relay.stop();
    assert!(
        refused(via, TARGET_IP, lab.target.port),
        "S7: via route must fail"
    );
    assert!(
        reaches(direct, TARGET_IP, lab.target.port),
        "S7: an explicitly selected direct route stays valid"
    );
}

#[test]
fn s08_disabled_user_cannot_reconnect() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = lab.client(&core, VIA_TAG);
    assert!(reaches(via, TARGET_IP, lab.target.port));

    let mut disabled = lab.relay_user.clone();
    disabled.enabled = false;
    let doc = lab.relay.apply(&lab.sb, &[disabled]).unwrap();
    assert!(doc["inbounds"][0]["users"].as_array().unwrap().is_empty());
    lab.relay.start(&lab.sb);
    assert!(
        refused(via, TARGET_IP, lab.target.port),
        "S8: disabled at the relay"
    );

    // Disabling at the exit revokes both routes that use credential B.
    lab.relay
        .apply(&lab.sb, std::slice::from_ref(&lab.relay_user))
        .unwrap();
    lab.relay.start(&lab.sb);
    assert!(
        reaches(via, TARGET_IP, lab.target.port),
        "re-enabled at the relay"
    );
    let direct = lab.client(&core, DIRECT_TAG);
    let mut exit_disabled = lab.exit_user.clone();
    exit_disabled.enabled = false;
    lab.exit.apply(&lab.sb, &[exit_disabled]).unwrap();
    lab.exit.start(&lab.sb);
    assert!(
        refused(via, TARGET_IP, lab.target.port),
        "S8: disabled at the exit (via)"
    );
    assert!(
        refused(direct, TARGET_IP, lab.target.port),
        "S8: disabled at the exit (direct)"
    );
}

#[test]
fn s09_expired_user_loses_authorization_after_reconciliation() {
    let (mut lab, _serial) = lab!();
    let mut expiring = lab.relay_user.clone();
    expiring.expires_at = Some(unix_now() + 3);
    lab.relay.apply(&lab.sb, &[expiring.clone()]).unwrap();
    lab.relay.start(&lab.sb);
    let core = lab.provisioned_config(&expiring).unwrap();
    let via = lab.client(&core, VIA_TAG);
    assert!(
        reaches(via, TARGET_IP, lab.target.port),
        "active before expiry"
    );

    while unix_now() <= expiring.expires_at.unwrap() {
        std::thread::sleep(Duration::from_millis(250));
    }
    // Expiry is enforced at reconciliation (vpn-expiry-reconcile runs
    // `render-config` every minute), not by the running process itself.
    let doc = lab.relay.apply(&lab.sb, &[expiring]).unwrap();
    assert!(doc["inbounds"][0]["users"].as_array().unwrap().is_empty());
    lab.relay.start(&lab.sb);
    assert!(
        refused(via, TARGET_IP, lab.target.port),
        "S9: expired after reconciliation"
    );
}

#[test]
fn s10_credential_rotation_old_fails_new_works_scopes_independent() {
    let (mut lab, _serial) = lab!();
    let old_core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let old_via = lab.client(&old_core, VIA_TAG);
    assert!(reaches(old_via, TARGET_IP, lab.target.port));

    // Rotate credential A (first hop) only.
    let mut rotated_a = lab.relay_user.clone();
    rotated_a.vless_uuid = uuid(0xa2);
    lab.relay.apply(&lab.sb, &[rotated_a.clone()]).unwrap();
    lab.relay.start(&lab.sb);
    assert!(
        refused(old_via, TARGET_IP, lab.target.port),
        "S10: old credential A fails"
    );
    let core_a = lab.provisioned_config(&rotated_a).unwrap();
    assert_eq!(
        core_a["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == VIA_TAG)
            .unwrap()["uuid"],
        uuid(0xb1),
        "rotating A did not touch B"
    );
    let via_a = lab.client(&core_a, VIA_TAG);
    assert!(
        reaches(via_a, TARGET_IP, lab.target.port),
        "S10: new credential A works"
    );

    // Rotate credential B (exit) only: at the exit, then the operator
    // records it for the relay user.
    let mut exit_user = lab.exit_user.clone();
    exit_user.vless_uuid = uuid(0xb2);
    lab.exit.apply(&lab.sb, &[exit_user]).unwrap();
    lab.exit.start(&lab.sb);
    assert!(
        refused(via_a, TARGET_IP, lab.target.port),
        "S10: old credential B fails"
    );
    let mut rotated_b = rotated_a.clone();
    rotated_b.peer_credentials.insert(
        "de1-direct".into(),
        PeerCredential::VlessReality { uuid: uuid(0xb2) },
    );
    let relay_doc_before = lab.relay.apply(&lab.sb, &[rotated_a]).unwrap();
    let relay_doc_after = lab.relay.apply(&lab.sb, &[rotated_b.clone()]).unwrap();
    assert_eq!(
        relay_doc_before, relay_doc_after,
        "rotating B did not touch the relay's config"
    );
    let core_b = lab.provisioned_config(&rotated_b).unwrap();
    let via_b = lab.client(&core_b, VIA_TAG);
    assert!(
        reaches(via_b, TARGET_IP, lab.target.port),
        "S10: new credential B works"
    );
}

#[test]
fn s11_malformed_or_dangling_declarations_fail_closed_and_apply_nothing() {
    let (lab, _serial) = lab!();
    let good = std::fs::read(lab.relay.config_path()).unwrap();
    let original = std::fs::read_to_string(&lab.relay.deployment_path).unwrap();
    for (broken, why) in [
        (
            original.replace(
                "via_endpoint_id = \"reality-1\"",
                "via_endpoint_id = \"ghost\"",
            ),
            "dangling via",
        ),
        (
            original.replace(
                "credential_ref = \"de1-direct\"",
                "credential_ref = \"ghost\"",
            ),
            "dangling credential_ref",
        ),
        (
            original.replace(
                "capabilities = [\"tcp\"]",
                "capabilities = [\"tcp\", \"udp\"]",
            ),
            "udp relay",
        ),
        (
            original.replace("role = \"relay\"", "role = \"gateway\""),
            "invalid role",
        ),
        (
            original.replace("node_id = \"ru1\"\n", ""),
            "missing node id",
        ),
        (
            original.replace("schema_version = 2", "schema_version = 9"),
            "future schema",
        ),
        (original[..original.len() / 2].to_string(), "truncated"),
    ] {
        std::fs::write(&lab.relay.deployment_path, &broken).unwrap();
        assert!(
            DeploymentConfig::load(&lab.relay.deployment_path).is_err(),
            "S11: {why} must not load"
        );
    }
    // A declaration that loads but is broken in memory still cannot render.
    std::fs::write(&lab.relay.deployment_path, &original).unwrap();
    let mut in_memory = lab.relay.deployment();
    in_memory.access_paths.clear();
    assert!(render_server_config_for_deployment(
        &in_memory,
        std::slice::from_ref(&lab.relay_user),
        &lab.relay.reality,
        &lab.relay.hysteria,
        0
    )
    .is_err());
    assert_eq!(
        std::fs::read(lab.relay.config_path()).unwrap(),
        good,
        "S11: nothing applied"
    );

    // Provisioning for a user without an exit credential is an explicit
    // no-route error, never the first hop served as an exit.
    let no_exit = user("bob", &uuid(0xc1), None);
    assert!(matches!(
        lab.provisioned_config(&no_exit),
        Err(CompatError::NoSelectableRoute)
    ));
}

#[test]
fn s12_unpaired_relay_cannot_reach_anything_as_an_exit() {
    let (mut lab, _serial) = lab!();
    lab.write_relay_deployment(false);
    let doc = lab
        .relay
        .apply(&lab.sb, std::slice::from_ref(&lab.relay_user))
        .unwrap();
    assert_eq!(doc["route"]["rules"].as_array().unwrap().len(), 2);
    lab.relay.start(&lab.sb);
    assert!(matches!(
        lab.provisioned_config(&lab.relay_user.clone()),
        Err(CompatError::NoSelectableRoute)
    ));

    let control = lab.selftest_target_on_relay_loopback();
    let socks = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(
        reaches(socks, "127.0.0.1", control.port),
        "control: tunnel alive"
    );
    assert!(
        refused(socks, TARGET_IP, lab.target.port),
        "S12: generic target"
    );
    assert!(
        refused(socks, UNDECLARED_IP, lab.undeclared.port),
        "S12: second target"
    );
    assert!(
        refused(socks, EXIT_IP, lab.exit.reality_port),
        "S12: not even the exit"
    );
    let other_loopback = spawn_http_target("127.0.0.1:0");
    assert!(
        refused(socks, "127.0.0.1", other_loopback.port),
        "S12: other loopback ports"
    );
    assert_eq!(
        lab.target.hits() + lab.undeclared.hits() + other_loopback.hits(),
        0
    );
}

#[test]
fn s13_relay_reaches_only_the_declared_exit_target() {
    let (mut lab, _serial) = lab!();
    // Declare an HTTP server as the "exit" so the forwarding decision is
    // observable as plain HTTP, independent of REALITY.
    let declared = spawn_http_target(&format!("{TARGET_IP}:0"));
    let same_host_other_port = spawn_http_target(&format!("{TARGET_IP}:0"));
    let named = spawn_http_target("[::]:0");
    let text = std::fs::read_to_string(&lab.relay.deployment_path)
        .unwrap()
        .replacen(
            &format!("host = \"{EXIT_IP}\"\nport = {}", lab.exit_tap.port),
            &format!("host = \"{TARGET_IP}\"\nport = {}", declared.port),
            2,
        )
        + &format!(
            r#"
[[peer_endpoints]]
id = "named-direct"
tag = "Named direct"
host = "localhost"
port = {port}
transport = "vless_reality"
reality_public_key = "{pbk}"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:named"

[[peer_endpoints]]
id = "named-via"
tag = "Named via"
host = "localhost"
port = {port}
transport = "vless_reality"
reality_public_key = "{pbk}"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:named"
path = "via-ru1"
credential_ref = "named-direct"
"#,
            port = named.port,
            pbk = lab.exit.reality.public_key_hex
        );
    std::fs::write(&lab.relay.deployment_path, text).unwrap();
    let doc = lab
        .relay
        .apply(&lab.sb, std::slice::from_ref(&lab.relay_user))
        .unwrap();
    assert_eq!(
        doc["route"]["rules"].as_array().unwrap().len(),
        4,
        "self-test + 2 exits + reject"
    );
    lab.relay.start(&lab.sb);

    let socks = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(
        reaches(socks, TARGET_IP, declared.port),
        "S13: declared exit (IP) allowed"
    );
    assert!(
        reaches(socks, "localhost", named.port),
        "S13: declared exit (DNS name) allowed"
    );
    assert!(
        refused(socks, UNDECLARED_IP, lab.undeclared.port),
        "S13: undeclared host"
    );
    assert!(
        refused(socks, TARGET_IP, same_host_other_port.port),
        "S13: undeclared port"
    );
    assert!(
        refused(socks, "127.0.0.1", named.port),
        "S13: the IP form of a host declared by name is a different destination"
    );
    assert_eq!(lab.undeclared.hits() + same_host_other_port.hits(), 0);
}

#[test]
fn s14_reload_and_repair_preserve_role_and_restrictions() {
    let (mut lab, _serial) = lab!();
    // A repair/update re-runs migrate + render-config against the
    // persisted state. It must neither change the role nor loosen policy.
    let before = std::fs::read(lab.relay.config_path()).unwrap();
    assert_eq!(
        migrate_deployment_toml(&lab.relay.deployment_path).unwrap(),
        compat_config::deployment::DeploymentMigrationOutcome::AlreadyCurrent
    );
    for _ in 0..2 {
        let doc = lab
            .relay
            .apply(&lab.sb, std::slice::from_ref(&lab.relay_user))
            .unwrap();
        assert_eq!(
            doc["route"]["rules"].as_array().unwrap().last().unwrap()["action"],
            "reject"
        );
    }
    assert_eq!(
        std::fs::read(lab.relay.config_path()).unwrap(),
        before,
        "idempotent render"
    );
    assert_eq!(lab.relay.deployment().role, NodeRole::Relay);

    // A pre-v2 relay file (explicit role, no node_id) migrates to v2 as a
    // relay, never as an exit.
    let v1 = std::fs::read_to_string(&lab.relay.deployment_path)
        .unwrap()
        .replace("schema_version = 2", "schema_version = 1")
        .replace("node_id = \"ru1\"\n", "");
    std::fs::write(&lab.relay.deployment_path, v1).unwrap();
    assert!(matches!(
        migrate_deployment_toml(&lab.relay.deployment_path).unwrap(),
        compat_config::deployment::DeploymentMigrationOutcome::Migrated { .. }
    ));
    let migrated = lab.relay.deployment();
    assert_eq!(migrated.role, NodeRole::Relay);
    assert_eq!(migrated.node_id, "127");
    lab.relay
        .apply(&lab.sb, std::slice::from_ref(&lab.relay_user))
        .unwrap();
    lab.relay.start(&lab.sb);

    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = lab.client(&core, VIA_TAG);
    assert!(
        reaches(via, TARGET_IP, lab.target.port),
        "S14: via route after repair"
    );
    let socks = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(
        refused(socks, UNDECLARED_IP, lab.undeclared.port),
        "S14: still restricted"
    );
}

#[test]
fn s15_crash_restart_never_passes_through_an_unrestricted_state() {
    let (mut lab, _serial) = lab!();
    let restricted = std::fs::read(lab.relay.config_path()).unwrap();

    // A candidate that fails `sing-box check` must never replace the live
    // restricted document.
    let bogus = serde_json::json!({"inbounds": [{"type": "no-such-inbound"}]});
    assert!(
        apply_config_atomically(&bogus, &lab.relay.config_path(), |p| {
            let out = lab.sb.check(p);
            if out.status.success() {
                Ok(())
            } else {
                Err(CompatError::ConfigValidationFailed("rejected".into()))
            }
        })
        .is_err()
    );
    assert_eq!(std::fs::read(lab.relay.config_path()).unwrap(), restricted);

    // SIGKILL, then restart from what is on disk, as systemd would.
    lab.relay.stop();
    lab.relay.start(&lab.sb);
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = lab.client(&core, VIA_TAG);
    assert!(
        reaches(via, TARGET_IP, lab.target.port),
        "S15: via route after restart"
    );
    let socks = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(
        refused(socks, UNDECLARED_IP, lab.undeclared.port),
        "S15: restricted after restart"
    );
    assert_eq!(lab.undeclared.hits(), 0);

    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(lab.relay.config_path()).unwrap()).unwrap();
    assert_eq!(
        on_disk["route"]["rules"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["action"],
        "reject",
        "S15: there is no on-disk state in which the relay is unrestricted"
    );
}

// ----------------------------------------------------------------------
// S16–S17: the served profile as a real client runs it (D3)
//
// S1–S15 pin `route.final` to one route tag, which bypasses every group
// the served profile contains. The real two-VPS acceptance found that the
// automatic `urltest` group dialled "Germany · Direct" from the client
// device even while Privacy+ was selected, telling the exit the client's
// address. These scenarios run the profile unmodified and count
// client-to-exit connections at `Lab::exit_tap`.
// ----------------------------------------------------------------------

/// Longer than Core start-up plus the automatic group's initial probe
/// round; the 1-minute interval itself is exercised on the real hosts.
const IDLE_WINDOW: Duration = Duration::from_secs(8);

fn wait_for_tap(tap: &Tap, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if tap.client_connections() > 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn s16_privacy_plus_client_never_connects_to_the_exit_directly() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    assert!(
        core["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["type"] == "urltest"),
        "the served automatic group is part of what is exercised"
    );

    let explicit = lab.served_client(&core, Some(VIA_TAG));
    let default = lab.served_client(&core, None);
    let automatic = lab.served_client(&core, Some("auto"));
    for (socks, what) in [
        (explicit, "explicit Privacy+"),
        (default, "served default"),
        (automatic, "automatic Privacy+"),
    ] {
        assert!(
            reaches(socks, TARGET_IP, lab.target.port),
            "S16: {what} works through the served selector"
        );
    }
    std::thread::sleep(IDLE_WINDOW);
    assert!(
        lab.exit_tap.all_connections() > 0,
        "the tap sees the exit's traffic: the relay's connections pass through it"
    );
    assert_eq!(
        lab.exit_tap.client_connections(),
        0,
        "S16: a Privacy+ client opened a direct connection to the exit"
    );

    // Control: the pre-fix group shape, Direct and Privacy+ in one
    // automatic group, is caught by the same instrument.
    let mut mixed = core.clone();
    for outbound in mixed["outbounds"].as_array_mut().unwrap() {
        if outbound["type"] == "urltest" {
            outbound["outbounds"] = serde_json::json!([DIRECT_TAG, VIA_TAG]);
        }
    }
    lab.served_client(&mixed, Some(VIA_TAG));
    assert!(
        wait_for_tap(&lab.exit_tap, Duration::from_secs(20)),
        "control: a mixed automatic group must be visible as a direct exit connection"
    );
}

#[test]
fn s17_relay_down_privacy_plus_fails_closed_while_direct_stays_explicit() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let explicit = lab.served_client(&core, Some(VIA_TAG));
    let automatic = lab.served_client(&core, Some("auto"));
    assert!(reaches(explicit, TARGET_IP, lab.target.port));
    assert!(reaches(automatic, TARGET_IP, lab.target.port));

    lab.relay.stop();
    let hits = lab.target.hits();
    assert!(
        refused(explicit, TARGET_IP, lab.target.port),
        "S17: explicit Privacy+ must fail without the relay"
    );
    assert!(
        refused(automatic, TARGET_IP, lab.target.port),
        "S17: automatic Privacy+ must fail without the relay"
    );
    std::thread::sleep(IDLE_WINDOW);
    assert_eq!(lab.target.hits(), hits, "S17: nothing reached the target");
    assert_eq!(
        lab.exit_tap.client_connections(),
        0,
        "S17: no Privacy+ client fell back to, or probed, the direct route"
    );

    let direct = lab.served_client(&core, Some(DIRECT_TAG));
    assert!(
        reaches(direct, TARGET_IP, lab.target.port),
        "S17: an explicitly chosen Direct route works without the relay"
    );
    assert!(
        lab.exit_tap.client_connections() > 0,
        "S17: and it really is the direct connection"
    );
}

// ----------------------------------------------------------------------
// Privacy: what the relay and exit write to their logs
// ----------------------------------------------------------------------

#[test]
fn relay_and_exit_logs_carry_no_credentials_and_no_rejected_destinations() {
    let (mut lab, _serial) = lab!();
    let core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();
    let via = lab.client(&core, VIA_TAG);
    assert!(reaches(via, TARGET_IP, lab.target.port));
    let socks = lab.first_hop_only_client(&lab.relay_user.vless_uuid.clone());
    assert!(refused(socks, UNDECLARED_IP, lab.undeclared.port));
    std::thread::sleep(Duration::from_millis(500));

    for (name, log) in [
        ("relay", lab.relay.log_text()),
        ("exit", lab.exit.log_text()),
    ] {
        for secret in [
            lab.relay_user.vless_uuid.as_str(),
            lab.exit_user.vless_uuid.as_str(),
            lab.relay.reality.private_key_hex.expose(),
            lab.exit.reality.private_key_hex.expose(),
            "synthetic-hy2-alice",
            "synthetic-token-hash-alice",
        ] {
            assert!(
                !log.contains(secret),
                "{name} log leaked a credential:\n{log}"
            );
        }
        assert!(
            !log.contains(UNDECLARED_IP) && !log.contains(&format!(":{}", lab.target.port)),
            "{name} log at the production level recorded a tunnelled destination:\n{log}"
        );
    }
}

/// D4 (real two-VPS acceptance): sing-box reports a rejected VLESS login as
/// `process connection from <client address>: unknown UUID: <presented
/// value>` at ERROR severity. With the production log level that line went
/// to journald/syslog, so revoked — and, after a disable/re-enable, currently
/// valid — credentials and client addresses were persisted on disk.
#[test]
fn rejected_and_revoked_credentials_never_reach_relay_or_exit_logs() {
    let (mut lab, _serial) = lab!();
    let old_core = lab.provisioned_config(&lab.relay_user.clone()).unwrap();

    // Revoke A at the relay and B at the exit by rotating both.
    let revoked_a = lab.relay_user.vless_uuid.clone();
    let revoked_b = lab.exit_user.vless_uuid.clone();
    let mut relay_user = lab.relay_user.clone();
    relay_user.vless_uuid = uuid(0xa7);
    relay_user.peer_credentials.insert(
        "de1-direct".into(),
        PeerCredential::VlessReality { uuid: uuid(0xb7) },
    );
    let mut exit_user = lab.exit_user.clone();
    exit_user.vless_uuid = uuid(0xb7);
    lab.relay
        .apply(&lab.sb, std::slice::from_ref(&relay_user))
        .unwrap();
    lab.exit
        .apply(&lab.sb, std::slice::from_ref(&exit_user))
        .unwrap();
    lab.relay.start(&lab.sb);
    lab.exit.start(&lab.sb);
    let core = lab.provisioned_config(&relay_user).unwrap();

    let invalid_at_relay = uuid(0xee);
    let invalid_at_exit = uuid(0xef);
    let mut random_first_hop = core.clone();
    set_outbound_uuid(
        &mut random_first_hop,
        |o| o["uuid"] == uuid(0xa7),
        &invalid_at_relay,
    );
    let mut revoked_exit = core.clone();
    set_outbound_uuid(&mut revoked_exit, |o| o["tag"] == VIA_TAG, &revoked_b);
    let mut random_direct = core.clone();
    set_outbound_uuid(
        &mut random_direct,
        |o| o["tag"] == DIRECT_TAG,
        &invalid_at_exit,
    );

    let rejected = [
        (
            lab.client(&old_core, VIA_TAG),
            "revoked credential A at the relay",
        ),
        (
            lab.client(&random_first_hop, VIA_TAG),
            "random credential at the relay",
        ),
        (
            lab.client(&revoked_exit, VIA_TAG),
            "revoked credential B at the exit",
        ),
        (
            lab.client(&random_direct, DIRECT_TAG),
            "random credential at the exit",
        ),
    ];
    for (socks, what) in rejected {
        assert!(
            refused(socks, TARGET_IP, lab.target.port),
            "{what} must be rejected"
        );
    }
    let current = lab.client(&core, VIA_TAG);
    assert!(
        reaches(current, TARGET_IP, lab.target.port),
        "the rotated credentials work"
    );
    std::thread::sleep(Duration::from_millis(500));

    for (name, log) in [
        ("relay", lab.relay.log_text()),
        ("exit", lab.exit.log_text()),
    ] {
        for (value, what) in [
            (revoked_a.as_str(), "revoked credential A"),
            (revoked_b.as_str(), "revoked credential B"),
            (invalid_at_relay.as_str(), "presented credential"),
            (invalid_at_exit.as_str(), "presented credential"),
            (relay_user.vless_uuid.as_str(), "current credential A"),
            (exit_user.vless_uuid.as_str(), "current credential B"),
            (
                lab.relay.reality.private_key_hex.expose(),
                "REALITY private key",
            ),
            (
                lab.exit.reality.private_key_hex.expose(),
                "REALITY private key",
            ),
            ("synthetic-hy2-alice", "Hysteria2 password"),
            ("synthetic-token-hash-alice", "subscription token hash"),
        ] {
            assert!(
                !log.contains(value),
                "{name} log persisted a {what}:\n{log}"
            );
        }
        for marker in ["unknown UUID", "process connection from"] {
            assert!(
                !log.contains(marker),
                "{name} log recorded a per-connection event ({marker:?}):\n{log}"
            );
        }
    }
}

/// The other half of D4: silencing per-connection Core events must not make
/// a broken node undiagnosable. A start failure still reaches the service
/// log with a non-zero exit status, which is what systemd, the watchdog and
/// `journalctl -u sing-box` surface.
#[test]
fn core_start_failure_stays_visible_with_production_logging() {
    let (mut lab, _serial) = lab!();
    lab.relay.stop();
    let squatter = TcpListener::bind(format!("[::]:{}", lab.relay.reality_port))
        .expect("occupy the relay's REALITY port");
    let mut child = lab.sb.run_logged(&lab.relay.config_path(), &lab.relay.log);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "sing-box kept running on a busy port"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(squatter);
    assert!(!status.success(), "a start failure exits non-zero");
    let log = lab.relay.log_text();
    assert!(
        log.contains("FATAL") && log.contains("start service"),
        "the start failure is reported:\n{log}"
    );
}
