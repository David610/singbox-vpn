//! Protocol-level synthetic health probing (Phase 4 "protocol health").
//!
//! The Worker control plane cannot speak UDP or run a sing-box client, so
//! every real handshake measurement is taken here, on a node: the agent
//! spawns a short-lived local sing-box *client* against a PEER node's public
//! REALITY / Hysteria2 endpoint (and against its own endpoint over loopback
//! as a fallback), then drives real requests through it:
//!
//! | dimension      | how it is measured                                            |
//! |----------------|---------------------------------------------------------------|
//! | `tcp_connect`  | plain TCP connect to the REALITY listener (TCP only)          |
//! | `handshake`    | any request through the tunnel completed                      |
//! | `https_ipv4`   | `https://1.1.1.1/cdn-cgi/trace` (IP literal, no DNS needed)   |
//! | `dns`          | `https://www.gstatic.com/generate_204` via socks5h, so the    |
//! |                | hostname is resolved by the *server*                          |
//! | `egress_ipv4`  | `ip=` from the trace, compared to the target's expected IP    |
//! | `ipv6`         | AAAA-only `https://ipv6.icanhazip.com` via socks5h: `egress`  |
//! |                | when it works, `blocked` when it fails while IPv4+DNS work    |
//! | `latency_ms`   | median over `SAMPLES` IPv4 requests; `loss_pct` over them     |
//!
//! Probe credentials: each node owns one reserved, non-customer probe user
//! (`probe_user_name`, default `arcana-probe`) created through the normal
//! `vpn-admin user create` path. Its share links are published to the
//! control plane (`POST /api/agent/probe-credential`), which hands them only
//! to other authenticated agents (`GET /api/agent/probe-targets`). A probe
//! credential grants proxy egress on one node and nothing else: it is not
//! tied to any customer, subscription, device or traffic record.
//!
//! Secrets (UUIDs, passwords, obfs passwords) are never logged: errors are
//! reduced to short classes before they reach `tracing` or the report.

use crate::config::{AgentConfig, ProtocolProbeConfig, StaticProbeTarget};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TRACE_V4_URL: &str = "https://1.1.1.1/cdn-cgi/trace";
/// AAAA-only hostname: resolved by the server (socks5h), so success proves
/// the server's IPv6 egress. (An IPv6 *literal* through reqwest's socks5
/// proxy is not sent as an address, so a literal cannot be used here.)
const V6_ONLY_URL: &str = "https://ipv6.icanhazip.com";
const DNS_URL: &str = "https://www.gstatic.com/generate_204";
const SAMPLES: usize = 3;
/// Temp dirs holding a probe client config (which carries the probe
/// credential) use this prefix so a dir orphaned by a hard kill mid-probe
/// is swept on the next agent start.
const PROBE_DIR_PREFIX: &str = ".vpn-probe-";

/// Removes probe config dirs left behind by a previous process that was
/// killed mid-probe (Drop never ran).
pub fn sweep_stale_probe_dirs(base: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(base) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(PROBE_DIR_PREFIX)
        })
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Reality,
    Hysteria2,
}

/// A parsed probe endpoint. Never `Debug`-printed with secrets: the
/// derived impl is replaced below.
#[derive(Clone, PartialEq)]
pub enum Endpoint {
    Reality {
        host: String,
        port: u16,
        uuid: String,
        flow: Option<String>,
        sni: String,
        fingerprint: String,
        public_key: String,
        short_id: String,
    },
    Hysteria2 {
        host: String,
        port: u16,
        password: String,
        sni: String,
        obfs: Option<(String, String)>,
    },
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, host, port) = match self {
            Endpoint::Reality { host, port, .. } => ("reality", host, port),
            Endpoint::Hysteria2 { host, port, .. } => ("hysteria2", host, port),
        };
        write!(f, "Endpoint({kind} {host}:{port} <credentials redacted>)")
    }
}

impl Endpoint {
    pub fn protocol(&self) -> Protocol {
        match self {
            Endpoint::Reality { .. } => Protocol::Reality,
            Endpoint::Hysteria2 { .. } => Protocol::Hysteria2,
        }
    }

    fn with_host(&self, new_host: &str) -> Self {
        let mut ep = self.clone();
        match &mut ep {
            Endpoint::Reality { host, .. } | Endpoint::Hysteria2 { host, .. } => {
                *host = new_host.to_string()
            }
        }
        ep
    }

