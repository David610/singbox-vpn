//! Evidence labels.
//!
//! A claim is only as strong as the environment it was observed in. The
//! types here make the two dimensions that are most often conflated —
//! *what kind of test* and *which network vantage point* — separate
//! values, so a Russian datacenter measurement can never be rendered or
//! compared as a Russian mobile or residential result.

use serde::{Deserialize, Serialize};
use std::fmt;

/// How a behaviour was verified. Ordered from weakest to strongest only
/// within the "verified" family; `Unverified` and `Blocked` are not
/// evidence at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING-KEBAB-CASE")]
pub enum EvidenceLevel {
    Unverified,
    Blocked,
    CodeVerified,
    CiVerified,
    LocalVerified,
    VpsVerified,
    DeviceVerified,
    NetworkVerified,
}

impl EvidenceLevel {
    pub fn as_label(self) -> &'static str {
        match self {
            EvidenceLevel::Unverified => "UNVERIFIED",
            EvidenceLevel::Blocked => "BLOCKED",
            EvidenceLevel::CodeVerified => "CODE-VERIFIED",
            EvidenceLevel::CiVerified => "CI-VERIFIED",
            EvidenceLevel::LocalVerified => "LOCAL-VERIFIED",
            EvidenceLevel::VpsVerified => "VPS-VERIFIED",
            EvidenceLevel::DeviceVerified => "DEVICE-VERIFIED",
            EvidenceLevel::NetworkVerified => "NETWORK-VERIFIED",
        }
    }

    pub fn is_evidence(self) -> bool {
        !matches!(self, EvidenceLevel::Unverified | EvidenceLevel::Blocked)
    }
}

impl fmt::Display for EvidenceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_label())
    }
}

/// The class of network a measurement was taken from. Deliberately not
/// ordered: none of these substitutes for another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkClass {
    /// Emulated impairment (netem, namespaces) — reproducible, not real.
    Simulated,
    /// Loopback or a single host.
    Loopback,
    /// A hosting provider's network (a VPS used as a vantage point).
    Datacenter,
    /// A fixed-line consumer ISP.
    Residential,
    /// A cellular operator.
    Mobile,
}

impl NetworkClass {
    fn label(self) -> &'static str {
        match self {
            NetworkClass::Simulated => "SIMULATED-NETWORK",
            NetworkClass::Loopback => "LOOPBACK",
            NetworkClass::Datacenter => "DATACENTER-NETWORK",
            NetworkClass::Residential => "RESIDENTIAL-NETWORK",
            NetworkClass::Mobile => "MOBILE-NETWORK",
        }
    }
}

/// Where a network measurement was observed from. `country` is an
/// ISO 3166-1 alpha-2 code declared by the operator, never geolocated.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Vantage {
    pub class: NetworkClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

impl Vantage {
    pub fn new(class: NetworkClass, country: Option<&str>) -> Self {
        Vantage {
            class,
            country: country.map(|c| c.to_ascii_uppercase()),
        }
    }

    /// `RUSSIAN-DATACENTER-NETWORK VERIFIED`-style label. Only RU gets a
    /// spelled-out adjective because it is the benchmark the product is
    /// judged against; every other country uses its code.
    pub fn verified_label(&self) -> String {
        let prefix = match self.country.as_deref() {
            Some("RU") => "RUSSIAN-".to_string(),
            Some(code) => format!("{code}-"),
            None => String::new(),
        };
        format!("{prefix}{} VERIFIED", self.class.label())
    }
}

/// A requirement a claim must meet before it may be published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRequirement {
    pub minimum_level: EvidenceLevel,
    /// When set, the claim must have been observed from exactly this
    /// network class (and country, when given).
    pub vantage: Option<Vantage>,
}

/// One recorded observation backing a claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub subject: String,
    pub level: EvidenceLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vantage: Option<Vantage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_ref: Option<String>,
}

