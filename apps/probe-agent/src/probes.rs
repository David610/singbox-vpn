//! Layered probes for one target.
//!
//! What each layer means here (and what it cannot see):
//!
//! * **L1** — TCP connect to the endpoint (TCP transports only; UDP has no
//!   connectionless reachability signal).
//! * **L2** — for AmneziaWG, a completed handshake reported by upstream
//!   `awg show latest-handshakes`. For sing-box transports the handshake is
//!   not separately observable through a client process: L2 is reported
//!   passed when L4 succeeds, and failed when L1 passed but the tunnel
//!   carried nothing, with the error inferred from the client's own log.
//! * **L3** — the local engine (sing-box client / amneziawg-go) started.
//! * **L4** — the configured URL answered 200/204 through the tunnel.
//! * **L5** — a bounded download of exactly the configured size completed
//!   before its deadline.
//!
//! Only controlled URLs from the configuration are fetched; no user traffic
//! exists on a probe host. Classification is per result and uses no
//! cross-vantage controls, so it never yields `CENSORSHIP_SUSPECTED`; that
//! requires `vpn-probe evaluate` over several vantages.

use crate::config::{ProbeConfig, TargetConfig};
use anyhow::{bail, Context, Result};
use platform_core::failure::{classify, ClassifierPolicy, ErrorKind, FailureClass, FailureInput};
use platform_core::health::{HealthLayer, LayerResult};
use platform_core::probe::ProbeResult;
use platform_core::transport::{DataPlaneEngine, L4Protocol};
use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

struct Outcome {
    layers: Vec<LayerResult>,
    failure: Option<(HealthLayer, ErrorKind)>,
    handshake_ms: Option<u32>,
    rtt_ms: Option<u32>,
    transfer_kbps: Option<u32>,
}

impl Outcome {
    fn new() -> Self {
        Outcome { layers: Vec::new(), failure: None, handshake_ms: None, rtt_ms: None, transfer_kbps: None }
    }
    fn pass(&mut self, layer: HealthLayer, ms: Option<u32>) {
        self.layers.push(LayerResult { layer, ok: true, duration_ms: ms });
    }
    fn fail(&mut self, layer: HealthLayer, error: ErrorKind) {
        self.layers.push(LayerResult { layer, ok: false, duration_ms: None });
        self.failure = Some((layer, error));
    }
}

pub fn probe(cfg: &ProbeConfig, target: &TargetConfig) -> ProbeResult {
    let started = now_ms();
    let caps = target.transport.capabilities().expect("validated at load");
    let outcome = match caps.engine {
        DataPlaneEngine::SingBox => probe_singbox(cfg, target).unwrap_or_else(|e| engine_error(e)),
        DataPlaneEngine::AmneziaWg => probe_amneziawg(cfg, target).unwrap_or_else(|e| engine_error(e)),
    };
    let failure = outcome.failure.map(|(layer, error)| {
        classify(
            &FailureInput {
                failed_layer: layer,
                error,
                transport_l4: caps.l4,
                local_network_control_ok: None,
                endpoint_ok_from_other_vantage: None,
                same_port_control_ok: None,
                other_transports_same_node_failed: None,
                repetitions: 1,
            },
            &ClassifierPolicy::default(),
        )
        .class
    });
    ProbeResult {
        vantage_id: cfg.vantage_id.clone(),
        vantage: cfg.vantage(),
        route_id: target.route_id.clone(),
        node_id: target.node_id.clone(),
        transport: target.transport.clone(),
        at_ms: started,
        layers: outcome.layers,
        failure,
        handshake_ms: outcome.handshake_ms,
        rtt_ms: outcome.rtt_ms,
        jitter_ms: None,
        loss_permille: None,
        transfer_kbps: outcome.transfer_kbps,
    }
}

fn engine_error(e: anyhow::Error) -> Outcome {
    eprintln!("probe engine error: {e:#}");
    let mut o = Outcome::new();
    o.fail(HealthLayer::Tunnel, ErrorKind::Other);
    o
}

