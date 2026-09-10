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

use crate::model::{CompatEndpoint, CompatUser, EndpointOrigin, PeerCredential, PublicParameters};
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
    contract_endpoint_opt(user, endpoint, flow, tag_override)?.ok_or_else(|| {
        CompatError::Parse(format!(
            "user {} has no credential for peer endpoint {:?}; this server cannot mint one because it does not control that server (see `vpn-admin user peer set`)",
            user.id, endpoint.id
        ))
    })
}

/// As [`contract_endpoint`], but returns `Ok(None)` for a PEER endpoint
/// this user has no credential for.
///
/// That case is not an error at the document level: a user simply has not
/// been given access to that peer yet, and the correct response is to omit
/// the endpoint entirely. Serving it with a placeholder or with the user's
/// LOCAL credential would hand the client something that cannot possibly
/// authenticate, and the client has no way to tell that from a network
/// failure.
pub fn contract_endpoint_opt(
    user: &CompatUser,
    endpoint: &CompatEndpoint,
    flow: VlessFlow,
    tag_override: Option<&str>,
) -> Result<Option<contract::Endpoint>, CompatError> {
    let server_name = endpoint
        .server_name
        .clone()
        .unwrap_or_else(|| endpoint.host.clone());
    let tag = tag_override
        .map(str::to_string)
        .unwrap_or_else(|| endpoint.label.clone());

    // Resolve the credential BEFORE shaping anything, so "which secret
    // does this user present to this endpoint" stays a single decision
    // with a single answer.
    let credential = match endpoint.origin {
        EndpointOrigin::Local => None,
        EndpointOrigin::Peer => match user.peer_credential(&endpoint.id) {
            None => return Ok(None),
            Some(c) => {
                if c.transport() != endpoint.transport {
                    return Err(CompatError::Parse(format!(
                        "user {}: peer credential for {:?} is a {} credential but that endpoint is {}; a credential is never coerced across transports",
                        user.id,
                        endpoint.id,
                        c.transport().as_str(),
                        endpoint.transport.as_str()
                    )));
                }
                Some(c)
            }
        },
    };

    let params = match &endpoint.public_parameters {
        PublicParameters::Reality {
            public_key_hex,
            short_id,
            fingerprint,
        } => {
            let uuid = match credential {
                None => user.vless_uuid.clone(),
                Some(PeerCredential::VlessReality { uuid }) => uuid.clone(),
                Some(other) => {
                    return Err(CompatError::Parse(format!(
                        "user {}: endpoint {:?} needs a vless-reality credential, got {}",
                        user.id,
                        endpoint.id,
                        other.transport().as_str()
                    )))
                }
            };
            contract::TransportParams::VlessReality {
                uuid,
                flow: match flow {
                    VlessFlow::Vision => Some(contract::VLESS_FLOW_VISION.to_string()),
                    VlessFlow::VisionOff => None,
                },
                reality: contract::RealityParams {
                    public_key: public_key_hex.clone(),
                    short_id: short_id.clone(),
                    fingerprint: fingerprint.clone(),
                },
            }
        }
        PublicParameters::Hysteria2 { obfs_password } => {
            let password = match credential {
                None => user.hysteria2_password.expose().to_string(),
                Some(PeerCredential::Hysteria2 { password }) => password.expose().to_string(),
                Some(other) => {
                    return Err(CompatError::Parse(format!(
                        "user {}: endpoint {:?} needs a hysteria2 credential, got {}",
                        user.id,
                        endpoint.id,
                        other.transport().as_str()
                    )))
                }
            };
            contract::TransportParams::Hysteria2 {
                password,
                obfs: obfs_password
                    .as_ref()
                    .map(|pw| contract::Hysteria2Obfs::salamander(pw.clone())),
            }
        }
    };

    Ok(Some(
        contract::Endpoint::new(
            endpoint.id.clone(),
            tag,
            endpoint.host.clone(),
            endpoint.port,
            server_name,
            params,
        )
        .with_metadata(
            endpoint.failure_domain.clone(),
            endpoint.region.clone(),
            endpoint.provider.clone(),
            endpoint.asn.clone(),
            endpoint.path.as_deref().map(contract::PathType::from_wire),
        ),
    ))
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
    provisioning_document_with_mode_and_access_paths(user, endpoints, mode, &[])
}

