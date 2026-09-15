//! Benchmark scenarios, results and honest comparison.
//!
//! Results are only comparable when they were measured in the same
//! environment class with the same scenario and metric. Anything else is
//! reported as `NOT COMPARABLE`, never as a win or a loss. Specification:
//! `docs/platform-v2/BENCHMARK_SPEC.md`.

use crate::evidence::NetworkClass;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Normal,
    HighLatency,
    PacketLoss,
    Jitter,
    UdpBlocked,
    UdpDegraded,
    TcpInterruption,
    DnsFailure,
    MtuProblems,
    Ipv4Only,
    Ipv6Only,
    DualStack,
    ServerRestart,
    TransportProcessCrash,
    NodeUnavailable,
    NetworkHandover,
    SleepWake,
}

/// Emulated impairment applied to the client's uplink (Linux `tc netem`
/// plus nftables drops). `None` fields mean "not impaired".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Impairment {
    pub delay_ms: Option<u32>,
    pub jitter_ms: Option<u32>,
    pub loss_permille: Option<u16>,
    pub rate_kbit: Option<u32>,
    pub drop_udp: bool,
    pub udp_loss_permille: Option<u16>,
    pub mtu: Option<u16>,
    pub block_dns: bool,
    pub tcp_reset_after_ms: Option<u32>,
}

impl Scenario {
    pub const ALL: [Scenario; 17] = [
        Scenario::Normal,
        Scenario::HighLatency,
        Scenario::PacketLoss,
        Scenario::Jitter,
        Scenario::UdpBlocked,
        Scenario::UdpDegraded,
        Scenario::TcpInterruption,
        Scenario::DnsFailure,
        Scenario::MtuProblems,
        Scenario::Ipv4Only,
        Scenario::Ipv6Only,
        Scenario::DualStack,
        Scenario::ServerRestart,
        Scenario::TransportProcessCrash,
        Scenario::NodeUnavailable,
        Scenario::NetworkHandover,
        Scenario::SleepWake,
    ];

    /// Reproducible impairment for emulated runs, or `None` when the
    /// scenario is an event (restart, handover) or needs real hardware.
    pub fn impairment(self) -> Option<Impairment> {
        let base = Impairment::default();
        Some(match self {
            Scenario::Normal | Scenario::Ipv4Only | Scenario::Ipv6Only | Scenario::DualStack => {
                base
            }
            Scenario::HighLatency => Impairment {
                delay_ms: Some(300),
                ..base
            },
            Scenario::PacketLoss => Impairment {
                loss_permille: Some(30),
                ..base
            },
            Scenario::Jitter => Impairment {
                delay_ms: Some(80),
                jitter_ms: Some(60),
                ..base
            },
            Scenario::UdpBlocked => Impairment {
                drop_udp: true,
                ..base
            },
            Scenario::UdpDegraded => Impairment {
                udp_loss_permille: Some(150),
                ..base
            },
            Scenario::TcpInterruption => Impairment {
                tcp_reset_after_ms: Some(10_000),
                ..base
            },
            Scenario::DnsFailure => Impairment {
                block_dns: true,
                ..base
            },
            Scenario::MtuProblems => Impairment {
                mtu: Some(1280),
                ..base
            },
            Scenario::ServerRestart
            | Scenario::TransportProcessCrash
            | Scenario::NodeUnavailable
            | Scenario::NetworkHandover
            | Scenario::SleepWake => return None,
        })
    }