fn wants(t: &TargetConfig, l: HealthLayer) -> bool {
    t.layers.contains(&l)
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn wait_port(port: u16, child: &mut Child, deadline: Duration) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!("engine exited early with {status}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("engine did not open its local port within {deadline:?}")
}

struct Killer(Child);
impl Drop for Killer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `curl` result: (exit code, http code, bytes, seconds, bytes/s).
struct Curl {
    exit: i32,
    http: u32,
    size: u64,
    seconds: f64,
    speed: f64,
}

fn curl(prefix: &[&str], extra: &[&str], url: &str, max_time: u64) -> Result<Curl> {
    let mut cmd = Command::new(prefix.first().copied().unwrap_or("curl"));
    let mut args: Vec<String> = prefix.iter().skip(1).map(|s| s.to_string()).collect();
    if !prefix.is_empty() {
        args.push("curl".into());
    }
    args.extend(
        ["-sS", "-o", "/dev/null", "--max-time", &max_time.to_string(), "-w", "%{http_code} %{size_download} %{time_total} %{speed_download}"]
            .iter()
            .map(|s| s.to_string()),
    );
    args.extend(extra.iter().map(|s| s.to_string()));
    args.push(url.into());
    let out = cmd.args(&args).stderr(Stdio::null()).output().context("running curl")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut p = text.split_whitespace();
    Ok(Curl {
        exit: out.status.code().unwrap_or(-1),
        http: p.next().and_then(|v| v.parse().ok()).unwrap_or(0),
        size: p.next().and_then(|v| v.parse().ok()).unwrap_or(0),
        seconds: p.next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
        speed: p.next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
    })
}

fn tcp_l1(host: &str, port: u16) -> std::result::Result<u32, ErrorKind> {
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|_| ErrorKind::DnsNxDomain)?.collect();
    let addr = addrs.first().ok_or(ErrorKind::DnsNxDomain)?;
    let start = Instant::now();
    match TcpStream::connect_timeout(addr, Duration::from_secs(5)) {
        Ok(_) => Ok(start.elapsed().as_millis() as u32),
        Err(e) => Err(match e.kind() {
            std::io::ErrorKind::ConnectionRefused => ErrorKind::ConnectRefused,
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => ErrorKind::ConnectTimeout,
            std::io::ErrorKind::ConnectionReset => ErrorKind::ConnectReset,
            _ => ErrorKind::HostUnreachable,
        }),
    }
}

/// Infer what the client saw from its own error log.
pub fn infer_singbox_error(log: &str) -> ErrorKind {
    let l = log.to_ascii_lowercase();
    if l.contains("authentication") || l.contains("auth failed") || l.contains("unauthorized") {
        ErrorKind::AuthRejected
    } else if l.contains("connection refused") {
        ErrorKind::ConnectRefused
    } else if l.contains("connection reset") || l.contains("broken pipe") || l.contains("eof") {
        ErrorKind::HandshakeReset
    } else if l.contains("timeout") || l.contains("deadline exceeded") {
        ErrorKind::HandshakeTimeout
    } else if l.contains("tls") || l.contains("reality") || l.contains("certificate") {
        ErrorKind::TlsAlert
    } else {
        ErrorKind::Other
    }
}

