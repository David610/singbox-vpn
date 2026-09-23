use crate::config::AgentConfig;
use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Serialize)]
pub struct Heartbeat {
    pub agent_version: String,
    pub vpn_version: Option<String>,
    pub singbox_version: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub disk_percent: Option<f64>,
    pub network_rx_bps: Option<u64>,
    pub network_tx_bps: Option<u64>,
    pub configured_users: Option<u64>,
    pub active_users_recent: Option<u64>,
}

#[derive(Default)]
pub struct TelemetrySampler {
    last_cpu: Option<(u64, u64)>, // total, idle
    last_net: Option<(u64, u64, Instant)>,
    vpn_version: Option<String>,
    singbox_version: Option<String>,
}

impl TelemetrySampler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sample(&mut self, cfg: &AgentConfig) -> Heartbeat {
        let cpu = read_cpu().and_then(|current| {
            let percent = self.last_cpu.and_then(|previous| cpu_percent(previous, current));
            self.last_cpu = Some(current);
            percent
        });

        let net = read_network_totals().and_then(|(rx, tx)| {
            let now = Instant::now();
            let rates = self.last_net.and_then(|(prev_rx, prev_tx, prev_at)| {
                let elapsed = now.duration_since(prev_at).as_secs_f64();
                if elapsed <= 0.0 {
                    return None;
                }
                Some((
                    ((rx.saturating_sub(prev_rx)) as f64 / elapsed) as u64,
                    ((tx.saturating_sub(prev_tx)) as f64 / elapsed) as u64,
                ))
            });
            self.last_net = Some((rx, tx, now));
            rates
        });

        if self.vpn_version.is_none() {
            self.vpn_version = command_first_line(&cfg.vpn_admin_binary, &["--version"]);
        }
        if self.singbox_version.is_none() {
            self.singbox_version = command_first_line("sing-box", &["version"]);
        }

        Heartbeat {
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            vpn_version: self.vpn_version.clone(),
            singbox_version: self.singbox_version.clone(),
            uptime_seconds: read_uptime(),
            cpu_percent: cpu,
            memory_percent: read_memory_percent(),
            disk_percent: read_disk_percent(),
            network_rx_bps: net.map(|v| v.0),
            network_tx_bps: net.map(|v| v.1),
            configured_users: configured_users(cfg),
            // Reliable per-user recency comes from the optional stats
            // collector, not from process-level network activity.
            active_users_recent: None,
        }
    }
}

fn command_first_line(binary: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(binary).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn configured_users(cfg: &AgentConfig) -> Option<u64> {
    let output = std::process::Command::new(&cfg.vpn_admin_binary)
        .arg("--config")
        .arg(&cfg.vpn_admin_config)
        .args(["user", "list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(stdout.lines().skip(1).filter(|line| !line.trim().is_empty()).count() as u64)
}

fn read_uptime() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/uptime").ok()?;
    text.split_whitespace().next()?.parse::<f64>().ok().map(|v| v as u64)
}

fn read_cpu() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_line(text.lines().next()?)
}

fn parse_cpu_line(line: &str) -> Option<(u64, u64)> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = parts.filter_map(|v| v.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let total = values.iter().copied().sum();
    let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

fn cpu_percent(previous: (u64, u64), current: (u64, u64)) -> Option<f64> {
    let total = current.0.saturating_sub(previous.0);
    if total == 0 {
        return None;
    }
    let idle = current.1.saturating_sub(previous.1);
    let busy = total.saturating_sub(idle);
    Some(((busy as f64 / total as f64) * 100.0).clamp(0.0, 100.0))
}

fn read_memory_percent() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_memory_percent(&text)
}

fn parse_memory_percent(text: &str) -> Option<f64> {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        let mut p = line.split_whitespace();
        match p.next()? {
            "MemTotal:" => total = p.next()?.parse::<f64>().ok(),
            "MemAvailable:" => available = p.next()?.parse::<f64>().ok(),
            _ => {}
        }
    }
    let total = total?;
    let available = available?;
    if total <= 0.0 {
        return None;
    }
    Some((((total - available) / total) * 100.0).clamp(0.0, 100.0))
}

fn read_disk_percent() -> Option<f64> {
    let output = std::process::Command::new("df").args(["-Pk", "/"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.lines().nth(1)?;
    let percent = line.split_whitespace().nth(4)?.trim_end_matches('%');
    percent.parse::<f64>().ok().map(|v| v.clamp(0.0, 100.0))
}

fn read_network_totals() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    parse_network_totals(&text)
}

fn parse_network_totals(text: &str) -> Option<(u64, u64)> {
    let mut rx = 0u64;
    let mut tx = 0u64;
    let mut found = false;
    for line in text.lines().skip(2) {
        let (iface, rest) = line.split_once(':')?;
        if iface.trim() == "lo" {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 16 {
            continue;
        }
        rx = rx.saturating_add(fields[0].parse::<u64>().ok()?);
        tx = tx.saturating_add(fields[8].parse::<u64>().ok()?);
        found = true;
    }
    found.then_some((rx, tx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_parser_and_delta_are_bounded() {
        let a = parse_cpu_line("cpu  100 0 50 850 0 0 0 0").unwrap();
        let b = parse_cpu_line("cpu  140 0 60 900 0 0 0 0").unwrap();
        let value = cpu_percent(a, b).unwrap();
        assert!((0.0..=100.0).contains(&value));
        assert!(value > 0.0);
    }

    #[test]
    fn memory_parser_uses_available_memory() {
        let value = parse_memory_percent("MemTotal: 1000 kB\nMemAvailable: 250 kB\n").unwrap();
        assert!((value - 75.0).abs() < 0.01);
    }

    #[test]
    fn network_parser_excludes_loopback() {
        let text = "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0\n eth0: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\n";
        assert_eq!(parse_network_totals(text), Some((100, 200)));
    }
}