    /// Whether the scenario can be run without physical devices.
    pub fn is_emulatable(self) -> bool {
        !matches!(self, Scenario::NetworkHandover | Scenario::SleepWake)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    ConnectionSuccessRatePermille,
    TimeToUsableInternetMs,
    HandshakeMs,
    RecoveryMs,
    ThroughputKbps,
    LatencyMs,
    JitterMs,
    LossPermille,
    LargeTransferCompletedPermille,
    CpuPermille,
    RssKib,
    BatteryMwh,
    Ipv4LeakCount,
    Ipv6LeakCount,
    DnsLeakCount,
}

impl Metric {
    /// Whether a larger value is better.
    pub fn higher_is_better(self) -> bool {
        matches!(
            self,
            Metric::ConnectionSuccessRatePermille
                | Metric::ThroughputKbps
                | Metric::LargeTransferCompletedPermille
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub class: NetworkClass,
    /// Free-form but stable description (host, kernel, emulator version).
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// e.g. "tamara+amneziawg", "amneziavpn+amneziawg".
    pub subject: String,
    pub scenario: Scenario,
    pub metric: Metric,
    pub environment: Environment,
    pub samples: Vec<u64>,
    pub commit: String,
    pub date: String,
}

impl BenchmarkResult {
    pub fn median(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut s = self.samples.clone();
        s.sort_unstable();
        Some(s[(s.len() - 1) / 2])
    }
}

/// Scorecard cell vocabulary. No numeric opinion scores exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Pass,
    Fail,
    Tbd,
    Unverified,
    NotComparable,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Tbd => "TBD",
            Verdict::Unverified => "UNVERIFIED",
            Verdict::NotComparable => "NOT COMPARABLE",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    pub verdict: Verdict,
    /// Set only for comparable results.
    pub ours_median: Option<u64>,
    pub theirs_median: Option<u64>,
    pub note: String,
}

/// Minimum samples per side before a comparison is allowed.
pub const MIN_SAMPLES: usize = 5;

/// Compare `ours` against `theirs`. `Pass` means ours is at least as good
/// as theirs within `tolerance_permille`; `Fail` means worse.
pub fn compare(
    ours: Option<&BenchmarkResult>,
    theirs: Option<&BenchmarkResult>,
    tolerance_permille: u64,
) -> Comparison {
    let (Some(a), Some(b)) = (ours, theirs) else {
        return Comparison {
            verdict: Verdict::Unverified,
            ours_median: None,
            theirs_median: None,
            note: "missing measurement".into(),
        };
    };
    let not_comparable = |note: &str| Comparison {
        verdict: Verdict::NotComparable,
        ours_median: None,
        theirs_median: None,
        note: note.into(),
    };
    if a.scenario != b.scenario || a.metric != b.metric {
        return not_comparable("different scenario or metric");
    }
    if a.environment.class != b.environment.class {
        return not_comparable("different environment class");
    }
    if a.samples.len() < MIN_SAMPLES || b.samples.len() < MIN_SAMPLES {
        return Comparison {
            verdict: Verdict::Tbd,
            ours_median: a.median(),
            theirs_median: b.median(),
            note: format!("fewer than {MIN_SAMPLES} samples"),
        };
    }
    let (ma, mb) = (a.median().unwrap_or(0), b.median().unwrap_or(0));
    let slack = mb.saturating_mul(tolerance_permille) / 1000;
    let ok = if a.metric.higher_is_better() {
        ma + slack >= mb
    } else {
        ma <= mb + slack
    };
    Comparison {
        verdict: if ok { Verdict::Pass } else { Verdict::Fail },
        ours_median: Some(ma),
        theirs_median: Some(mb),
        note: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(subject: &str, class: NetworkClass, metric: Metric, samples: &[u64]) -> BenchmarkResult {
        BenchmarkResult {
            subject: subject.into(),
            scenario: Scenario::Normal,
            metric,
            environment: Environment {
                class,
                description: "netns".into(),
            },
            samples: samples.to_vec(),
            commit: "abc".into(),
            date: "2026-09-15".into(),
        }
    }

    #[test]
    fn different_environments_are_not_comparable() {
        let ours = r(
            "tamara+awg",
            NetworkClass::Simulated,
            Metric::HandshakeMs,
            &[1, 2, 3, 4, 5],
        );
        let theirs = r(
            "amnezia+awg",
            NetworkClass::Datacenter,
            Metric::HandshakeMs,
            &[9, 9, 9, 9, 9],
        );
        assert_eq!(
            compare(Some(&ours), Some(&theirs), 50).verdict,
            Verdict::NotComparable
        );
    }

    #[test]
    fn missing_or_thin_data_is_not_a_verdict() {
        let ours = r("a", NetworkClass::Simulated, Metric::HandshakeMs, &[1, 2]);
        let theirs = r(
            "b",
            NetworkClass::Simulated,
            Metric::HandshakeMs,
            &[1, 2, 3, 4, 5],
        );
        assert_eq!(compare(Some(&ours), None, 50).verdict, Verdict::Unverified);
        assert_eq!(
            compare(Some(&ours), Some(&theirs), 50).verdict,
            Verdict::Tbd
        );
    }

    #[test]
    fn direction_and_tolerance() {
        let fast = r("a", NetworkClass::Simulated, Metric::HandshakeMs, &[100; 5]);
        let slow = r("b", NetworkClass::Simulated, Metric::HandshakeMs, &[104; 5]);
        assert_eq!(compare(Some(&fast), Some(&slow), 0).verdict, Verdict::Pass);
        assert_eq!(compare(Some(&slow), Some(&fast), 0).verdict, Verdict::Fail);
        assert_eq!(compare(Some(&slow), Some(&fast), 50).verdict, Verdict::Pass);
        let tp_low = r(
            "a",
            NetworkClass::Simulated,
            Metric::ThroughputKbps,
            &[900; 5],
        );
        let tp_high = r(
            "b",
            NetworkClass::Simulated,
            Metric::ThroughputKbps,
            &[1000; 5],
        );
        assert_eq!(
            compare(Some(&tp_low), Some(&tp_high), 50).verdict,
            Verdict::Fail
        );
        assert_eq!(
            compare(Some(&tp_high), Some(&tp_low), 0).verdict,
            Verdict::Pass
        );
    }

    #[test]
    fn every_scenario_is_classified() {
        for s in Scenario::ALL {
            if s.impairment().is_none() {
                assert!(matches!(
                    s,
                    Scenario::ServerRestart
                        | Scenario::TransportProcessCrash
                        | Scenario::NodeUnavailable
                        | Scenario::NetworkHandover
                        | Scenario::SleepWake
                ));
            }
        }
        assert!(!Scenario::SleepWake.is_emulatable());
        assert_eq!(Verdict::NotComparable.to_string(), "NOT COMPARABLE");
    }
}