fn probe_singbox(cfg: &ProbeConfig, t: &TargetConfig) -> Result<Outcome> {
    let mut o = Outcome::new();
    let mut outbound: serde_json::Value = serde_json::from_slice(&std::fs::read(&t.profile)?)
        .with_context(|| format!("profile {:?} is not sing-box outbound JSON", t.profile))?;
    let host = outbound["server"].as_str().context("profile has no server")?.to_string();
    let port = outbound["server_port"].as_u64().context("profile has no server_port")? as u16;
    let l4 = t.transport.capabilities().expect("known").l4;

    if l4 == L4Protocol::Tcp && wants(t, HealthLayer::Reachability) {
        match tcp_l1(&host, port) {
            Ok(ms) => {
                o.rtt_ms = Some(ms);
                o.pass(HealthLayer::Reachability, Some(ms));
            }
            Err(kind) => {
                o.fail(HealthLayer::Reachability, kind);
                return Ok(o);
            }
        }
    }

    let dir = tempdir()?;
    let local = free_port()?;
    outbound["tag"] = "probe".into();
    let config = serde_json::json!({
        "log": {"level": "warn"},
        "inbounds": [{"type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": local}],
        "outbounds": [outbound],
        "route": {"final": "probe"},
    });
    let cfg_path = dir.join("client.json");
    write_private(&cfg_path, &serde_json::to_vec(&config)?)?;
    let log_path = dir.join("client.log");
    let child = Command::new(&cfg.singbox_binary)
        .args(["run", "-c"])
        .arg(&cfg_path)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log_path)?)
        .spawn()
        .with_context(|| format!("starting {:?}", cfg.singbox_binary))?;
    let mut child = Killer(child);
    if let Err(e) = wait_port(local, &mut child.0, Duration::from_secs(10)) {
        eprintln!("sing-box client did not start: {e:#}");
        o.fail(HealthLayer::Tunnel, ErrorKind::Other);
        return Ok(o);
    }

    let proxy = format!("socks5h://127.0.0.1:{local}");
    let r = curl(&[], &["--proxy", &proxy], &cfg.l4_url, 20)?;
    if r.exit == 0 && matches!(r.http, 200 | 204) {
        let ms = (r.seconds * 1000.0) as u32;
        o.handshake_ms = Some(ms);
        o.pass(HealthLayer::Handshake, None);
        o.pass(HealthLayer::Tunnel, None);
        o.pass(HealthLayer::Internet, Some(ms));
    } else {
        std::thread::sleep(Duration::from_millis(200));
        let mut log = String::new();
        let _ = std::fs::File::open(&log_path).and_then(|mut f| f.read_to_string(&mut log));
        let kind = if r.exit == 0 { ErrorKind::StallAfterHandshake } else { infer_singbox_error(&log) };
        if kind == ErrorKind::StallAfterHandshake {
            o.pass(HealthLayer::Handshake, None);
            o.pass(HealthLayer::Tunnel, None);
            o.fail(HealthLayer::Internet, kind);
        } else {
            o.fail(HealthLayer::Handshake, kind);
        }
        return Ok(o);
    }

    if wants(t, HealthLayer::Transfer) {
        transfer(cfg, &[], &["--proxy", &proxy], &mut o)?;
    }
    Ok(o)
}

fn transfer(cfg: &ProbeConfig, prefix: &[&str], extra: &[&str], o: &mut Outcome) -> Result<()> {
    let url = cfg.l5_url.as_deref().context("l5_url")?;
    let range = format!("0-{}", cfg.l5_bytes.saturating_sub(1));
    let mut args = extra.to_vec();
    args.extend(["-r", &range]);
    let r = curl(prefix, &args, url, cfg.l5_deadline_seconds)?;
    if r.exit == 0 && r.size >= cfg.l5_bytes {
        o.transfer_kbps = Some((r.speed * 8.0 / 1000.0) as u32);
        o.pass(HealthLayer::Transfer, Some((r.seconds * 1000.0) as u32));
    } else {
        o.fail(HealthLayer::Transfer, if r.exit == 28 { ErrorKind::StallAfterHandshake } else { ErrorKind::Other });
    }
    Ok(())
}