impl Claim {
    /// Whether this claim may be published as meeting `requirement`.
    ///
    /// Levels are compared by strength within the verified family. A
    /// vantage requirement is matched exactly: a datacenter observation
    /// never satisfies a mobile or residential requirement, and a
    /// simulated one never satisfies any real-network requirement.
    pub fn satisfies(&self, requirement: &ClaimRequirement) -> bool {
        if !self.level.is_evidence() || !requirement.minimum_level.is_evidence() {
            return false;
        }
        if strength(self.level) < strength(requirement.minimum_level) {
            return false;
        }
        match (&requirement.vantage, &self.vantage) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(required), Some(observed)) => {
                required.class == observed.class
                    && match &required.country {
                        None => true,
                        Some(country) => observed.country.as_ref() == Some(country),
                    }
            }
        }
    }

    /// The label a report may print for this claim.
    pub fn label(&self) -> String {
        match (&self.vantage, self.level) {
            (Some(vantage), EvidenceLevel::NetworkVerified) => vantage.verified_label(),
            _ => self.level.as_label().to_string(),
        }
    }
}

fn strength(level: EvidenceLevel) -> u8 {
    match level {
        EvidenceLevel::Unverified | EvidenceLevel::Blocked => 0,
        EvidenceLevel::CodeVerified => 1,
        EvidenceLevel::CiVerified => 2,
        EvidenceLevel::LocalVerified => 3,
        EvidenceLevel::VpsVerified => 4,
        EvidenceLevel::DeviceVerified => 5,
        EvidenceLevel::NetworkVerified => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_claim(class: NetworkClass, country: &str) -> Claim {
        Claim {
            subject: "reality reachable".into(),
            level: EvidenceLevel::NetworkVerified,
            vantage: Some(Vantage::new(class, Some(country))),
            commit: None,
            date: None,
            evidence_ref: None,
        }
    }

    fn requirement(class: NetworkClass, country: &str) -> ClaimRequirement {
        ClaimRequirement {
            minimum_level: EvidenceLevel::NetworkVerified,
            vantage: Some(Vantage::new(class, Some(country))),
        }
    }

    #[test]
    fn russian_datacenter_is_labelled_as_datacenter_not_mobile() {
        let claim = network_claim(NetworkClass::Datacenter, "ru");
        assert_eq!(claim.label(), "RUSSIAN-DATACENTER-NETWORK VERIFIED");
    }

    #[test]
    fn datacenter_never_satisfies_mobile_or_residential() {
        let claim = network_claim(NetworkClass::Datacenter, "RU");
        assert!(claim.satisfies(&requirement(NetworkClass::Datacenter, "RU")));
        assert!(!claim.satisfies(&requirement(NetworkClass::Mobile, "RU")));
        assert!(!claim.satisfies(&requirement(NetworkClass::Residential, "RU")));
    }

    #[test]
    fn simulated_never_satisfies_a_real_network_requirement() {
        let claim = network_claim(NetworkClass::Simulated, "RU");
        for class in [
            NetworkClass::Datacenter,
            NetworkClass::Residential,
            NetworkClass::Mobile,
        ] {
            assert!(!claim.satisfies(&requirement(class, "RU")));
        }
    }

    #[test]
    fn unverified_and_blocked_satisfy_nothing() {
        for level in [EvidenceLevel::Unverified, EvidenceLevel::Blocked] {
            let claim = Claim {
                subject: "x".into(),
                level,
                vantage: None,
                commit: None,
                date: None,
                evidence_ref: None,
            };
            assert!(!claim.satisfies(&ClaimRequirement {
                minimum_level: EvidenceLevel::CodeVerified,
                vantage: None,
            }));
        }
    }

    #[test]
    fn weaker_levels_do_not_satisfy_stronger_requirements() {
        let claim = Claim {
            subject: "awg handshake".into(),
            level: EvidenceLevel::LocalVerified,
            vantage: None,
            commit: None,
            date: None,
            evidence_ref: None,
        };
        assert!(claim.satisfies(&ClaimRequirement {
            minimum_level: EvidenceLevel::CiVerified,
            vantage: None,
        }));
        assert!(!claim.satisfies(&ClaimRequirement {
            minimum_level: EvidenceLevel::VpsVerified,
            vantage: None,
        }));
    }

    #[test]
    fn country_mismatch_fails() {
        let claim = network_claim(NetworkClass::Datacenter, "DE");
        assert!(!claim.satisfies(&requirement(NetworkClass::Datacenter, "RU")));
    }

    #[test]
    fn labels_round_trip_through_serde() {
        let json = serde_json::to_string(&EvidenceLevel::VpsVerified).unwrap();
        assert_eq!(json, "\"VPS-VERIFIED\"");
        let back: EvidenceLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, EvidenceLevel::VpsVerified);
    }
}
