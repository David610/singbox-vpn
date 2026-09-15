//! Live AmneziaWG interoperability against pinned upstream binaries.
//!
//! Proves that what `compat_config::amneziawg` renders is accepted by the
//! real `awg` tools and produces a working, correctly-rejecting
//! amneziawg-go data plane between two network namespaces on one host.
//!
//! Requirements (the test prints SKIP and passes when any is missing,
//! so ordinary `cargo test` stays unprivileged):
//!
//! * running as root (network namespaces, TUN devices);
//! * `AMNEZIAWG_GO_BIN` — amneziawg-go built from the pinned commit;
//! * `AWG_BIN` — `awg` built from the pinned amneziawg-tools commit;
//! * `AWG_QUICK_BIN` — upstream `wg-quick/linux.bash` (used only for its
//!   `strip` parser);
//! * `ip`, `ping`, `curl`, `python3`, `timeout`.
//!
//! Set `AMNEZIAWG_INTEROP_REQUIRED=1` in evidence runs: a missing
//! requirement then fails the test instead of skipping it, so an
//! unavailable environment can never be recorded as a pass.
//!
//! Evidence level when it runs: LOCAL-VERIFIED live data plane
//! (loopback namespaces on one kernel). It says nothing about real
//! networks, providers or censorship.

use compat_config::amneziawg::{
    derive_public_key, generate_private_key, render_client_awg_quick, render_server_setconf,
    AmneziaWgProvider, AwgCredential, AwgNodeConfig, AwgParams, AwgProfile,
};
use platform_core::transport::TransportProvider;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Bins {
    go: String,
    awg: String,
    quick: String,
}

fn skip(reason: &str) -> Option<Bins> {
    if std::env::var("AMNEZIAWG_INTEROP_REQUIRED").as_deref() == Ok("1") {
        panic!("amneziawg_interop required but unavailable: {reason}");
    }
    eprintln!("SKIP amneziawg_interop: {reason}");
    None
}

