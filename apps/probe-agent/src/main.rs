//! `vpn-probe`: synthetic route measurement from a vantage point.
//!
//! ```text
//! vpn-probe run       --config probe.toml --output results.jsonl [--once] [--route ID]
//! vpn-probe summarize --config probe.toml --results results.jsonl [--json]
//! vpn-probe evaluate  --results a.jsonl [--results b.jsonl …] --node fi1 [--json]
//! ```
//!
//! Results from a datacenter VPS support at most a
//! `<COUNTRY>-DATACENTER-NETWORK VERIFIED` label; they are never evidence
//! about residential or mobile networks. See
//! `docs/platform-v2/RUSSIAN_PROBE.md`.

mod config;
mod probes;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::ProbeConfig;
use platform_core::probe::{summarize, ProbeResult};
use platform_core::replacement::{Decision, NodeEvidence, ReplacementOrchestrator, ReplacementPolicy};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "vpn-probe", version, about = "Synthetic route prober (never carries user traffic)")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Probe every target (optionally one route), appending JSONL results.
    Run {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// One pass instead of looping every `interval_seconds`.
        #[arg(long)]
        once: bool,
        #[arg(long)]
        route: Option<String>,
    },
    /// Per-route success, failures and latency, with the evidence label the
    /// vantage can support.
    Summarize {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Would the replacement policy replace `node` on this evidence? Read-only.
    Evaluate {
        #[arg(long, required = true)]
        results: Vec<PathBuf>,
        #[arg(long)]
        node: String,
        #[arg(long)]
        json: bool,
    },
}

fn read_results(path: &Path) -> Result<Vec<ProbeResult>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).with_context(|| format!("{path:?} line {}", i + 1)))
        .collect()
}

fn append(path: &Path, result: &ProbeResult) -> Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o640);
    }
    let mut f = opts.open(path).with_context(|| format!("opening {path:?}"))?;
    let line = serde_json::to_string(result)?;
    // Defense in depth: a result must never carry a credential-shaped value.
    for v in [&result.route_id, &result.node_id, &result.vantage_id] {
        if let Some(shape) = platform_core::telemetry::sensitive_shape(v) {
            anyhow::bail!("refusing to write a probe result whose identifier looks like a {shape}");
        }
    }
    writeln!(f, "{line}")?;
    f.sync_data()?;
    Ok(())
}

fn cmd_run(config: &Path, output: &Path, once: bool, route: Option<&str>) -> Result<()> {
    let cfg = ProbeConfig::load(config)?;
    loop {
        for target in cfg.targets.iter().filter(|t| route.is_none_or(|r| r == t.route_id)) {
            let result = probes::probe(&cfg, target);
            let status = match (result.failure, result.to_observation().is_usable()) {
                (_, true) => "ok".to_string(),
                (Some(class), false) => serde_json::to_string(&class)?.trim_matches('"').to_string(),
                (None, false) => "incomplete".to_string(),
            };
            println!("{} {} {}", result.at_ms, result.route_id, status);
            append(output, &result)?;
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(cfg.interval_seconds));
    }
}

fn cmd_summarize(config: &Path, results: &Path, json: bool) -> Result<()> {
    let cfg = ProbeConfig::load(config)?;
    let matrix = cfg.matrix();
    let results: Vec<ProbeResult> = read_results(results)?
        .into_iter()
        .filter(|r| r.vantage_id == cfg.vantage_id)
        .collect();
    let summary = summarize(&matrix, &results);
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    println!("vantage {} ({:?}); {} result(s)", cfg.vantage_id, cfg.vantage(), results.len());
    for s in summary {
        let failures: Vec<String> = s
            .failures
            .iter()
            .map(|(c, n)| format!("{}={n}", serde_json::to_string(c).unwrap_or_default().trim_matches('"')))
            .collect();
        println!(
            "{:<28} {:<14} runs {:>4}  L4 ok {:>4}  L5 ok {:>4}  hs {:>6}  rtt {:>6}  [{}]  {}",
            s.route_id,
            s.transport.as_str(),
            s.runs,
            s.l4_ok,
            s.l5_ok,
            s.median_handshake_ms.map(|v| format!("{v}ms")).unwrap_or_else(|| "-".into()),
            s.median_rtt_ms.map(|v| format!("{v}ms")).unwrap_or_else(|| "-".into()),
            failures.join(" "),
            s.evidence_label
        );
    }
    Ok(())
}

fn cmd_evaluate(results: &[PathBuf], node: &str, json: bool) -> Result<()> {
    let mut evidence: Vec<NodeEvidence> = Vec::new();
    let mut latest = 0;
    for path in results {
        for r in read_results(path)? {
            latest = latest.max(r.at_ms);
            evidence.extend(r.to_node_evidence());
        }
    }
    let mut orchestrator = ReplacementOrchestrator::new(ReplacementPolicy::default());
    orchestrator.adopt_active(node, None, "unknown", 0);
    let decision = orchestrator.evaluate(node, &evidence, latest);
    if json {
        println!("{}", serde_json::to_string_pretty(&decision)?);
    } else {
        match decision {
            Decision::Replace { failures, vantages } => println!(
                "{node}: REPLACEMENT JUSTIFIED by policy ({failures} qualifying failures from {vantages} vantage(s)); nothing was changed"
            ),
            Decision::Hold { reason } => println!("{node}: hold ({reason:?}); nothing was changed"),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Run { config, output, once, route } => cmd_run(&config, &output, once, route.as_deref()),
        Cmd::Summarize { config, results, json } => cmd_summarize(&config, &results, json),
        Cmd::Evaluate { results, node, json } => cmd_evaluate(&results, &node, json),
    }
}