fn tempdir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("vpn-probe-{}-{}", std::process::id(), now_ms()));
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Split an awg-quick profile into (`awg setconf` text, addresses, MTU).
pub fn split_awg_quick(ini: &str) -> Result<(String, Vec<String>, Option<u16>)> {
    let mut setconf = String::new();
    let mut addresses = Vec::new();
    let mut mtu = None;
    let mut section = "";
    for raw in ini.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            section = if line.eq_ignore_ascii_case("[interface]") { "interface" } else { "peer" };
            setconf.push_str(line);
            setconf.push('\n');
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if section == "interface" {
            match key {
                "Address" => {
                    addresses.extend(value.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()));
                    continue;
                }
                "MTU" => {
                    mtu = Some(value.parse().context("MTU")?);
                    continue;
                }
                "DNS" | "Table" | "PreUp" | "PostUp" | "PreDown" | "PostDown" | "SaveConfig" => continue,
                _ => {}
            }
        }
        setconf.push_str(&format!("{key} = {value}\n"));
    }
    if addresses.is_empty() {
        bail!("AmneziaWG profile has no Address");
    }
    Ok((setconf, addresses, mtu))
}

fn run(args: &[&str]) -> Result<String> {
    let out = Command::new(args[0]).args(&args[1..]).output().with_context(|| format!("running {}", args[0]))?;
    if !out.status.success() {
        bail!("{} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

struct AwgLab {
    ns: String,
    iface: String,
}

impl Drop for AwgLab {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["netns", "del", &self.ns]).output();
        let _ = Command::new("ip").args(["link", "del", &self.iface]).output();
        let _ = std::fs::remove_file(format!("/var/run/amneziawg/{}.sock", self.iface));
        let _ = std::fs::remove_dir_all(format!("/etc/netns/{}", self.ns));
    }
}

fn probe_amneziawg(cfg: &ProbeConfig, t: &TargetConfig) -> Result<Outcome> {
    let mut o = Outcome::new();
    let ini = std::fs::read_to_string(&t.profile)?;
    let (setconf, addresses, mtu) = split_awg_quick(&ini)?;
    let tag = format!("{}", (std::process::id() as u64 + now_ms()) % 100_000);
    let lab = AwgLab { ns: format!("vprobe{tag}"), iface: format!("awgp{tag}") };
    let dir = tempdir()?;
    let conf = dir.join("setconf.conf");
    write_private(&conf, setconf.as_bytes())?;

    // The UDP socket is created in the host namespace, then the interface
    // moves into a private namespace: probe traffic can only leave through
    // the tunnel, and host routing is never touched.
    let started = Instant::now();
    let setup = || -> Result<()> {
        run(&["ip", "netns", "add", &lab.ns])?;
        run(&[cfg.amneziawg_go_binary.to_str().context("binary path")?, &lab.iface])?;
        run(&[cfg.awg_binary.to_str().context("binary path")?, "setconf", &lab.iface, conf.to_str().context("path")?])?;
        run(&["ip", "link", "set", &lab.iface, "netns", &lab.ns])?;
        for a in &addresses {
            run(&["ip", "-n", &lab.ns, "address", "add", a, "dev", &lab.iface])?;
        }
        let mtu = mtu.unwrap_or(1380).to_string();
        run(&["ip", "-n", &lab.ns, "link", "set", &lab.iface, "mtu", &mtu, "up"])?;
        run(&["ip", "-n", &lab.ns, "link", "set", "lo", "up"])?;
        run(&["ip", "-n", &lab.ns, "route", "add", "default", "dev", &lab.iface])?;
        if addresses.iter().any(|a| a.contains(':')) {
            run(&["ip", "-n", &lab.ns, "-6", "route", "add", "default", "dev", &lab.iface])?;
        }
        let etc = format!("/etc/netns/{}", lab.ns);
        std::fs::create_dir_all(&etc)?;
        std::fs::write(format!("{etc}/resolv.conf"), format!("nameserver {}\n", cfg.awg_namespace_resolver))?;
        Ok(())
    };
    if let Err(e) = setup() {
        eprintln!("AmneziaWG probe setup failed: {e:#}");
        o.fail(HealthLayer::Tunnel, ErrorKind::TunCreateFailed);
        return Ok(o);
    }

    // Any packet triggers the handshake; the resolver address is routed
    // into the tunnel.
    let mut trigger = Command::new("ip")
        .args(["netns", "exec", &lab.ns, "ping", "-c", "6", "-i", "1", "-W", "1", &cfg.awg_namespace_resolver])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let handshake_start = Instant::now();
    let mut handshake = None;
    while handshake_start.elapsed() < Duration::from_secs(8) {
        let text = run(&[cfg.awg_binary.to_str().unwrap_or("awg"), "show", &lab.iface, "latest-handshakes"]).unwrap_or_default();
        if text.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) > 0 {
            handshake = Some(handshake_start.elapsed().as_millis() as u32);
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = trigger.kill();
    let _ = trigger.wait();
    let _ = started;
    match handshake {
        Some(ms) => {
            o.pass(HealthLayer::Tunnel, None);
            o.handshake_ms = Some(ms);
            o.pass(HealthLayer::Handshake, Some(ms));
        }
        None => {
            o.pass(HealthLayer::Tunnel, None);
            o.fail(HealthLayer::Handshake, ErrorKind::NoHandshakeResponse);
            return Ok(o);
        }
    }
    if !wants(t, HealthLayer::Internet) && !wants(t, HealthLayer::Transfer) {
        return Ok(o);
    }
    let prefix = ["ip", "netns", "exec", lab.ns.as_str()];
    let r = curl(&prefix, &[], &cfg.l4_url, 20)?;
    if r.exit == 0 && matches!(r.http, 200 | 204) {
        o.pass(HealthLayer::Internet, Some((r.seconds * 1000.0) as u32));
    } else {
        o.fail(HealthLayer::Internet, ErrorKind::StallAfterHandshake);
        return Ok(o);
    }
    if wants(t, HealthLayer::Transfer) {
        transfer(cfg, &prefix, &[], &mut o)?;
    }
    drop(lab);
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awg_quick_split_keeps_protocol_keys_and_extracts_interface_policy() {
        let ini = "[Interface]\nPrivateKey = k\nAddress = 10.66.0.2/32, fd66::2/128\nDNS = 1.1.1.1\nMTU = 1380\nJc = 4\nH1 = 1-2\n\n[Peer]\nPublicKey = p\nEndpoint = vpn.example.com:51820\nAllowedIPs = 0.0.0.0/0\n";
        let (setconf, addrs, mtu) = split_awg_quick(ini).unwrap();
        assert_eq!(addrs, vec!["10.66.0.2/32", "fd66::2/128"]);
        assert_eq!(mtu, Some(1380));
        assert!(setconf.contains("Jc = 4") && setconf.contains("H1 = 1-2") && setconf.contains("Endpoint = vpn.example.com:51820"));
        assert!(!setconf.contains("DNS") && !setconf.contains("Address") && !setconf.contains("MTU"));
        assert!(split_awg_quick("[Interface]\nPrivateKey = k\n").is_err());
    }

    #[test]
    fn singbox_log_inference_is_conservative() {
        assert_eq!(infer_singbox_error("dial tcp 1.2.3.4:443: i/o timeout"), ErrorKind::HandshakeTimeout);
        assert_eq!(infer_singbox_error("read: connection reset by peer"), ErrorKind::HandshakeReset);
        assert_eq!(infer_singbox_error("connect: connection refused"), ErrorKind::ConnectRefused);
        assert_eq!(infer_singbox_error("hysteria2: authentication failed"), ErrorKind::AuthRejected);
        assert_eq!(infer_singbox_error("something unexpected"), ErrorKind::Other);
    }

    #[test]
    fn a_single_probe_never_classifies_censorship() {
        for kind in [ErrorKind::HandshakeReset, ErrorKind::HandshakeTimeout, ErrorKind::NoHandshakeResponse] {
            let c = classify(
                &FailureInput {
                    failed_layer: HealthLayer::Handshake,
                    error: kind,
                    transport_l4: L4Protocol::Udp,
                    local_network_control_ok: None,
                    endpoint_ok_from_other_vantage: None,
                    same_port_control_ok: None,
                    other_transports_same_node_failed: None,
                    repetitions: 1,
                },
                &ClassifierPolicy::default(),
            );
            assert_ne!(c.class, FailureClass::CensorshipSuspected);
        }
    }
}