/// As `provisioning_document_with_mode`, with additive non-secret first-hop
/// metadata. The endpoint/config pair is still rendered atomically from the
/// same endpoint set; access paths only describe how those concrete routes are
/// reached and never carry credentials.
pub fn provisioning_document_with_mode_and_access_paths(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
    mode: DiagnosticMode,
    access_paths: &[contract::AccessPath],
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
        // `_opt`: a peer endpoint this user has no credential for is
        // omitted, not rendered broken. See `contract_endpoint_opt`.
        if let Some(built) = contract_endpoint_opt(
            user,
            ep,
            if vision_off {
                VlessFlow::VisionOff
            } else {
                VlessFlow::Vision
            },
            None,
        )? {
            contract_endpoints.push(built);
        }
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

    // The embedded Core config is rendered from THIS list, not from a
    // second derivation of it, so the catalog and the config a client
    // hands to Core cannot describe different things. `validate()` then
    // cross-checks them anyway — a guarantee worth having twice, since it
    // is the whole basis for a client trusting one fetch.
    let singbox_config = crate::render::render_singbox_config_from_contract(
        &contract_endpoints,
        crate::render::SelectionProfile::default(),
        match mode {
            DiagnosticMode::TcpOnly => crate::render::CompatibilityMode::TcpOnly,
            DiagnosticMode::VisionOff | DiagnosticMode::None => {
                crate::render::CompatibilityMode::Normal
            }
        },
    )?;

    let doc = contract::ProvisioningDocument::new(
        contract::ServerInfo::current(SERVER_VERSION),
        capabilities,
        contract_endpoints,
    )
    .with_experimental_capabilities(experimental)
    .with_access_paths(access_paths.to_vec())
    .with_singbox_config(singbox_config);
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
            peer_credentials: Default::default(),
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
        //
        // The scan runs over the ENVELOPE, with `singbox_config` removed.
        // That is not a weakening: the embedded Core config legitimately
        // contains `"insecure": false` — an assertion that certificate
        // verification is ON — which a substring scan for the word
        // "insecure" cannot tell from the opt-out it exists to forbid.
        // The embedded half is audited structurally instead (see
        // `embedded_core_config_is_audited_structurally_not_by_substring`
        // below and `ProvisioningDocument::audit_embedded_config`), which
        // catches strictly more, because it can judge position and value.
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        let mut envelope: serde_json::Value =
            serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        envelope.as_object_mut().unwrap().remove("singbox_config");
        let json = envelope.to_string().to_ascii_lowercase();
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
    fn embedded_core_config_is_audited_structurally_not_by_substring() {
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        let config = doc.singbox_config.as_ref().expect("config is embedded");

        // The value a substring scan could not have judged.
        let hy2 = config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["type"] == "hysteria2")
            .expect("hysteria2 outbound");
        assert_eq!(
            hy2["tls"]["insecure"],
            serde_json::json!(false),
            "certificate verification must be asserted ON, and this is exactly the value the \
             envelope's substring audit had to stop judging"
        );

        // Client-owned policy is absent by allowlist, not by luck.
        let root = config.as_object().unwrap();
        for key in root.keys() {
            assert!(
                matches!(key.as_str(), "outbounds" | "route"),
                "unexpected top-level key {key:?} in the embedded config"
            );
        }
        assert!(!doc.to_json().unwrap().contains("private_key"));
    }

    #[test]
    fn a_zero_peer_document_embeds_a_config_whose_selector_matches_its_catalog() {
        // The invariant a client relies on when it trusts ONE fetch.
        let doc = provisioning_document(&user(), &endpoints()).unwrap();
        let config = doc.singbox_config.as_ref().unwrap();
        let selector = config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == contract::SELECTOR_GROUP_TAG)
            .expect("selector group");
        let mut options: Vec<String> = selector["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let mut expected: Vec<String> = doc.endpoints.iter().map(|e| e.tag.clone()).collect();
        expected.push(contract::AUTO_GROUP_TAG.to_string());
        options.sort();
        expected.sort();
        assert_eq!(options, expected);
        assert_eq!(config["route"]["final"], contract::SELECTOR_GROUP_TAG);
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