    fn host_port(&self) -> (&str, u16) {
        match self {
            Endpoint::Reality { host, port, .. } | Endpoint::Hysteria2 { host, port, .. } => {
                (host.as_str(), *port)
            }
        }
    }

    /// Parses a `vless://` (REALITY) or `hysteria2://` share link as
    /// rendered by `crates/compat-config/src/render.rs`.
    pub fn parse(uri: &str) -> Result<Self> {
        let url = reqwest::Url::parse(uri).map_err(|_| anyhow!("unparseable probe URI"))?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("probe URI has no host"))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let port = url.port().unwrap_or(443);
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let user = percent_decode(url.username());
        match url.scheme() {
            "vless" => {
                if q.get("security").map(String::as_str) != Some("reality") {
                    bail!("vless probe URI is not a REALITY endpoint");
                }
                Ok(Endpoint::Reality {
                    host,
                    port,
                    uuid: user,
                    flow: q.get("flow").cloned().filter(|f| !f.is_empty()),
                    sni: q.get("sni").cloned().unwrap_or_default(),
                    fingerprint: q.get("fp").cloned().unwrap_or_else(|| "chrome".into()),
                    public_key: q
                        .get("pbk")
                        .cloned()
                        .ok_or_else(|| anyhow!("REALITY probe URI has no pbk"))?,
                    short_id: q.get("sid").cloned().unwrap_or_default(),
                })
            }
            "hysteria2" | "hy2" => Ok(Endpoint::Hysteria2 {
                host,
                port,
                password: user,
                sni: q.get("sni").cloned().unwrap_or_default(),
                obfs: match (q.get("obfs"), q.get("obfs-password")) {
                    (Some(kind), Some(pw)) if !kind.is_empty() => Some((kind.clone(), pw.clone())),
                    _ => None,
                },
            }),
            _ => bail!("unsupported probe URI scheme"),
        }
    }

    /// The sing-box client outbound for this endpoint.
    fn outbound(&self, tls_insecure: bool) -> Value {
        match self {
            Endpoint::Reality {
                host,
                port,
                uuid,
                flow,
                sni,
                fingerprint,
                public_key,
                short_id,
            } => {
                let mut ob = json!({
                    "type": "vless", "tag": "probe",
                    "server": host, "server_port": port, "uuid": uuid,
                    "tls": {
                        "enabled": true, "server_name": sni,
                        "utls": { "enabled": true, "fingerprint": fingerprint },
                        "reality": { "enabled": true, "public_key": public_key, "short_id": short_id }
                    }
                });
                if let Some(flow) = flow {
                    ob["flow"] = json!(flow);
                }
                ob
            }
            Endpoint::Hysteria2 {
                host,
                port,
                password,
                sni,
                obfs,
            } => {
                let mut ob = json!({
                    "type": "hysteria2", "tag": "probe",
                    "server": host, "server_port": port, "password": password,
                    "tls": { "enabled": true, "server_name": sni, "insecure": tls_insecure }
                });
                if let Some((kind, pw)) = obfs {
                    ob["obfs"] = json!({ "type": kind, "password": pw });
                }
                ob
            }
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One peer (or self) this agent should probe.
#[derive(Debug, Clone, Deserialize)]
pub struct ProbeTarget {
    pub node_id: String,
    #[serde(default)]
    pub expected_ipv4: Option<String>,
    #[serde(default)]
    pub reality_uri: Option<String>,
    #[serde(default)]
    pub hysteria2_uri: Option<String>,
}

impl From<&StaticProbeTarget> for ProbeTarget {
    fn from(t: &StaticProbeTarget) -> Self {
        Self {
            node_id: t.node_id.clone(),
            expected_ipv4: t.expected_ipv4.clone(),
            reality_uri: t.reality_uri.clone(),
            hysteria2_uri: t.hysteria2_uri.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Dimensions {
    pub tcp_connect: Option<bool>,
    pub handshake: bool,
    pub https_ipv4: bool,
    pub dns: bool,
    /// "egress" | "blocked" | "unknown"
    pub ipv6: &'static str,
    pub egress_ipv4: Option<String>,
    pub egress_ipv6: Option<String>,
    pub egress_ip_match: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProbeResult {
    pub target_node_id: String,
    /// "peer" or "self" (loopback fallback).
    pub vantage: &'static str,
    pub protocol: Protocol,
    /// Useful egress: handshake completed AND IPv4 HTTPS worked.
    pub ok: bool,
    pub dims: Dimensions,
    pub latency_ms: Option<u64>,
    pub loss_pct: u8,
    pub failure_streak: u32,
    pub error: Option<&'static str>,
}

/// A completed probe round, attached to exactly one heartbeat (taken, not
/// copied) so the control plane never counts one round twice.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    pub version: u32,
    pub round: u64,
    /// Surfaced to the control plane so an insecure probe config cannot
    /// run unnoticed.
    pub tls_insecure_for_tests: bool,
    pub hysteria2_cert_days_remaining: Option<i64>,
    pub results: Vec<ProbeResult>,
}

pub type SharedReport = Arc<Mutex<Option<ProbeReport>>>;

pub fn take_report(shared: &SharedReport) -> Option<Value> {
    let report = shared.lock().ok()?.take()?;
    serde_json::to_value(report).ok()
}

/// Background loop: publish own probe credential, fetch targets, probe,
/// stash the report for the next heartbeat.
pub async fn run_loop(cfg: AgentConfig, shared: SharedReport) {
    let Some(pcfg) = cfg.protocol_probe.clone() else {
        return;
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("building reqwest client");
    let interval = Duration::from_secs(pcfg.interval_secs.clamp(15, 600));
    let mut streaks: HashMap<(String, &'static str, String), u32> = HashMap::new();
    let mut own: Option<ProbeTarget> = None;
    let mut published = false;
    let mut round = 0_u64;
    if pcfg.tls_insecure_for_tests {
        tracing::error!(
            "protocol_probe.tls_insecure_for_tests is ON: Hysteria2 certificates are NOT verified; \
             test rigs only, never production"
        );
    }
    let swept = sweep_stale_probe_dirs(&std::env::temp_dir());
    if swept > 0 {
        tracing::info!(swept, "removed stale probe config dirs");
    }
    loop {
        if own.is_none() && pcfg.publish_self {
            match tokio::task::block_in_place(|| ensure_probe_user(&cfg, &pcfg)) {
                Ok(t) => own = Some(t),
                Err(e) => tracing::warn!(error = %e, "ensuring the reserved probe user failed"),
            }
        }
        // Republish periodically: the control plane may have dropped the
        // row (or rejected an early publish) and must converge.
        if round.is_multiple_of(30) {
            published = false;
        }
        if let (false, Some(t)) = (published, own.as_ref()) {
            match publish_credential(&http, &cfg, t).await {
                Ok(()) => published = true,
                Err(e) => tracing::warn!(error = %e, "publishing probe credential failed"),
            }
        }

        let mut targets: Vec<ProbeTarget> = pcfg.static_targets.iter().map(Into::into).collect();
        if pcfg.fetch_targets {
            match fetch_targets(&http, &cfg).await {
                Ok(mut t) => targets.append(&mut t),
                Err(e) => tracing::warn!(error = %e, "fetching probe targets failed"),
            }
        }
        targets.retain(|t| t.node_id != cfg.node_id);
        targets.dedup_by(|a, b| a.node_id == b.node_id);

        let mut results = Vec::new();
        for t in &targets {
            results.extend(probe_target(&pcfg, t, "peer", None).await);
        }
        if pcfg.self_probe {
            if let Some(t) = own.as_ref() {
                results.extend(probe_target(&pcfg, t, "self", Some(&pcfg.self_probe_host)).await);
            }
        }
        for r in &mut results {
            let key = (
                r.target_node_id.clone(),
                r.vantage,
                format!("{:?}", r.protocol),
            );
            let s = streaks.entry(key).or_insert(0);
            *s = if r.ok { 0 } else { s.saturating_add(1) };
            r.failure_streak = *s;
        }
        round += 1;
        let report = ProbeReport {
            version: 1,
            round,
            tls_insecure_for_tests: pcfg.tls_insecure_for_tests,
            hysteria2_cert_days_remaining: pcfg
                .hysteria2_cert_path
                .as_deref()
                .and_then(cert_days_remaining),
            results,
        };
        tracing::info!(
            round,
            summary = %summarize(&report),
            "protocol probe round complete"
        );
        if let Ok(mut slot) = shared.lock() {
            *slot = Some(report);
        }
        tokio::time::sleep(interval).await;
    }
}

fn summarize(report: &ProbeReport) -> String {
    report
        .results
        .iter()
        .map(|r| {
            format!(
                "{}/{}/{:?}={}",
                r.target_node_id,
                r.vantage,
                r.protocol,
                if r.ok {
                    "ok"
                } else {
                    r.error.unwrap_or("fail")
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn probe_target(
    pcfg: &ProtocolProbeConfig,
    t: &ProbeTarget,
    vantage: &'static str,
    host_override: Option<&str>,
) -> Vec<ProbeResult> {
    let mut out = Vec::new();
    for uri in [t.reality_uri.as_deref(), t.hysteria2_uri.as_deref()]
        .into_iter()
        .flatten()
    {
        let Ok(mut ep) = Endpoint::parse(uri) else {
            tracing::warn!(target = %t.node_id, "skipping unparseable probe URI");
            continue;
        };
        if let Some(h) = host_override {
            ep = ep.with_host(h);
        }
        let expected = if vantage == "peer" {
            t.expected_ipv4.as_deref()
        } else {
            None
        };
        out.push(probe_endpoint(pcfg, &t.node_id, vantage, &ep, expected).await);
    }
    out
}

/// Runs every dimension for one endpoint through a fresh sing-box client.
pub async fn probe_endpoint(
    pcfg: &ProtocolProbeConfig,
    target_node_id: &str,
    vantage: &'static str,
    ep: &Endpoint,
    expected_ipv4: Option<&str>,
) -> ProbeResult {
    let timeout = Duration::from_secs(pcfg.timeout_secs.clamp(2, 30));
    let mut dims = Dimensions {
        tcp_connect: None,
        handshake: false,
        https_ipv4: false,
        dns: false,
        ipv6: "unknown",
        egress_ipv4: None,
        egress_ipv6: None,
        egress_ip_match: None,
    };
    let mut result = ProbeResult {
        target_node_id: target_node_id.to_string(),
        vantage,
        protocol: ep.protocol(),
        ok: false,
        dims: dims.clone(),
        latency_ms: None,
        loss_pct: 100,
        failure_streak: 0,
        error: None,
    };

    let (host, port) = ep.host_port();
    if ep.protocol() == Protocol::Reality {
        let ok = tcp_connect(host, port, Duration::from_secs(3)).await;
        dims.tcp_connect = Some(ok);
        if !ok {
            result.dims = dims;
            result.error = Some("tcp_unreachable");
            return result;
        }
    }

    let client = match SingBoxClient::start(&pcfg.singbox_binary, ep, pcfg.tls_insecure_for_tests) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "could not start probe sing-box client");
            result.dims = dims;
            result.error = Some("client_start_failed");
            return result;
        }
    };
    if !client.wait_ready(Duration::from_secs(5)).await {
        result.dims = dims;
        result.error = Some("client_not_ready");
        return result;
    }
    let (Ok(http), Ok(dns_http)) = (client.http(timeout, false), client.http(timeout, true)) else {
        result.dims = dims;
        result.error = Some("client_build_failed");
        return result;
    };

    let mut latencies = Vec::new();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        if let Ok(body) = get_text(&http, TRACE_V4_URL).await {
            latencies.push(started.elapsed().as_millis() as u64);
            if dims.egress_ipv4.is_none() {
                dims.egress_ipv4 = trace_ip(&body);
            }
        }
    }
    dims.https_ipv4 = !latencies.is_empty();
    result.loss_pct = (((SAMPLES - latencies.len()) * 100) / SAMPLES) as u8;
    latencies.sort_unstable();
    result.latency_ms = latencies.get(latencies.len() / 2).copied();

    dims.dns = matches!(
        dns_http
            .get(DNS_URL)
            .send()
            .await
            .map(|r| r.status().as_u16()),
        Ok(204)
    );
    match get_text(&dns_http, V6_ONLY_URL).await {
        Ok(body) => {
            dims.ipv6 = "egress";
            let ip = body.trim();
            dims.egress_ipv6 = (ip.contains(':') && ip.len() <= 64).then(|| ip.to_string());
        }
        // Only call it blocked when IPv4 and DNS both work, so the failure
        // is attributable to IPv6 itself.
        Err(_) if dims.https_ipv4 && dims.dns => dims.ipv6 = "blocked",
        Err(_) => dims.ipv6 = "unknown",
    }
    dims.handshake = dims.https_ipv4 || dims.dns || dims.ipv6 == "egress";
    if let (Some(expected), Some(seen)) = (expected_ipv4, dims.egress_ipv4.as_deref()) {
        dims.egress_ip_match = Some(expected == seen);
    }
    result.ok = dims.handshake && dims.https_ipv4;
    result.error = if result.ok {
        None
    } else if !dims.handshake {
        Some("handshake_failed")
    } else {
        Some("no_ipv4_egress")
    };
    result.dims = dims;
    result
}

async fn get_text(http: &reqwest::Client, url: &str) -> Result<String> {
    let res = http.get(url).send().await?;
    if !res.status().is_success() {
        bail!("status {}", res.status());
    }
    Ok(res.text().await?)
}

fn trace_ip(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("ip="))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.len() <= 64)
}

async fn tcp_connect(host: &str, port: u16, timeout: Duration) -> bool {
    let addr = format!("{host}:{port}");
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

struct SingBoxClient {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Drop for SingBoxClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl SingBoxClient {
    fn start(binary: &str, ep: &Endpoint, tls_insecure: bool) -> Result<Self> {
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let dir = tempfile::Builder::new()
            .prefix(PROBE_DIR_PREFIX)
            .tempdir()?;
        let path = dir.path().join("probe.json");
        let config = client_config(ep, port, tls_insecure);
        // tempdir is 0700 and owned by this process; the file carries the
        // probe credential and is removed with the directory on drop.
        let mut f = std::fs::File::create(&path)?;
        f.write_all(config.to_string().as_bytes())?;
        drop(f);
        let child = Command::new(binary)
            .args(["run", "-c"])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawning sing-box")?;
        Ok(Self {
            child,
            port,
            _dir: dir,
        })
    }

    async fn wait_ready(&self, max: Duration) -> bool {
        let deadline = Instant::now() + max;
        while Instant::now() < deadline {
            if tokio::net::TcpStream::connect(("127.0.0.1", self.port))
                .await
                .is_ok()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// `remote_dns = true` (socks5h) hands hostnames to the server to
    /// resolve — what the DNS dimension needs. IP-literal URLs use plain
    /// socks5 so the literal travels as an address, not as a bracketed
    /// "hostname" the server would fail to resolve (IPv6 literals).
    fn http(&self, timeout: Duration, remote_dns: bool) -> Result<reqwest::Client> {
        let scheme = if remote_dns { "socks5h" } else { "socks5" };
        Ok(reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!(
                "{scheme}://127.0.0.1:{}",
                self.port
            ))?)
            .timeout(timeout)
            .build()?)
    }
}

pub fn client_config(ep: &Endpoint, port: u16, tls_insecure: bool) -> Value {
    json!({
        "log": { "level": "error" },
        "inbounds": [{ "type": "mixed", "tag": "in", "listen": "127.0.0.1", "listen_port": port }],
        "outbounds": [ep.outbound(tls_insecure)],
        "route": { "final": "probe" }
    })
}

/// Days until the local Hysteria2 certificate's notAfter (negative once
/// expired). `None` when the file or `openssl` is unavailable.
pub fn cert_days_remaining(path: &str) -> Option<i64> {
    let out = Command::new("openssl")
        .args(["x509", "-enddate", "-noout", "-in", path])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let not_after = parse_openssl_date(text.trim().strip_prefix("notAfter=")?)?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    Some((not_after - now).div_euclid(86_400))
}

/// Parses `Sep 29 12:00:00 2026 GMT` into unix seconds.
fn parse_openssl_date(s: &str) -> Option<i64> {
    let normalized = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let fmt = time::format_description::parse_borrowed::<2>(
        "[month repr:short] [day padding:none] [hour]:[minute]:[second] [year] GMT",
    )
    .ok()?;
    time::PrimitiveDateTime::parse(&normalized, &fmt)
        .ok()
        .map(|dt| dt.assume_utc().unix_timestamp())
}

/// Ensures this node's reserved probe user exists and returns its links
/// as a self-target. Idempotent: an existing user of that name is reused.
fn ensure_probe_user(cfg: &AgentConfig, pcfg: &ProtocolProbeConfig) -> Result<ProbeTarget> {
    let admin = |args: &[&str]| -> Result<String> {
        let out = Command::new(&cfg.vpn_admin_binary)
            .arg("--config")
            .arg(&cfg.vpn_admin_config)
            .args(args)
            .output()
            .context("running vpn-admin")?;
        if !out.status.success() {
            // stderr can echo arguments; never forward it.
            bail!(
                "vpn-admin {} exited with {}",
                args[0..2].join(" "),
                out.status
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let list = admin(&["user", "list"])?;
    let id = match find_user_id(&list, &pcfg.probe_user_name) {
        Some(id) => id,
        None => {
            let created = admin(&["user", "create", "--name", &pcfg.probe_user_name, "--json"])?;
            let v: Value = serde_json::from_str(created.trim()).context("parsing user create")?;
            v.get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("user create returned no id"))?
                .to_string()
        }
    };
    let links = admin(&["user", "links", &id])?;
    let mut t = ProbeTarget {
        node_id: cfg.node_id.clone(),
        expected_ipv4: None,
        reality_uri: None,
        hysteria2_uri: None,
    };
    for line in links.lines().map(str::trim) {
        if line.starts_with("vless://") && t.reality_uri.is_none() {
            t.reality_uri = Some(line.to_string());
        } else if (line.starts_with("hysteria2://") || line.starts_with("hy2://"))
            && t.hysteria2_uri.is_none()
        {
            t.hysteria2_uri = Some(line.to_string());
        }
    }
    if t.reality_uri.is_none() && t.hysteria2_uri.is_none() {
        bail!("vpn-admin user links printed no usable URI");
    }
    Ok(t)
}

/// `vpn-admin user list` prints a header then one row per user; the row
/// whose name column equals `name` yields the id in the first column.
fn find_user_id(list: &str, name: &str) -> Option<String> {
    list.lines().skip(1).find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        (cols.len() >= 2 && cols[1] == name).then(|| cols[0].to_string())
    })
}

async fn publish_credential(
    http: &reqwest::Client,
    cfg: &AgentConfig,
    t: &ProbeTarget,
) -> Result<()> {
    let res = http
        .post(format!("{}/api/agent/probe-credential", cfg.worker_url))
        .bearer_auth(&cfg.agent_api_key)
        .json(&json!({ "reality_uri": t.reality_uri, "hysteria2_uri": t.hysteria2_uri }))
        .send()
        .await
        .context("POST /api/agent/probe-credential failed")?;
    if !res.status().is_success() {
        bail!("POST /api/agent/probe-credential returned {}", res.status());
    }
    Ok(())
}

#[derive(Deserialize)]
struct TargetsResponse {
    targets: Vec<ProbeTarget>,
}

async fn fetch_targets(http: &reqwest::Client, cfg: &AgentConfig) -> Result<Vec<ProbeTarget>> {
    let res = http
        .get(format!("{}/api/agent/probe-targets", cfg.worker_url))
        .bearer_auth(&cfg.agent_api_key)
        .send()
        .await
        .context("GET /api/agent/probe-targets failed")?;
    if !res.status().is_success() {
        bail!("GET /api/agent/probe-targets returned {}", res.status());
    }
    Ok(res.json::<TargetsResponse>().await?.targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VLESS: &str = "vless://11111111-1111-4111-8111-111111111111@vpn.example.com:443?encryption=none&security=reality&sni=www.cloudflare.com&fp=chrome&pbk=PUBKEY&sid=abcd&type=tcp&flow=xtls-rprx-vision#n1";
    const HY2: &str = "hysteria2://hy2%2Bpass@vpn.example.com:8443?sni=vpn.example.com&insecure=0&obfs=salamander&obfs-password=ob#n1";

    #[test]
    fn parses_reality_share_link() {
        let ep = Endpoint::parse(VLESS).unwrap();
        match &ep {
            Endpoint::Reality {
                host,
                port,
                flow,
                public_key,
                short_id,
                sni,
                ..
            } => {
                assert_eq!(host, "vpn.example.com");
                assert_eq!(*port, 443);
                assert_eq!(flow.as_deref(), Some("xtls-rprx-vision"));
                assert_eq!(public_key, "PUBKEY");
                assert_eq!(short_id, "abcd");
                assert_eq!(sni, "www.cloudflare.com");
            }
            _ => panic!("wrong kind"),
        }
        assert_eq!(ep.protocol(), Protocol::Reality);
    }

    #[test]
    fn parses_hysteria2_share_link_with_obfs_and_encoded_password() {
        let ep = Endpoint::parse(HY2).unwrap();
        match &ep {
            Endpoint::Hysteria2 {
                port,
                password,
                obfs,
                ..
            } => {
                assert_eq!(*port, 8443);
                assert_eq!(password, "hy2+pass");
                assert_eq!(obfs.as_ref().unwrap().0, "salamander");
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn rejects_non_reality_vless_and_unknown_schemes() {
        assert!(Endpoint::parse("vless://u@h:1?security=tls").is_err());
        assert!(Endpoint::parse("ss://x@h:1").is_err());
        assert!(Endpoint::parse("not a uri").is_err());
    }

    #[test]
    fn debug_never_prints_credentials() {
        let ep = Endpoint::parse(HY2).unwrap();
        let s = format!("{ep:?}");
        assert!(!s.contains("hy2+pass") && !s.contains("ob#") && s.contains("redacted"));
    }

    #[test]
    fn host_override_is_used_for_loopback_self_probe() {
        let ep = Endpoint::parse(VLESS).unwrap().with_host("127.0.0.1");
        assert_eq!(ep.host_port(), ("127.0.0.1", 443));
    }

    #[test]
    fn client_config_routes_everything_through_the_probe_outbound() {
        let ep = Endpoint::parse(HY2).unwrap();
        let c = client_config(&ep, 1080, false);
        assert_eq!(c["route"]["final"], "probe");
        assert_eq!(c["outbounds"][0]["type"], "hysteria2");
        assert_eq!(c["outbounds"][0]["obfs"]["type"], "salamander");
        assert_eq!(c["outbounds"][0]["tls"]["insecure"], false);
        let r = client_config(&Endpoint::parse(VLESS).unwrap(), 1080, true);
        assert_eq!(r["outbounds"][0]["tls"]["reality"]["public_key"], "PUBKEY");
        assert_eq!(r["outbounds"][0]["flow"], "xtls-rprx-vision");
    }

    #[test]
    fn sweeps_only_probe_prefixed_dirs() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join(".vpn-probe-abc")).unwrap();
        std::fs::write(base.path().join(".vpn-probe-abc/probe.json"), "{}").unwrap();
        std::fs::create_dir(base.path().join(".tmpother")).unwrap();
        assert_eq!(sweep_stale_probe_dirs(base.path()), 1);
        assert!(!base.path().join(".vpn-probe-abc").exists());
        assert!(base.path().join(".tmpother").exists());
    }

    #[test]
    fn trace_ip_extracts_the_egress_address() {
        assert_eq!(
            trace_ip("fl=1\nip=62.238.46.190\nts=1\n").as_deref(),
            Some("62.238.46.190")
        );
        assert_eq!(trace_ip("fl=1\n"), None);
    }

    #[test]
    fn parses_openssl_enddate_formats() {
        assert_eq!(
            parse_openssl_date("Jan  1 00:00:00 2030 GMT"),
            Some(1_893_456_000)
        );
        assert!(parse_openssl_date("Sep 29 12:00:00 2026 GMT").is_some());
        assert!(parse_openssl_date("garbage").is_none());
    }

    #[test]
    fn finds_probe_user_in_vpn_admin_list() {
        let list = "ID NAME ENABLED\nabc123 alice true\ndef456 arcana-probe true\n";
        assert_eq!(
            find_user_id(list, "arcana-probe").as_deref(),
            Some("def456")
        );
        assert_eq!(find_user_id(list, "bob"), None);
    }

    #[tokio::test]
    async fn unreachable_reality_endpoint_fails_fast_on_tcp_without_spawning() {
        let pcfg = ProtocolProbeConfig::default();
        // Port 1 refuses immediately.
        let ep = Endpoint::parse(&VLESS.replace("vpn.example.com:443", "127.0.0.1:1")).unwrap();
        let r = probe_endpoint(&pcfg, "peer-1", "peer", &ep, Some("1.2.3.4")).await;
        assert!(!r.ok);
        assert_eq!(r.dims.tcp_connect, Some(false));
        assert_eq!(r.error, Some("tcp_unreachable"));
        assert_eq!(r.loss_pct, 100);
    }
}
