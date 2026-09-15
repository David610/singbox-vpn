//! Typed, evidence-based failure classification.
//!
//! The classifier maps *mechanical* observations to a class and returns
//! the evidence it relied on. It never guesses: ambiguous input becomes
//! [`FailureClass::Unknown`], and [`FailureClass::CensorshipSuspected`]
//! requires independent controls that rule out the ordinary explanations
//! (the network being down, the endpoint being down) — a failed
//! connection alone is never evidence of censorship.

use crate::health::HealthLayer;
use crate::transport::L4Protocol;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FailureClass {
    LocalTunFailure,
    DnsFailure,
    TcpUnreachable,
    UdpUnreachable,
    HandshakeFailure,
    AuthFailure,
    PostHandshakeStall,
    PmtuOrMtuFailure,
    ServerOverloaded,
    EndpointDown,
    NetworkTransition,
    RouteLeakDetected,
    Ipv6LeakDetected,
    CensorshipSuspected,
    Unknown,
}

impl FailureClass {
    /// Whether the failure says something about the *route or endpoint*
    /// (and should therefore affect its score) rather than about the
    /// local device or network.
    pub fn implicates_route(self) -> bool {
        !matches!(
            self,
            FailureClass::LocalTunFailure
                | FailureClass::NetworkTransition
                | FailureClass::DnsFailure
                | FailureClass::RouteLeakDetected
                | FailureClass::Ipv6LeakDetected
        )
    }

    /// Whether the failure identifies the node itself, so other routes
    /// through the same node may share the penalty.
    pub fn implicates_node(self) -> bool {
        matches!(
            self,
            FailureClass::EndpointDown | FailureClass::ServerOverloaded
        )
    }

    /// Whether the failure is a UDP-path signal that should penalise UDP
    /// transports on this network.
    pub fn implicates_udp_path(self) -> bool {
        matches!(self, FailureClass::UdpUnreachable)
    }

    /// Leak detections are safety failures: the session must be torn down
    /// or blocked, never silently kept.
    pub fn is_safety_violation(self) -> bool {
        matches!(
            self,
            FailureClass::RouteLeakDetected | FailureClass::Ipv6LeakDetected
        )
    }
}

/// Low-level error kinds an engine or probe can report. These are
/// mechanical symptoms, not diagnoses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    TunCreateFailed,
    RouteInstallFailed,
    DnsTimeout,
    DnsNxDomain,
    ConnectRefused,
    ConnectTimeout,
    ConnectReset,
    HostUnreachable,
    /// A UDP handshake got no response at all within the deadline.
    NoHandshakeResponse,
    TlsAlert,
    HandshakeTimeout,
    HandshakeReset,
    /// The peer explicitly rejected the credential.
    AuthRejected,
    /// Tunnel came up but no payload flowed within the deadline.
    StallAfterHandshake,
    /// Small packets pass while large ones are lost.
    LargePacketLoss,
    ServerBusy,
    NetworkChanged,
    Ipv4LeakObserved,
    Ipv6LeakObserved,
    Other,
}

/// Everything the classifier may use. `None` means "not measured", which
/// is different from `Some(false)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureInput {
    pub failed_layer: HealthLayer,
    pub error: ErrorKind,
    pub transport_l4: L4Protocol,
    /// A generic control (e.g. HTTPS to a well-known non-VPN host) from
    /// the same local network succeeded.
    #[serde(default)]
    pub local_network_control_ok: Option<bool>,
    /// The same endpoint completed the same handshake from an independent
    /// vantage point within the evidence window.
    #[serde(default)]
    pub endpoint_ok_from_other_vantage: Option<bool>,
    /// A plain TLS/TCP control to a *different* host on the same port and
    /// protocol succeeded from this network.
    #[serde(default)]
    pub same_port_control_ok: Option<bool>,
    /// Other transports on the same node failed in the same window.
    #[serde(default)]
    pub other_transports_same_node_failed: Option<bool>,
    /// How many times this exact (route, error) repeated in the window.
    #[serde(default)]
    pub repetitions: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceItem {
    ErrorKind,
    LocalNetworkControlOk,
    LocalNetworkControlFailed,
    EndpointOkFromOtherVantage,
    EndpointFailedFromOtherVantage,
    SamePortControlOk,
    OtherTransportsSameNodeFailed,
    OtherTransportsSameNodeOk,
    Repeated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    pub class: FailureClass,
    pub evidence: Vec<EvidenceItem>,
}

