//! Synthetic probe matrix (e.g. a Russian VPS measuring foreign routes).
//!
//! A probe agent runs controlled measurements against configured
//! endpoints only. It never carries or observes user traffic. Its results
//! are labelled with the vantage they came from, so a datacenter probe
//! can at most support a `…-DATACENTER-NETWORK VERIFIED` claim.

use crate::evidence::{EvidenceLevel, NetworkClass, Vantage};
use crate::failure::FailureClass;
use crate::health::{HealthLayer, LayerResult, Observation};
use crate::replacement::NodeEvidence;
use crate::transport::TransportKind;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeTarget {
    pub route_id: String,
    pub node_id: String,
    pub transport: TransportKind,
    /// Layers to attempt, lowest first; the agent stops at the first
    /// failure.
    pub layers: Vec<HealthLayer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeMatrix {
    pub vantage_id: String,
    pub vantage: Vantage,
    pub interval_ms: u64,
    /// Controlled URL fetched through the tunnel for L4 (expects 204/200).
    pub l4_url: String,
    /// Bounded transfer for L5: bytes and deadline.
    pub l5_bytes: u64,
    pub l5_deadline_ms: u64,
    pub targets: Vec<ProbeTarget>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProbeMatrixError {
    #[error("probe matrix has no targets")]
    Empty,
    #[error("duplicate probe target {0:?}")]
    Duplicate(String),
    #[error("interval must be at least 60 s to stay polite to measured infrastructure")]
    IntervalTooShort,
    #[error("L5 transfer must be between 64 KiB and 64 MiB")]
    TransferOutOfRange,
    #[error("the L4 URL must be https")]
    InsecureUrl,
    #[error("target {0:?} has no layers or unordered layers")]
    BadLayers(String),
}

impl ProbeMatrix {
    pub fn validate(&self) -> Result<(), ProbeMatrixError> {
        if self.targets.is_empty() {
            return Err(ProbeMatrixError::Empty);
        }
        if self.interval_ms < 60_000 {
            return Err(ProbeMatrixError::IntervalTooShort);
        }
        if !(64 * 1024..=64 * 1024 * 1024).contains(&self.l5_bytes) {
            return Err(ProbeMatrixError::TransferOutOfRange);
        }
        // Plain HTTP is accepted only for lab vantages (loopback/simulated
        // system tests against a local target); a real network probe must
        // use TLS so an on-path party cannot fake success.
        let lab = matches!(self.vantage.class, NetworkClass::Simulated | NetworkClass::Loopback);
        if !(self.l4_url.starts_with("https://") || (lab && self.l4_url.starts_with("http://"))) {
            return Err(ProbeMatrixError::InsecureUrl);
        }
        let mut seen = std::collections::BTreeSet::new();
        for t in &self.targets {
            if !seen.insert(&t.route_id) {
                return Err(ProbeMatrixError::Duplicate(t.route_id.clone()));
            }
            if t.layers.is_empty() || t.layers.windows(2).any(|w| w[0] >= w[1]) {
                return Err(ProbeMatrixError::BadLayers(t.route_id.clone()));
            }
        }
        Ok(())
    }

    /// The strongest claim results from this matrix can support.
    pub fn evidence_level(&self) -> EvidenceLevel {
        match self.vantage.class {
            NetworkClass::Simulated | NetworkClass::Loopback => EvidenceLevel::LocalVerified,
            _ => EvidenceLevel::NetworkVerified,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub vantage_id: String,
    pub vantage: Vantage,
    pub route_id: String,
    pub node_id: String,
    pub transport: TransportKind,
    pub at_ms: TimestampMs,
    pub layers: Vec<LayerResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_permille: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_kbps: Option<u32>,
}

impl ProbeResult {
    pub fn to_observation(&self) -> Observation {
        Observation {
            route_id: self.route_id.clone(),
            network_id: format!("vantage:{}", self.vantage_id),
            at_ms: self.at_ms,
            layers: self.layers.clone(),
            failure: self.failure,
            handshake_ms: self.handshake_ms,
            rtt_ms: self.rtt_ms,
            jitter_ms: self.jitter_ms,
            loss_permille: self.loss_permille,
            transfer_kbps: self.transfer_kbps,
        }
    }

    pub fn to_node_evidence(&self) -> Option<NodeEvidence> {
        let class = self.failure?;
        self.to_observation().is_failure().then(|| NodeEvidence {
            node_id: self.node_id.clone(),
            vantage_id: self.vantage_id.clone(),
            at_ms: self.at_ms,
            class,
        })
    }
}

/// Per-route summary of a batch of results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSummary {
    pub route_id: String,
    pub transport: TransportKind,
    pub runs: u32,
    pub l4_ok: u32,
    pub l5_ok: u32,
    pub failures: BTreeMap<FailureClass, u32>,
    pub median_handshake_ms: Option<u32>,
    pub median_rtt_ms: Option<u32>,
    pub evidence_label: String,
}

pub fn summarize(matrix: &ProbeMatrix, results: &[ProbeResult]) -> Vec<RouteSummary> {
    let label = match matrix.evidence_level() {
        EvidenceLevel::NetworkVerified => matrix.vantage.verified_label(),
        other => other.as_label().to_string(),
    };
    matrix
        .targets
        .iter()
        .map(|t| {
            let mine: Vec<&ProbeResult> = results
                .iter()
                .filter(|r| r.route_id == t.route_id)
                .collect();
            let mut failures = BTreeMap::new();
            let mut hs = Vec::new();
            let mut rtt = Vec::new();
            let mut l4_ok = 0;
            let mut l5_ok = 0;
            for r in &mine {
                let obs = r.to_observation();
                if obs.is_usable() {
                    l4_ok += 1;
                } else if obs.is_failure() {
                    *failures
                        .entry(r.failure.unwrap_or(FailureClass::Unknown))
                        .or_insert(0) += 1;
                }
                if obs.layer(HealthLayer::Transfer) == Some(true) {
                    l5_ok += 1;
                }
                hs.extend(r.handshake_ms);
                rtt.extend(r.rtt_ms);
            }
            RouteSummary {
                route_id: t.route_id.clone(),
                transport: t.transport.clone(),
                runs: mine.len() as u32,
                l4_ok,
                l5_ok,
                failures,
                median_handshake_ms: median(hs),
                median_rtt_ms: median(rtt),
                // A route with no successful runs supports no positive claim.
                evidence_label: if l4_ok > 0 {
                    label.clone()
                } else {
                    "UNVERIFIED".into()
                },
            }
        })
        .collect()
}

fn median(mut v: Vec<u32>) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[(v.len() - 1) / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix() -> ProbeMatrix {
        ProbeMatrix {
            vantage_id: "ru-probe-1".into(),
            vantage: Vantage::new(NetworkClass::Datacenter, Some("RU")),
            interval_ms: 15 * 60_000,
            l4_url: "https://probe-target.example.net/generate_204".into(),
            l5_bytes: 4 * 1024 * 1024,
            l5_deadline_ms: 30_000,
            targets: vec![
                ProbeTarget {
                    route_id: "fi1/awg".into(),
                    node_id: "fi1".into(),
                    transport: TransportKind::AmneziaWg,
                    layers: vec![
                        HealthLayer::Handshake,
                        HealthLayer::Internet,
                        HealthLayer::Transfer,
                    ],
                },
                ProbeTarget {
                    route_id: "fi1/reality".into(),
                    node_id: "fi1".into(),
                    transport: TransportKind::VlessReality,
                    layers: vec![
                        HealthLayer::Reachability,
                        HealthLayer::Handshake,
                        HealthLayer::Internet,
                    ],
                },
            ],
        }
    }

    fn result(route: &str, ok: bool, class: Option<FailureClass>) -> ProbeResult {
        let m = matrix();
        let layers = if ok {
            vec![
                LayerResult {
                    layer: HealthLayer::Handshake,
                    ok: true,
                    duration_ms: Some(80),
                },
                LayerResult {
                    layer: HealthLayer::Internet,
                    ok: true,
                    duration_ms: Some(200),
                },
            ]
        } else {
            vec![LayerResult {
                layer: HealthLayer::Handshake,
                ok: false,
                duration_ms: None,
            }]
        };
        ProbeResult {
            vantage_id: m.vantage_id,
            vantage: m.vantage,
            route_id: route.into(),
            node_id: "fi1".into(),
            transport: TransportKind::AmneziaWg,
            at_ms: 0,
            layers,
            failure: class,
            handshake_ms: ok.then_some(80),
            rtt_ms: ok.then_some(45),
            jitter_ms: None,
            loss_permille: None,
            transfer_kbps: None,
        }
    }

    #[test]
    fn matrix_validation() {
        matrix().validate().unwrap();
        let mut m = matrix();
        m.interval_ms = 1_000;
        assert_eq!(m.validate(), Err(ProbeMatrixError::IntervalTooShort));
        let mut m = matrix();
        m.l4_url = "http://x".into();
        assert_eq!(m.validate(), Err(ProbeMatrixError::InsecureUrl));
        m.vantage = Vantage::new(NetworkClass::Loopback, None);
        assert_eq!(m.validate(), Ok(()), "lab vantages may probe a local http target");
        let mut m = matrix();
        m.targets[0].layers = vec![HealthLayer::Internet, HealthLayer::Handshake];
        assert!(matches!(m.validate(), Err(ProbeMatrixError::BadLayers(_))));
        let mut m = matrix();
        m.targets.push(m.targets[0].clone());
        assert!(matches!(m.validate(), Err(ProbeMatrixError::Duplicate(_))));
    }

    #[test]
    fn russian_datacenter_probe_is_labelled_as_such() {
        let m = matrix();
        let results = vec![
            result("fi1/awg", true, None),
            result("fi1/reality", false, Some(FailureClass::HandshakeFailure)),
        ];
        let summary = summarize(&m, &results);
        assert_eq!(
            summary[0].evidence_label,
            "RUSSIAN-DATACENTER-NETWORK VERIFIED"
        );
        assert_eq!(summary[1].evidence_label, "UNVERIFIED");
        assert_eq!(
            summary[1].failures.get(&FailureClass::HandshakeFailure),
            Some(&1)
        );
    }

    #[test]
    fn simulated_probes_never_claim_network_verification() {
        let mut m = matrix();
        m.vantage = Vantage::new(NetworkClass::Simulated, None);
        assert_eq!(m.evidence_level(), EvidenceLevel::LocalVerified);
        assert_eq!(
            summarize(&m, &[result("fi1/awg", true, None)])[0].evidence_label,
            "LOCAL-VERIFIED"
        );
    }

    #[test]
    fn failures_become_node_evidence_successes_do_not() {
        assert!(result("fi1/awg", true, None).to_node_evidence().is_none());
        let e = result("fi1/awg", false, Some(FailureClass::EndpointDown))
            .to_node_evidence()
            .unwrap();
        assert_eq!(
            (e.node_id.as_str(), e.vantage_id.as_str()),
            ("fi1", "ru-probe-1")
        );
    }
}
