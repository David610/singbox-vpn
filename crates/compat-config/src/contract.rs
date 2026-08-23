//! Bridge from this crate's server-side domain types (`CompatUser`,
//! `CompatEndpoint`) to the versioned first-party provisioning contract
//! (`provisioning_contract`).
//!
//! This module is the ONE place that decides what credential material a
//! client receives for a given user and listener. Every client-facing
//! renderer in `render.rs` — the first-party JSON contract, the
//! Hiddify-compatible share links, and the native sing-box subscription
//! JSON — starts from a [`provisioning_contract::Endpoint`] built here,
//! so there is exactly one definition of "the UUID/flow/REALITY
//! parameters/password this user gets for this endpoint" rather than one
//! per output format that can silently drift apart.

use crate::model::{CompatEndpoint, CompatUser, PublicParameters};
use crate::CompatError;
use provisioning_contract as contract;

/// The server version reported in `server.version`. Tracks this crate's
/// package version, which is the workspace release version.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether the generated VLESS endpoint requests the XTLS Vision flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VlessFlow {
    /// Production: `xtls-rprx-vision`.
    #[default]
    Vision,
    /// The `diag-vision-off` diagnostic only: no flow at all. Requires
    /// the matching per-user server-side opt-in
    /// (`CompatUser::vision_off_experiment`) — see that field's doc
    /// comment.
    VisionOff,
}

/// Build the contract representation of one listener for one user.
///
/// `tag_override` replaces the client-visible label (used by the
/// diagnostic profiles, which mark their endpoints so a tester can tell
/// which profile is actually selected in the client UI).
pub fn contract_endpoint(
    user: &CompatUser,
    endpoint: &CompatEndpoint,
    flow: VlessFlow,
    tag_override: Option<&str>,
) -> Result<contract::Endpoint, CompatError> {
    let server_name = endpoint
        .server_name
        .clone()
        .unwrap_or_else(|| endpoint.host.clone());
    let tag = tag_override
        .map(str::to_string)
        .unwrap_or_else(|| endpoint.label.clone());

    let params = match &endpoint.public_parameters {
        PublicParameters::Reality {
            public_key_hex,
            short_id,
            fingerprint,
        } => contract::TransportParams::VlessReality {
            uuid: user.vless_uuid.clone(),
            flow: match flow {
                VlessFlow::Vision => Some(contract::VLESS_FLOW_VISION.to_string()),
                VlessFlow::VisionOff => None,
            },
            reality: contract::RealityParams {
                public_key: public_key_hex.clone(),
                short_id: short_id.clone(),
                fingerprint: fingerprint.clone(),
            },
        },
        PublicParameters::Hysteria2 { obfs_password } => contract::TransportParams::Hysteria2 {
            password: user.hysteria2_password.expose().to_string(),
            obfs: obfs_password
                .as_ref()
                .map(|pw| contract::Hysteria2Obfs::salamander(pw.clone())),
        },
    };

    Ok(contract::Endpoint {
        id: endpoint.id.clone(),
        tag,
        host: endpoint.host.clone(),
        port: endpoint.port,
        server_name,
        params,
    })
}

/// Build the contract representation of every listener offered to
/// `user`, in the order supplied.
pub fn contract_endpoints(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
) -> Result<Vec<contract::Endpoint>, CompatError> {
    endpoints
        .iter()
        .map(|ep| contract_endpoint(user, ep, VlessFlow::default(), None))
        .collect()
}

/// The full versioned provisioning document for `user`.
///
/// `capabilities` are DERIVED from the endpoints this deployment
/// actually has — never a hardcoded or aspirational list. A transport
/// that is not configured produces neither a capability nor an endpoint,
/// which is how a client learns "REALITY yes / Hysteria2 no".
///
/// `experimental_capabilities` lists the diagnostic modes this user can
/// currently request. They are deliberately in their own field: they are
/// never production capabilities, never defaults, and a client must not
/// select one on its own — see `docs/PROVISIONING_CONTRACT.md`.
pub fn provisioning_document(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
) -> Result<contract::ProvisioningDocument, CompatError> {
    provisioning_document_with_mode(user, endpoints, DiagnosticMode::None)
}

