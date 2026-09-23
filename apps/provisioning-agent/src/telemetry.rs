use crate::config::AgentConfig;
use serde_json::{json, Value};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
struct CpuSample {
    total: u64,
    idle: u64,
}

#[derive(Debug, Clone, Copy)]
struct NetSample {
    rx: u64,
    tx: u64,
}

pub struct TelemetrySampler {
    started: Instant,
    previous_cpu: Option<CpuSample>,
    previous_net: Option<(Instant, NetSample)>,
}

impl TelemetrySampler {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            previous_cpu: read_cpu_sample(),
            previous_net: read_net_sample().map(|sample| (Instant::now(), sample)),
        }
    }

    pub fn collect(&mut self, cfg: &AgentConfig) -> Value {
        let cpu_percent = self.cpu_percent();
        let (network_rx_bps, network_tx_bps) = self.network_bps();

        json!({
            "agent_version": env!("CARGO_PKG_VERSION"),
            // vpn-admin is part of the same workspace/release as this agent.
            "vpn_version": env!("CARGO_PKG_VERSION"),
            "singbox_version": command_first_version("sing-box", &["version"]),
            "uptime_seconds": read_uptime_seconds()
                .unwrap_or_else(|| self.started.elapsed().as_secs()),
            "cpu_percent": cpu_percent,
            "memory_percent": read_memory_percent(),
            "disk_percent": read_disk_percent(),
            "network_rx_bps": network_rx_bps,
            "network_tx_bps": network_tx_bps,
            "configured_users": configured_user_count(cfg),
            // The current supported server does not expose a verified
            // per-user active-session source. Null is intentionally
            // different from zero.
            "active_users_recent": Value::Null,
        })
    }

    fn cpu_percent(&mut self) -> Option<f64> {
        let current = read_cpu_sample()?;
        let previous = self.previous_cpu.replace(current)?;
        let total_delta = current.total.saturating_sub(previous.total);
        let idle_delta = current.idle.saturating_sub(previous.idle);
        if total_delta == 0 {
            return None;
        }
        let busy = total_delta.saturating_sub(idle_delta);
        Some(((busy as f64 / total_delta as f64) * 100.0).clamp(0.0, 100.0))
    }

    fn network_bps(&mut self) -> (Option<u64>, Option<u64>) {
        let now = Instant::now();
        let Some(current) = read_net_sample() else {
            return (None, None);
        };
        let Some((previous_at, previous)) = self.previous_net.replace((now, current)) else {
            return (None, None);
        };
        let elapsed = now.duration_since(previous_at).as_secs_f64();
        if elapsed <= 0.0 {
            return (None, None);
        }
        let rx = current.rx.saturating_sub(previous.rx);
        let tx = current.tx.saturating_sub(previous.tx);
        (
            Some((rx as f64 / elapsed).round() as u64),
            Some((tx as f64 / elapsed).round() as u64),
        )
    }
}

fn read_uptime_seconds() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/uptime").ok()?;
    let seconds = text.split_whitespace().next()?.parse::<f64>().ok()?;
    Some(seconds.max(0.0).floor() as u64)
}

fn read_cpu_sample() -> Option<CpuSample> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    let mut parts = line
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse::<u64>().ok());
    let user = parts.next()?;
    let nice = parts.next()?;
    let system = parts.next()?;
    let idle = parts.next()?;
    let iowait = parts.next().unwrap_or(0);
    let irq = parts.next().unwrap_or(0);
    let softirq = parts.next().unwrap_or(0);
    let steal = parts.next().unwrap_or(0);
    Some(CpuSample {
        total: user + nice + system + idle + iowait + irq + softirq + steal,
        idle: idle + iowait,
    })
}

fn read_memory_percent() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total = value.split_whitespace().next()?.parse::<f64>().ok();
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available = value.split_whitespace().next()?.parse::<f64>().ok();
        }
    }
    let total = total?;
    let available = available?;
    if total <= 0.0 {
        return None;
    }
    Some((((total - available).max(0.0) / total) * 100.0).clamp(0.0, 100.0))
}

fn read_net_sample() -> Option<NetSample> {
    let text = std::fs::read_to_string("/proc/net/dev").ok()?;
    let mut rx = 0_u64;
    let mut tx = 0_u64;
    let mut found = false;
    for line in text.lines().skip(2) {
        let (iface, counters) = line.split_once(':')?;
        if iface.trim() == "lo" {
            continue;
        }
        let values: Vec<&str> = counters.split_whitespace().collect();
        if values.len() < 9 {
            continue;
        }
        rx = rx.saturating_add(values[0].parse::<u64>().ok()?);
        tx = tx.saturating_add(values[8].parse::<u64>().ok()?);
        found = true;
    }
    found.then_some(NetSample { rx, tx })
}

fn read_disk_percent() -> Option<f64> {
    // POSIX df output keeps the percentage field stable and avoids adding a
    // native filesystem-stat dependency solely for one operational metric.
    let output = Command::new("df").args(["-P", "/"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.lines().nth(1)?;
    let pct = line.split_whitespace().nth(4)?.trim_end_matches('%');
    pct.parse::<f64>().ok().map(|v| v.clamp(0.0, 100.0))
}

fn configured_user_count(cfg: &AgentConfig) -> Option<u64> {
    let output = Command::new(&cfg.vpn_admin_binary)
        .arg("--config")
        .arg(&cfg.vpn_admin_config)
        .args(["user", "list"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    // vpn-admin's human list has one header row followed by one user row.
    Some(
        stdout
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .count() as u64,
    )
}

fn command_first_version(binary: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(binary).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let line = stdout.lines().find(|line| !line.trim().is_empty())?.trim();
    let version = line
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    Some(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_readers_never_produce_out_of_range_percentages() {
        if let Some(value) = read_memory_percent() {
            assert!((0.0..=100.0).contains(&value));
        }
        if let Some(value) = read_disk_percent() {
            assert!((0.0..=100.0).contains(&value));
        }
    }

    #[test]
    fn uptime_is_nonzero_on_linux_ci() {
        if cfg!(target_os = "linux") {
            assert!(read_uptime_seconds().unwrap_or(0) > 0);
        }
    }
}
