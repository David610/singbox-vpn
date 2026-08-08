//! Failure taxonomy and observation types shared by the failure classifier,
//! transports, and the scoring/policy engine. See
//! `docs/FAILURE_CLASSIFICATION.md` for the rationale behind each category.

use common::{EndpointId, TransportId, UnixSeconds};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FailureCategory {
    DnsFailure,
    TcpTimeout,
    TcpReset,
    TlsFailure,
    QuicFailure,
    UdpUnavailable,
    HandshakeTimeout,
    StallAfterConnect,
    EndpointUnreachable,
    ProviderDegradation,
    TransportDegradation,
    GeneralRouteFailure,
    PossibleShutdown,
    LocalNetworkFailure,
}

impl FailureCategory {
    /// Whether this failure category should ever mutate remote
    /// transport/endpoint scoring. `LocalNetworkFailure` must not — it says
    /// nothing about the network path beyond the local host (invariant #4
    /// in FAILURE_CLASSIFICATION.md).
    pub fn is_remote_signal(&self) -> bool {
        !matches!(self, FailureCategory::LocalNetworkFailure)
    }

    /// Whether this category is transport-attributable (should lower the
    /// *transport's* score) as opposed to endpoint-attributable.
    pub fn is_transport_attributable(&self) -> bool {
        matches!(
            self,
            FailureCategory::TcpReset
                | FailureCategory::TlsFailure
                | FailureCategory::QuicFailure
                | FailureCategory::UdpUnavailable
                | FailureCategory::TransportDegradation
        )
    }

    /// Whether this category is endpoint-attributable (should lower the
    /// *endpoint's* score but leave the transport's score untouched).
    pub fn is_endpoint_attributable(&self) -> bool {
        matches!(
            self,
            FailureCategory::EndpointUnreachable
                | FailureCategory::TcpTimeout
                | FailureCategory::HandshakeTimeout
                | FailureCategory::StallAfterConnect
        )
    }
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub transport: TransportId,
    pub endpoint: EndpointId,
    pub at: UnixSeconds,
    pub outcome: Outcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure(FailureCategory),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_network_failure_is_never_a_remote_signal() {
        assert!(!FailureCategory::LocalNetworkFailure.is_remote_signal());
        for cat in [
            FailureCategory::DnsFailure,
            FailureCategory::TcpTimeout,
            FailureCategory::TcpReset,
            FailureCategory::TlsFailure,
            FailureCategory::QuicFailure,
            FailureCategory::UdpUnavailable,
            FailureCategory::HandshakeTimeout,
            FailureCategory::StallAfterConnect,
            FailureCategory::EndpointUnreachable,
            FailureCategory::ProviderDegradation,
            FailureCategory::TransportDegradation,
            FailureCategory::GeneralRouteFailure,
            FailureCategory::PossibleShutdown,
        ] {
            assert!(cat.is_remote_signal());
        }
    }

    #[test]
    fn transport_and_endpoint_attribution_are_disjoint() {
        let all = [
            FailureCategory::DnsFailure,
            FailureCategory::TcpTimeout,
            FailureCategory::TcpReset,
            FailureCategory::TlsFailure,
            FailureCategory::QuicFailure,
            FailureCategory::UdpUnavailable,
            FailureCategory::HandshakeTimeout,
            FailureCategory::StallAfterConnect,
            FailureCategory::EndpointUnreachable,
            FailureCategory::ProviderDegradation,
            FailureCategory::TransportDegradation,
            FailureCategory::GeneralRouteFailure,
            FailureCategory::PossibleShutdown,
            FailureCategory::LocalNetworkFailure,
        ];
        for cat in all {
            assert!(!(cat.is_transport_attributable() && cat.is_endpoint_attributable()));
        }
    }
}