/// Which diagnostic profile a provisioning document represents.
///
/// [`DiagnosticMode::None`] is production and the only thing a client
/// ever gets without asking for something else by name. The other
/// variants exist so an operator running a documented experiment can get
/// that experiment's profile through the SAME versioned contract the
/// production client uses, instead of a parallel undocumented format —
/// they are never defaults, never negotiated automatically, and always
/// reported through `experimental_capabilities`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiagnosticMode {
    #[default]
    None,
    /// `diag-tcp-only`: every UDP-carrying option removed, so the
    /// profile cannot fall back to one — see
    /// `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`.
    TcpOnly,
    /// `diag-vision-off`: the VLESS+REALITY endpoint carries no flow.
    /// Requires `CompatUser::vision_off_experiment`; more fingerprintable
    /// than production — see `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md`.
    VisionOff,
}

impl DiagnosticMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "tcp-only" | "diag-tcp-only" => Some(Self::TcpOnly),
            "vision-off" | "diag-vision-off" => Some(Self::VisionOff),
            _ => None,
        }
    }
}

/// [`provisioning_document`] for a specific diagnostic profile. The
/// production path is `DiagnosticMode::None`.
pub fn provisioning_document_with_mode(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
    mode: DiagnosticMode,
) -> Result<contract::ProvisioningDocument, CompatError> {
    let mut contract_endpoints = Vec::with_capacity(endpoints.len());
    for ep in endpoints {
        if mode == DiagnosticMode::TcpOnly
            && matches!(ep.transport, crate::model::CompatTransport::Hysteria2)
        {
            continue;
        }
        let vision_off = mode == DiagnosticMode::VisionOff
            && matches!(ep.transport, crate::model::CompatTransport::VlessReality);
        contract_endpoints.push(contract_endpoint(
            user,
            ep,
            if vision_off {
                VlessFlow::VisionOff
            } else {
                VlessFlow::Vision
            },
            None,
        )?);
    }

    let mut capabilities: Vec<contract::Capability> = Vec::new();
    for ep in &contract_endpoints {
        let cap = contract::Capability::for_transport(&ep.transport());
        if !capabilities.contains(&cap) {
            capabilities.push(cap);
        }
    }

    let mut experimental = Vec::new();
    if endpoints
        .iter()
        .any(|ep| matches!(ep.transport, crate::model::CompatTransport::VlessReality))
    {
        // `diag-tcp-only` needs a VLESS+REALITY endpoint to remain
        // usable after Hysteria2 is dropped; advertising it for a
        // Hysteria2-only deployment would advertise an empty profile.
        experimental.push(contract::ExperimentalCapability::tcp_only());
        if user.vision_off_experiment {
            // Only advertised while the matching per-user server-side
            // opt-in is on: without it sing-box's VLESS server rejects
            // the flow mismatch, so advertising it would be a promise
            // the server cannot keep.
            experimental.push(contract::ExperimentalCapability::vision_off());
        }
    }

    let doc = contract::ProvisioningDocument::new(
        contract::ServerInfo::current(SERVER_VERSION),
        capabilities,
        contract_endpoints,
    )
    .with_experimental_capabilities(experimental);
    doc.validate()?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CompatTransport;
    use crate::render::standard_endpoints;
    use crate::secret::SecretString;

    fn user() -> CompatUser {
        CompatUser {
            id: "u1".into(),
            name: "test".into(),
            enabled: true,
            vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
            hysteria2_password: SecretString::new("hy2pass"),
            subscription_token_hash_hex: "hash".into(),
            created_at: 0,
            expires_at: None,
            vision_off_experiment: false,
        }
    }

    fn endpoints() -> Vec<CompatEndpoint> {
        standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake",
            "0a1b2c3d",
            "www.example-decoy.com",
            None,
        )
    }

    #[test]
    fn document_advertises_exactly_the_configured_transports() {
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        assert_eq!(
            doc.capabilities,
            vec![
                contract::Capability::VlessReality,
                contract::Capability::Hysteria2
            ]
        );
        assert!(doc.supports(&contract::Capability::Hysteria2));
    }

    #[test]
    fn a_transport_with_no_endpoint_is_not_advertised() {
        let reality_only: Vec<CompatEndpoint> = endpoints()
            .into_iter()
            .filter(|e| e.transport == CompatTransport::VlessReality)
            .collect();
        let doc = provisioning_document(&user(), &reality_only).unwrap();
        assert!(doc.supports(&contract::Capability::VlessReality));
        assert!(
            !doc.supports(&contract::Capability::Hysteria2),
            "capabilities must reflect real configuration, not intent"
        );
        assert_eq!(doc.endpoints.len(), 1);
    }

    #[test]
    fn vision_off_is_only_advertised_when_the_per_user_opt_in_is_on() {
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        assert_eq!(
            doc.experimental_capabilities,
            vec![contract::ExperimentalCapability::tcp_only()],
            "vision-off must not be advertised without its server-side opt-in"
        );

        let mut u = user();
        u.vision_off_experiment = true;
        let doc = provisioning_document(&u, &endpoints()).unwrap();
        assert!(doc
            .experimental_capabilities
            .contains(&contract::ExperimentalCapability::vision_off()));
    }

    #[test]
    fn experimental_capabilities_never_leak_into_production_capabilities() {
        let mut u = user();
        u.vision_off_experiment = true;
        let doc = provisioning_document(&u, &endpoints()).unwrap();
        for cap in &doc.capabilities {
            assert!(
                !cap.as_str().starts_with("diag-"),
                "diagnostic capability {cap:?} leaked into production negotiation"
            );
        }
    }

    #[test]
    fn obfs_password_is_carried_only_when_configured() {
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        let hy2 = doc
            .endpoints
            .iter()
            .find(|e| e.transport() == contract::Transport::Hysteria2)
            .unwrap();
        assert!(matches!(
            &hy2.params,
            contract::TransportParams::Hysteria2 { obfs: None, .. }
        ));

        let with_obfs = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake",
            "0a1b2c3d",
            "www.example-decoy.com",
            Some("fake-obfs-password"),
        );
        let doc = provisioning_document(&user(), &with_obfs).unwrap();
        let hy2 = doc
            .endpoints
            .iter()
            .find(|e| e.transport() == contract::Transport::Hysteria2)
            .unwrap();
        let contract::TransportParams::Hysteria2 { obfs, .. } = &hy2.params else {
            panic!("expected hysteria2 params");
        };
        let obfs = obfs.as_ref().expect("obfs present");
        assert_eq!(obfs.obfs_type, "salamander");
        assert_eq!(obfs.password, "fake-obfs-password");
    }

    #[test]
    fn generated_document_never_carries_server_private_material() {
        // `provisioning_document` validates, and validation includes the
        // forbidden-content audit — so this passing is the proof.
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        let json = doc.to_json().unwrap().to_ascii_lowercase();
        for forbidden in [
            "private_key",
            "-----begin",
            ".pem",
            "/etc/",
            "insecure",
            "\"dns\"",
            "\"mtu\"",
            "\"tun\"",
            "auto_route",
            "kill_switch",
        ] {
            assert!(!json.contains(forbidden), "{forbidden} present in {json}");
        }
    }

    #[test]
    fn vision_off_endpoint_has_no_flow_and_production_endpoint_has_vision() {
        let eps = endpoints();
        let reality = &eps[0];
        let production = contract_endpoint(&user(), reality, VlessFlow::Vision, None).unwrap();
        let contract::TransportParams::VlessReality { flow, .. } = &production.params else {
            panic!("expected vless params");
        };
        assert_eq!(flow.as_deref(), Some("xtls-rprx-vision"));

        let diagnostic = contract_endpoint(
            &user(),
            reality,
            VlessFlow::VisionOff,
            Some("Reality (diag)"),
        )
        .unwrap();
        let contract::TransportParams::VlessReality { flow, .. } = &diagnostic.params else {
            panic!("expected vless params");
        };
        assert_eq!(flow, &None);
        assert_eq!(diagnostic.tag, "Reality (diag)");
    }
}