/// Thresholds for the censorship rule. Defaults are strict on purpose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifierPolicy {
    pub censorship_min_repetitions: u32,
}

impl Default for ClassifierPolicy {
    fn default() -> Self {
        ClassifierPolicy {
            censorship_min_repetitions: 3,
        }
    }
}

pub fn classify(input: &FailureInput, policy: &ClassifierPolicy) -> Classification {
    use ErrorKind as E;
    use EvidenceItem as Ev;
    use FailureClass as C;

    let mut evidence = vec![Ev::ErrorKind];
    let done = |class, evidence| Classification { class, evidence };

    // Local and safety conditions are decided by the symptom alone.
    match input.error {
        E::TunCreateFailed | E::RouteInstallFailed => return done(C::LocalTunFailure, evidence),
        E::NetworkChanged => return done(C::NetworkTransition, evidence),
        E::Ipv4LeakObserved => return done(C::RouteLeakDetected, evidence),
        E::Ipv6LeakObserved => return done(C::Ipv6LeakDetected, evidence),
        E::AuthRejected => return done(C::AuthFailure, evidence),
        E::ServerBusy => return done(C::ServerOverloaded, evidence),
        E::DnsTimeout | E::DnsNxDomain => return done(C::DnsFailure, evidence),
        _ => {}
    }

    // If the local network itself is down, nothing below can be
    // attributed to the route.
    if input.local_network_control_ok == Some(false) {
        evidence.push(Ev::LocalNetworkControlFailed);
        return done(C::Unknown, evidence);
    }

    // Endpoint down: every transport on the node fails AND an
    // independent vantage point fails too (or the peer refuses outright).
    let node_wide = input.other_transports_same_node_failed == Some(true);
    let other_vantage_failed = input.endpoint_ok_from_other_vantage == Some(false);
    if node_wide && other_vantage_failed {
        evidence.push(Ev::OtherTransportsSameNodeFailed);
        evidence.push(Ev::EndpointFailedFromOtherVantage);
        return done(C::EndpointDown, evidence);
    }

    if censorship_evidence(input, policy) {
        evidence.extend([
            Ev::LocalNetworkControlOk,
            Ev::EndpointOkFromOtherVantage,
            Ev::SamePortControlOk,
            Ev::Repeated,
        ]);
        if input.other_transports_same_node_failed == Some(false) {
            evidence.push(Ev::OtherTransportsSameNodeOk);
        }
        return done(C::CensorshipSuspected, evidence);
    }

    let class = match (input.error, input.transport_l4) {
        (E::LargePacketLoss, _) => C::PmtuOrMtuFailure,
        (E::StallAfterHandshake, _) => C::PostHandshakeStall,
        (
            E::ConnectRefused | E::ConnectTimeout | E::HostUnreachable | E::ConnectReset,
            L4Protocol::Tcp,
        ) if input.failed_layer == HealthLayer::Reachability => C::TcpUnreachable,
        (E::NoHandshakeResponse, L4Protocol::Udp) => C::UdpUnreachable,
        (E::TlsAlert | E::HandshakeTimeout | E::HandshakeReset | E::ConnectReset, _)
            if input.failed_layer == HealthLayer::Handshake =>
        {
            C::HandshakeFailure
        }
        (E::HandshakeTimeout, L4Protocol::Udp) => C::UdpUnreachable,
        _ => C::Unknown,
    };
    done(class, evidence)
}