fn bins() -> Option<Bins> {
    let uid = Command::new("id").arg("-u").output().ok()?;
    if String::from_utf8_lossy(&uid.stdout).trim() != "0" {
        return skip("not root");
    }
    let get = |k: &str| std::env::var(k).ok().filter(|p| Path::new(p).exists());
    match (get("AMNEZIAWG_GO_BIN"), get("AWG_BIN"), get("AWG_QUICK_BIN")) {
        (Some(go), Some(awg), Some(quick)) => Some(Bins { go, awg, quick }),
        _ => skip("AMNEZIAWG_GO_BIN/AWG_BIN/AWG_QUICK_BIN not set to existing files"),
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .unwrap_or_else(|e| panic!("spawn {args:?}: {e}"))
}

fn ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Two namespaces joined by a veth pair, each with an amneziawg-go device.
struct Lab {
    tag: String,
    dir: PathBuf,
    server_if: String,
    client_if: String,
    bins: Bins,
}

impl Lab {
    fn new(bins: Bins, name: &str) -> Lab {
        let tag = format!("{name}{}", std::process::id() % 10_000);
        let dir = std::env::temp_dir().join(format!("awg-interop-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let lab = Lab {
            server_if: format!("as{tag}"),
            client_if: format!("ac{tag}"),
            tag,
            dir,
            bins,
        };
        let (s, c) = (lab.ns("s"), lab.ns("c"));
        ok(&["ip", "netns", "add", &s]);
        ok(&["ip", "netns", "add", &c]);
        let (vs, vc) = (format!("vs{}", lab.tag), format!("vc{}", lab.tag));
        ok(&["ip", "link", "add", &vs, "type", "veth", "peer", "name", &vc]);
        ok(&["ip", "link", "set", &vs, "netns", &s]);
        ok(&["ip", "link", "set", &vc, "netns", &c]);
        ok(&["ip", "-n", &s, "addr", "add", "192.0.2.1/24", "dev", &vs]);
        ok(&["ip", "-n", &c, "addr", "add", "192.0.2.2/24", "dev", &vc]);
        for (ns, dev) in [(&s, &vs), (&c, &vc)] {
            ok(&["ip", "-n", ns, "link", "set", dev, "up"]);
            ok(&["ip", "-n", ns, "link", "set", "lo", "up"]);
        }
        ok(&["ip", "netns", "exec", &s, &lab.bins.go, &lab.server_if]);
        ok(&["ip", "netns", "exec", &c, &lab.bins.go, &lab.client_if]);
        lab
    }

    fn ns(&self, side: &str) -> String {
        format!("awgi-{side}-{}", self.tag)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    /// Parse the fallback profile through upstream awg-quick's own `strip`.
    fn strip(&self, ini: &str) -> String {
        let path = self.write(&format!("{}.conf", self.client_if), ini);
        let out = Command::new("bash")
            .arg(&self.bins.quick)
            .arg("strip")
            .arg(&path)
            .env("PATH", format!("{}:{}", Path::new(&self.bins.awg).parent().unwrap().display(), std::env::var("PATH").unwrap_or_default()))
            .output()
            .unwrap();
        assert!(out.status.success(), "awg-quick strip: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    fn setconf(&self, iface: &str, text: &str) -> Output {
        let path = self.write(&format!("{iface}.setconf"), text);
        run(&[&self.bins.awg, "setconf", iface, path.to_str().unwrap()])
    }

    fn syncconf(&self, iface: &str, text: &str) {
        let path = self.write(&format!("{iface}.sync"), text);
        ok(&[&self.bins.awg, "syncconf", iface, path.to_str().unwrap()]);
    }

    fn bring_up(&self, server_conf: &str, client_ini: &str, client_v4: &str) {
        self.bring_up_mtu(server_conf, client_ini, client_v4, None)
    }

    fn bring_up_mtu(&self, server_conf: &str, client_ini: &str, client_v4: &str, mtu: Option<u16>) {
        let out = self.setconf(&self.server_if, server_conf);
        assert!(out.status.success(), "server setconf: {}", String::from_utf8_lossy(&out.stderr));
        // Point the client at the veth address instead of the public host.
        let stripped = self.strip(client_ini).replace("vpn.example.com", "192.0.2.1");
        let out = self.setconf(&self.client_if, &stripped);
        assert!(out.status.success(), "client setconf: {}", String::from_utf8_lossy(&out.stderr));
        let (s, c) = (self.ns("s"), self.ns("c"));
        ok(&["ip", "-n", &s, "addr", "add", "10.66.0.1/24", "dev", &self.server_if]);
        ok(&["ip", "-n", &s, "link", "set", &self.server_if, "up"]);
        ok(&["ip", "-n", &c, "addr", "add", &format!("{client_v4}/32"), "dev", &self.client_if]);
        ok(&["ip", "-n", &c, "link", "set", &self.client_if, "up"]);
        ok(&["ip", "-n", &c, "route", "add", "10.66.0.0/24", "dev", &self.client_if]);
        if let Some(mtu) = mtu {
            let mtu = mtu.to_string();
            ok(&["ip", "-n", &s, "link", "set", &self.server_if, "mtu", &mtu]);
            ok(&["ip", "-n", &c, "link", "set", &self.client_if, "mtu", &mtu]);
        }
    }

    fn transfer_mbit(&self, bytes: usize) -> f64 {
        let blob = self.write("blob", &"x".repeat(bytes));
        let mut server_http = Command::new("ip")
            .args(["netns", "exec", &self.ns("s"), "python3", "-m", "http.server", "18080", "--bind", "10.66.0.1", "-d"])
            .arg(blob.parent().unwrap())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(800));
        let out = run(&[
            "ip", "netns", "exec", &self.ns("c"), "timeout", "60", "curl", "-sS", "-o", "/dev/null",
            "-w", "%{size_download} %{speed_download}", "http://10.66.0.1:18080/blob",
        ]);
        let _ = server_http.kill();
        let _ = server_http.wait();
        let summary = String::from_utf8_lossy(&out.stdout).to_string();
        let mut parts = summary.split_whitespace();
        let size: usize = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let bps: f64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        assert_eq!(size, bytes, "transfer incomplete: {summary} {}", String::from_utf8_lossy(&out.stderr));
        bps * 8.0 / 1_000_000.0
    }

    fn ping(&self, count: &str) -> bool {
        run(&["ip", "netns", "exec", &self.ns("c"), "ping", "-c", count, "-W", "2", "10.66.0.1"]).status.success()
    }

    fn latest_handshake(&self, iface: &str) -> u64 {
        ok(&[&self.bins.awg, "show", iface, "latest-handshakes"])
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Time from the first triggering packet to a completed handshake.
    fn handshake_within(&self, limit: Duration) -> Option<Duration> {
        let start = Instant::now();
        let mut trigger = Command::new("ip")
            .args(["netns", "exec", &self.ns("c"), "ping", "-c", "8", "-i", "0.5", "10.66.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut result = None;
        while start.elapsed() < limit {
            if self.latest_handshake(&self.client_if) > 0 {
                result = Some(start.elapsed());
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = trigger.kill();
        let _ = trigger.wait();
        result
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for side in ["s", "c"] {
            let ns = self.ns(side);
            let pids = run(&["ip", "netns", "pids", &ns]);
            for pid in String::from_utf8_lossy(&pids.stdout).split_whitespace() {
                let _ = run(&["kill", pid]);
            }
            let _ = run(&["ip", "netns", "del", &ns]);
        }
        for iface in [&self.server_if, &self.client_if] {
            let _ = std::fs::remove_file(format!("/var/run/amneziawg/{iface}.sock"));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn replace_line(text: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key} = ");
    let mut replaced = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                replaced = true;
                format!("{prefix}{value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    assert!(replaced, "{key} not found");
    out.join("
") + "
"
}

fn node(profile: AwgProfile) -> AwgNodeConfig {
    let server_private_key = generate_private_key();
    AwgNodeConfig {
        interface: "awg0".into(),
        public_host: "vpn.example.com".into(),
        listen_port: 51820,
        subnet_v4: "10.66.0.0/24".parse().unwrap(),
        subnet_v6: None,
        mtu: 1380,
        persistent_keepalive: 0,
        fallback_client_dns: vec![],
        server_public_key: derive_public_key(server_private_key.expose()).unwrap(),
        server_private_key: Some(server_private_key),
        params: AwgParams::generate(profile),
    }
}

fn issue(cfg: &AwgNodeConfig) -> AwgCredential {
    AmneziaWgProvider.issue_credentials(cfg, "alice", &[]).unwrap()
}

#[test]
fn awg3_profile_handshakes_and_transfers_with_upstream() {
    let Some(bins) = bins() else { return };
    let lab = Lab::new(bins, "ok");
    let mut cfg = node(AwgProfile::Awg3);
    cfg.params.signature_packets = vec!["<b 0xc0ffee><r 24><t>".into()];
    let cred = issue(&cfg);
    let server = render_server_setconf(&cfg, &[("alice", &cred)]).unwrap();
    let client = render_client_awg_quick(&cfg, &cred).unwrap();
    lab.bring_up_mtu(&server, &client, &cred.address_v4.to_string(), Some(cfg.mtu));

    let hs = lab.handshake_within(Duration::from_secs(10)).expect("no handshake with upstream amneziawg-go");
    assert!(lab.ping("3"), "no traffic after handshake");

    let mbit = lab.transfer_mbit(8 * 1024 * 1024);
    eprintln!(
        "EVIDENCE amneziawg awg-3 LOCAL-VERIFIED: handshake {} ms, 8 MiB at {:.1} Mbit/s (netns loopback; not a network measurement)",
        hs.as_millis(),
        mbit
    );
}

#[test]
fn awg2_compatible_profile_handshakes_with_upstream() {
    let Some(bins) = bins() else { return };
    let lab = Lab::new(bins, "v2");
    let cfg = node(AwgProfile::Awg2Compatible);
    let cred = issue(&cfg);
    lab.bring_up(
        &render_server_setconf(&cfg, &[("alice", &cred)]).unwrap(),
        &render_client_awg_quick(&cfg, &cred).unwrap(),
        &cred.address_v4.to_string(),
    );
    assert!(lab.handshake_within(Duration::from_secs(10)).is_some());
    assert!(lab.ping("2"));
}

#[test]
fn mismatched_server_side_parameters_never_handshake() {
    let Some(bins) = bins() else { return };
    let lab = Lab::new(bins, "mm");
    let cfg = node(AwgProfile::Awg3);
    let cred = issue(&cfg);
    // Same keys, independently generated S/H parameters.
    let mut client_cfg = cfg.clone();
    client_cfg.params = AwgParams::generate(AwgProfile::Awg3);
    client_cfg.params.header_protection_key = cfg.params.header_protection_key.clone();
    assert_ne!(client_cfg.params.headers(), cfg.params.headers());
    lab.bring_up(
        &render_server_setconf(&cfg, &[("alice", &cred)]).unwrap(),
        &render_client_awg_quick(&client_cfg, &cred).unwrap(),
        &cred.address_v4.to_string(),
    );
    assert!(lab.handshake_within(Duration::from_secs(6)).is_none(), "handshake despite mismatched H/S parameters");
}

#[test]
fn wrong_header_protection_key_or_psk_never_handshakes() {
    let Some(bins) = bins() else { return };
    for case in ["hpk", "psk"] {
        let lab = Lab::new(Bins { go: bins.go.clone(), awg: bins.awg.clone(), quick: bins.quick.clone() }, case);
        let cfg = node(AwgProfile::Awg3);
        let cred = issue(&cfg);
        let mut client_cfg = cfg.clone();
        let mut client_cred = cred.clone();
        match case {
            "hpk" => client_cfg.params.header_protection_key = Some(compat_config::amneziawg::generate_symmetric_key()),
            _ => client_cred.preshared_key = compat_config::amneziawg::generate_symmetric_key(),
        }
        lab.bring_up(
            &render_server_setconf(&cfg, &[("alice", &cred)]).unwrap(),
            &render_client_awg_quick(&client_cfg, &client_cred).unwrap(),
            &cred.address_v4.to_string(),
        );
        assert!(lab.handshake_within(Duration::from_secs(6)).is_none(), "{case}: handshake with wrong key");
    }
}

#[test]
fn revoked_peer_loses_the_tunnel_after_apply() {
    let Some(bins) = bins() else { return };
    let lab = Lab::new(bins, "rv");
    let cfg = node(AwgProfile::Awg3);
    let cred = issue(&cfg);
    lab.bring_up(
        &render_server_setconf(&cfg, &[("alice", &cred)]).unwrap(),
        &render_client_awg_quick(&cfg, &cred).unwrap(),
        &cred.address_v4.to_string(),
    );
    assert!(lab.handshake_within(Duration::from_secs(10)).is_some());
    assert!(lab.ping("2"));
    lab.syncconf(&lab.server_if, &render_server_setconf(&cfg, &[]).unwrap());
    assert!(!lab.ping("3"), "revoked peer still passes traffic");
}

#[test]
fn upstream_rejects_what_our_validator_rejects() {
    let Some(bins) = bins() else { return };
    let lab = Lab::new(bins, "vr");
    let cfg = node(AwgProfile::Awg3);
    let good = render_server_setconf(&cfg, &[]).unwrap();

    let overlapping = replace_line(&good, "H2", &cfg.params.h1.to_string());
    let mut ours = cfg.params.clone();
    ours.h2 = ours.h1;
    assert!(ours.validate().is_err());
    assert!(!lab.setconf(&lab.server_if, &overlapping).status.success(), "upstream accepted overlapping headers");

    let small_padding = replace_line(&good, "S1", "11");
    let mut ours = cfg.params.clone();
    ours.s1 = 11;
    assert!(ours.validate().is_err());
    assert!(!lab.setconf(&lab.server_if, &small_padding).status.success(), "upstream accepted S1 < 12 with header protection");

    assert!(lab.setconf(&lab.server_if, &good).status.success());
}

/// Measures what the rendered MTU and the AWG-3 overhead cost, so the
/// defaults are chosen from evidence. Informational: prints numbers,
/// asserts only that every variant transfers completely.
#[test]
fn transfer_cost_by_profile_and_mtu() {
    let Some(bins) = bins() else { return };
    for (profile, mtu) in [
        (AwgProfile::Awg3, None),
        (AwgProfile::Awg3, Some(1380)),
        (AwgProfile::Awg3, Some(1280)),
        (AwgProfile::Awg2Compatible, Some(1380)),
    ] {
        let lab = Lab::new(Bins { go: bins.go.clone(), awg: bins.awg.clone(), quick: bins.quick.clone() }, "mt");
        let cfg = node(profile);
        let cred = issue(&cfg);
        lab.bring_up_mtu(
            &render_server_setconf(&cfg, &[("alice", &cred)]).unwrap(),
            &render_client_awg_quick(&cfg, &cred).unwrap(),
            &cred.address_v4.to_string(),
            mtu,
        );
        assert!(lab.handshake_within(Duration::from_secs(10)).is_some());
        let mbit = lab.transfer_mbit(16 * 1024 * 1024);
        eprintln!("MEASURE {profile:?} tunnel_mtu={mtu:?}: 16 MiB at {mbit:.1} Mbit/s (LOCAL netns)");
        drop(lab);
    }
}