fn censorship_evidence(input: &FailureInput, policy: &ClassifierPolicy) -> bool {
    let pattern = matches!(
        input.error,
        ErrorKind::HandshakeReset
            | ErrorKind::HandshakeTimeout
            | ErrorKind::ConnectReset
            | ErrorKind::NoHandshakeResponse
            | ErrorKind::StallAfterHandshake
    );
    pattern
        && input.local_network_control_ok == Some(true)
        && input.endpoint_ok_from_other_vantage == Some(true)
        && input.same_port_control_ok == Some(true)
        && input.repetitions >= policy.censorship_min_repetitions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(layer: HealthLayer, error: ErrorKind, l4: L4Protocol) -> FailureInput {
        FailureInput {
            failed_layer: layer,
            error,
            transport_l4: l4,
            local_network_control_ok: None,
            endpoint_ok_from_other_vantage: None,
            same_port_control_ok: None,
            other_transports_same_node_failed: None,
            repetitions: 1,
        }
    }

    fn with_all_controls(mut i: FailureInput) -> FailureInput {
        i.local_network_control_ok = Some(true);
        i.endpoint_ok_from_other_vantage = Some(true);
        i.same_port_control_ok = Some(true);
        i.repetitions = 5;
        i
    }

    #[test]
    fn a_bare_failed_connection_is_never_censorship() {
        let p = ClassifierPolicy::default();
        for error in [
            ErrorKind::ConnectTimeout,
            ErrorKind::ConnectReset,
            ErrorKind::HandshakeReset,
            ErrorKind::NoHandshakeResponse,
            ErrorKind::StallAfterHandshake,
        ] {
            for l4 in [L4Protocol::Tcp, L4Protocol::Udp] {
                let c = classify(&input(HealthLayer::Handshake, error, l4), &p);
                assert_ne!(
                    c.class,
                    FailureClass::CensorshipSuspected,
                    "{error:?}/{l4:?}"
                );
            }
        }
    }

    #[test]
    fn censorship_requires_every_control_and_repetition() {
        let p = ClassifierPolicy::default();
        let full = with_all_controls(input(
            HealthLayer::Handshake,
            ErrorKind::HandshakeReset,
            L4Protocol::Tcp,
        ));
        assert_eq!(classify(&full, &p).class, FailureClass::CensorshipSuspected);

        let mut missing_vantage = full.clone();
        missing_vantage.endpoint_ok_from_other_vantage = None;
        assert_ne!(
            classify(&missing_vantage, &p).class,
            FailureClass::CensorshipSuspected
        );

        let mut missing_same_port = full.clone();
        missing_same_port.same_port_control_ok = Some(false);
        assert_ne!(
            classify(&missing_same_port, &p).class,
            FailureClass::CensorshipSuspected
        );

        let mut too_few = full.clone();
        too_few.repetitions = 2;
        assert_ne!(
            classify(&too_few, &p).class,
            FailureClass::CensorshipSuspected
        );

        let mut network_down = full;
        network_down.local_network_control_ok = Some(false);
        assert_eq!(classify(&network_down, &p).class, FailureClass::Unknown);
    }

    #[test]
    fn auth_errors_are_auth_even_with_controls() {
        let p = ClassifierPolicy::default();
        let i = with_all_controls(input(
            HealthLayer::Handshake,
            ErrorKind::AuthRejected,
            L4Protocol::Tcp,
        ));
        assert_eq!(classify(&i, &p).class, FailureClass::AuthFailure);
    }

    #[test]
    fn endpoint_down_needs_node_wide_and_other_vantage_failure() {
        let p = ClassifierPolicy::default();
        let mut i = input(
            HealthLayer::Reachability,
            ErrorKind::ConnectTimeout,
            L4Protocol::Tcp,
        );
        i.other_transports_same_node_failed = Some(true);
        assert_eq!(classify(&i, &p).class, FailureClass::TcpUnreachable);
        i.endpoint_ok_from_other_vantage = Some(false);
        assert_eq!(classify(&i, &p).class, FailureClass::EndpointDown);
    }

    #[test]
    fn mechanical_classes() {
        let p = ClassifierPolicy::default();
        let cases = [
            (
                HealthLayer::Reachability,
                ErrorKind::ConnectRefused,
                L4Protocol::Tcp,
                FailureClass::TcpUnreachable,
            ),
            (
                HealthLayer::Handshake,
                ErrorKind::NoHandshakeResponse,
                L4Protocol::Udp,
                FailureClass::UdpUnreachable,
            ),
            (
                HealthLayer::Handshake,
                ErrorKind::TlsAlert,
                L4Protocol::Tcp,
                FailureClass::HandshakeFailure,
            ),
            (
                HealthLayer::Internet,
                ErrorKind::StallAfterHandshake,
                L4Protocol::Udp,
                FailureClass::PostHandshakeStall,
            ),
            (
                HealthLayer::Transfer,
                ErrorKind::LargePacketLoss,
                L4Protocol::Udp,
                FailureClass::PmtuOrMtuFailure,
            ),
            (
                HealthLayer::Tunnel,
                ErrorKind::TunCreateFailed,
                L4Protocol::Udp,
                FailureClass::LocalTunFailure,
            ),
            (
                HealthLayer::Internet,
                ErrorKind::DnsTimeout,
                L4Protocol::Tcp,
                FailureClass::DnsFailure,
            ),
            (
                HealthLayer::Internet,
                ErrorKind::NetworkChanged,
                L4Protocol::Tcp,
                FailureClass::NetworkTransition,
            ),
            (
                HealthLayer::Internet,
                ErrorKind::Ipv6LeakObserved,
                L4Protocol::Tcp,
                FailureClass::Ipv6LeakDetected,
            ),
            (
                HealthLayer::Internet,
                ErrorKind::Other,
                L4Protocol::Tcp,
                FailureClass::Unknown,
            ),
            (
                HealthLayer::Tunnel,
                ErrorKind::ConnectReset,
                L4Protocol::Tcp,
                FailureClass::Unknown,
            ),
        ];
        for (layer, error, l4, expected) in cases {
            assert_eq!(
                classify(&input(layer, error, l4), &p).class,
                expected,
                "{layer:?}/{error:?}"
            );
        }
    }

    #[test]
    fn local_failures_do_not_implicate_routes() {
        assert!(!FailureClass::LocalTunFailure.implicates_route());
        assert!(!FailureClass::NetworkTransition.implicates_route());
        assert!(FailureClass::HandshakeFailure.implicates_route());
        assert!(FailureClass::EndpointDown.implicates_node());
        assert!(!FailureClass::HandshakeFailure.implicates_node());
        assert!(FailureClass::Ipv6LeakDetected.is_safety_violation());
    }

    #[test]
    fn serde_names_are_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&FailureClass::PmtuOrMtuFailure).unwrap(),
            "\"PMTU_OR_MTU_FAILURE\""
        );
    }

    proptest::proptest! {
        #[test]
        fn censorship_is_never_returned_without_all_controls(
            layer_idx in 0usize..6,
            err_idx in 0usize..20,
            udp in proptest::bool::ANY,
            local in proptest::option::of(proptest::bool::ANY),
            vantage in proptest::option::of(proptest::bool::ANY),
            same_port in proptest::option::of(proptest::bool::ANY),
            node in proptest::option::of(proptest::bool::ANY),
            reps in 0u32..10,
        ) {
            let errors = [
                ErrorKind::TunCreateFailed, ErrorKind::RouteInstallFailed, ErrorKind::DnsTimeout,
                ErrorKind::DnsNxDomain, ErrorKind::ConnectRefused, ErrorKind::ConnectTimeout,
                ErrorKind::ConnectReset, ErrorKind::HostUnreachable, ErrorKind::NoHandshakeResponse,
                ErrorKind::TlsAlert, ErrorKind::HandshakeTimeout, ErrorKind::HandshakeReset,
                ErrorKind::AuthRejected, ErrorKind::StallAfterHandshake, ErrorKind::LargePacketLoss,
                ErrorKind::ServerBusy, ErrorKind::NetworkChanged, ErrorKind::Ipv4LeakObserved,
                ErrorKind::Ipv6LeakObserved, ErrorKind::Other,
            ];
            let i = FailureInput {
                failed_layer: HealthLayer::ALL[layer_idx],
                error: errors[err_idx],
                transport_l4: if udp { L4Protocol::Udp } else { L4Protocol::Tcp },
                local_network_control_ok: local,
                endpoint_ok_from_other_vantage: vantage,
                same_port_control_ok: same_port,
                other_transports_same_node_failed: node,
                repetitions: reps,
            };
            let c = classify(&i, &ClassifierPolicy::default());
            if c.class == FailureClass::CensorshipSuspected {
                proptest::prop_assert_eq!(local, Some(true));
                proptest::prop_assert_eq!(vantage, Some(true));
                proptest::prop_assert_eq!(same_port, Some(true));
                proptest::prop_assert!(reps >= 3);
            }
        }
    }
}
