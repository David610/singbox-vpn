//! Assembly of the `schema_version = 2` route document from deployment
//! state, and its conversion into the platform route model.
//!
//! Credentials are resolved by the same function v1 uses
//! ([`crate::contract::contract_endpoint_opt`]), so both versions always
//! agree about which secret a user presents to which listener. AmneziaWG
//! credentials come from [`CompatUser::amneziawg`] via the AWG provider.
//!
//! Mapping from today's declarative deployment:
//!
//! * this node → one `node` (`node_id`, `role`, failure domain derived from
//!   `public_host`, which is exactly the shared-fate relationship its own
//!   listeners have);
//! * each distinct peer `failure_domain` → one `node`;
//! * local listeners and peers → `endpoints`; a relay alias that points at
//!   an exit through `credential_ref` reuses that exit's endpoint instead
//!   of duplicating the listener;
//! * every selectable listener → a direct `route`; every relay path → a
//!   two-hop `route` whose first hop is the relay's own REALITY listener.
//!
//! Only routes the user holds every credential for are emitted. Nodes and
//! endpoints nothing references are dropped, so a document never carries
//! a credential no route uses.

use crate::amneziawg::{AmneziaWgProvider, AwgNodeConfig};
use crate::contract::{contract_endpoint_opt, VlessFlow, SERVER_VERSION};
use crate::deployment::{
    default_node_id_for_host, DeploymentConfig, NodeRole, LOCAL_AMNEZIAWG_ENDPOINT_ID,
    LOCAL_HYSTERIA2_ENDPOINT_ID, LOCAL_REALITY_ENDPOINT_ID, RELAY_ACCESS_PATH_KIND,
};
use crate::model::{CompatEndpoint, CompatUser, EndpointOrigin};
use crate::CompatError;
use platform_core::route as pc;
use platform_core::transport::{TransportKind, TransportProvider};
use provisioning_contract as contract;
use provisioning_contract::v2;
use std::collections::{BTreeMap, BTreeSet};

/// Everything the v2 renderer needs, captured once at service start (like
/// the v1 endpoint set) so a request never reads key files.
#[derive(Clone, Debug)]
pub struct RouteContext {
    pub node_id: String,
    pub role: NodeRole,
    pub public_host: String,
    /// The canonical served endpoint set (`DeploymentConfig::served_endpoints`).
    pub endpoints: Vec<CompatEndpoint>,
    /// `(access path id, first-hop endpoint id)` for relay paths.
    pub relay_paths: Vec<(String, String)>,
    /// Public-only AmneziaWG node configuration, when enabled.
    pub amneziawg: Option<AwgNodeConfig>,
}

impl RouteContext {
    pub fn from_deployment(
        cfg: &DeploymentConfig,
        endpoints: Vec<CompatEndpoint>,
        amneziawg: Option<AwgNodeConfig>,
    ) -> Self {
        RouteContext {
            node_id: if cfg.node_id.is_empty() {
                default_node_id_for_host(&cfg.public_host)
            } else {
                cfg.node_id.clone()
            },
            role: cfg.role,
            public_host: cfg.public_host.clone(),
            endpoints,
            relay_paths: cfg
                .access_paths
                .iter()
                .filter(|p| p.kind == RELAY_ACCESS_PATH_KIND)
                .filter_map(|p| p.via_endpoint_id.clone().map(|via| (p.id.clone(), via)))
                .collect(),
            amneziawg: if cfg.role == NodeRole::Exit { amneziawg } else { None },
        }
    }
}

fn peer_node_id(failure_domain: &str) -> String {
    let mapped: String = failure_domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    format!("peer-{mapped}")
}

fn local_route_label(node_id: &str, transport: &str) -> String {
    format!("{node_id} · {transport}")
}

fn v2_params(endpoint: contract::Endpoint) -> v2::EndpointParams {
    match endpoint.params {
        contract::TransportParams::VlessReality { uuid, flow, reality } => v2::EndpointParams::VlessReality {
            server_name: endpoint.server_name,
            uuid,
            flow,
            reality,
        },
        contract::TransportParams::Hysteria2 { password, obfs } => v2::EndpointParams::Hysteria2 {
            server_name: endpoint.server_name,
            password,
            obfs,
        },
    }
}

/// Build the v2 document for `user`. `Err(NoSelectableRoute)` when the
/// user can use nothing.
pub fn provisioning_document_v2(
    ctx: &RouteContext,
    user: &CompatUser,
) -> Result<v2::ProvisioningDocumentV2, CompatError> {
    let mut nodes: BTreeMap<String, v2::Node> = BTreeMap::new();
    let mut endpoints: Vec<v2::Endpoint> = Vec::new();
    let mut routes: Vec<v2::Route> = Vec::new();

    let local_node = v2::Node {
        node_id: ctx.node_id.clone(),
        role: ctx.role.as_str().into(),
        status: "active".into(),
        failure_domain: format!("host:{}", ctx.public_host.to_ascii_lowercase()),
        provider: None,
        region: None,
        country: None,
        asn: None,
    };

    let first_hops: BTreeSet<&str> = ctx.relay_paths.iter().map(|(_, via)| via.as_str()).collect();
    let by_id: BTreeMap<&str, &CompatEndpoint> = ctx.endpoints.iter().map(|e| (e.id.as_str(), e)).collect();

    // Resolve every listener this user can authenticate to.
    let mut resolved: BTreeMap<String, v2::Endpoint> = BTreeMap::new();
    for ep in &ctx.endpoints {
        let Some(c) = contract_endpoint_opt(user, ep, VlessFlow::Vision, None)? else {
            continue;
        };
        let node_id = match ep.origin {
            EndpointOrigin::Local => ctx.node_id.clone(),
            EndpointOrigin::Peer => peer_node_id(ep.failure_domain.as_deref().unwrap_or(&ep.host)),
        };
        resolved.insert(
            ep.id.clone(),
            v2::Endpoint {
                endpoint_id: ep.id.clone(),
                node_id,
                host: c.host.clone(),
                port: c.port,
                params: v2_params(c),
            },
        );
    }

    let add_endpoint = |endpoints: &mut Vec<v2::Endpoint>, nodes: &mut BTreeMap<String, v2::Node>, e: &v2::Endpoint, source: Option<&CompatEndpoint>| {
        if endpoints.iter().any(|x| x.endpoint_id == e.endpoint_id) {
            return;
        }
        let node = if e.node_id == ctx.node_id {
            local_node.clone()
        } else {
            let src = source.expect("peer endpoints have a source");
            v2::Node {
                node_id: e.node_id.clone(),
                role: if first_hops.contains(src.id.as_str()) { "relay" } else { "exit" }.into(),
                status: "active".into(),
                failure_domain: src.failure_domain.clone().unwrap_or_else(|| format!("host:{}", src.host)),
                provider: src.provider.clone(),
                region: src.region.clone(),
                country: None,
                asn: src.asn.clone(),
            }
        };
        nodes.entry(node.node_id.clone()).or_insert(node);
        endpoints.push(e.clone());
    };

    for ep in &ctx.endpoints {
        let Some(resolved_ep) = resolved.get(&ep.id) else { continue };
        let is_first_hop = first_hops.contains(ep.id.as_str());
        let path = ep.path.as_deref().unwrap_or("direct");
        if path == "direct" {
            // A relay's own listener and any declared first hop are
            // infrastructure, never a direct exit.
            if is_first_hop || (ep.origin == EndpointOrigin::Local && ctx.role == NodeRole::Relay) {
                continue;
            }
            add_endpoint(&mut endpoints, &mut nodes, resolved_ep, Some(ep));
            routes.push(v2::Route {
                route_id: match ep.origin {
                    EndpointOrigin::Local => local_route_label(&ctx.node_id, ep.transport.as_str()).replace(" · ", "/"),
                    EndpointOrigin::Peer => ep.id.clone(),
                },
                label: match ep.origin {
                    EndpointOrigin::Local => local_route_label(&ctx.node_id, ep.transport.as_str()),
                    EndpointOrigin::Peer => ep.label.clone(),
                },
                kind: "direct".into(),
                hops: vec![ep.id.clone()],
                priority: if ep.id == LOCAL_HYSTERIA2_ENDPOINT_ID { 45 } else { 50 },
            });
            continue;
        }
        let Some((_, via)) = ctx.relay_paths.iter().find(|(id, _)| id == path) else {
            continue; // opaque legacy path label: not executable, not emitted
        };
        let (Some(first), Some(first_src)) = (resolved.get(via), by_id.get(via.as_str())) else {
            continue; // no first-hop credential: route not usable by this user
        };
        // Reuse the exit listener the alias points at, when declared.
        let exit_id = ep.credential_ref.as_deref().filter(|r| resolved.contains_key(*r)).unwrap_or(&ep.id);
        let exit = &resolved[exit_id];
        let exit_src = by_id.get(exit_id).copied().unwrap_or(ep);
        let mut first = first.clone();
        if first_src.origin == EndpointOrigin::Local {
            first.node_id = ctx.node_id.clone();
        }
        add_endpoint(&mut endpoints, &mut nodes, &first, Some(first_src));
        add_endpoint(&mut endpoints, &mut nodes, exit, Some(exit_src));
        routes.push(v2::Route {
            route_id: ep.id.clone(),
            label: ep.label.clone(),
            kind: "relayed".into(),
            hops: vec![via.clone(), exit_id.to_string()],
            priority: 20,
        });
    }

    if let (Some(awg), Some(cred)) = (&ctx.amneziawg, &user.amneziawg) {
        let p = AmneziaWgProvider
            .render_client_endpoint(awg, &user.id, cred)
            .map_err(|e| CompatError::ConfigValidationFailed(e.to_string()))?;
        let e = v2::Endpoint {
            endpoint_id: LOCAL_AMNEZIAWG_ENDPOINT_ID.into(),
            node_id: ctx.node_id.clone(),
            host: awg.public_host.clone(),
            port: awg.listen_port,
            params: v2::EndpointParams::AmneziaWg(v2::AmneziaWgParams {
                protocol: serde_json::to_value(p.protocol)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                server_public_key: p.server_public_key,
                client_private_key: p.client_private_key,
                preshared_key: p.preshared_key,
                addresses: p.addresses,
                recommended_mtu: p.mtu,
                persistent_keepalive: p.persistent_keepalive,
                jc: p.jc,
                jmin: p.jmin,
                jmax: p.jmax,
                s1: p.s1,
                s2: p.s2,
                s3: p.s3,
                s4: p.s4,
                h1: p.h1,
                h2: p.h2,
                h3: p.h3,
                h4: p.h4,
                signature_packets: p.signature_packets,
                header_protection_key: p.header_protection_key,
                content_padding_addition: p.content_padding_addition,
            }),
        };
        add_endpoint(&mut endpoints, &mut nodes, &e, None);
        routes.push(v2::Route {
            route_id: format!("{}/amneziawg", ctx.node_id),
            label: local_route_label(&ctx.node_id, "amneziawg"),
            kind: "direct".into(),
            hops: vec![LOCAL_AMNEZIAWG_ENDPOINT_ID.into()],
            priority: 50,
        });
    }

    if routes.is_empty() {
        return Err(CompatError::NoSelectableRoute);
    }
    let mut doc = v2::ProvisioningDocumentV2::new(contract::ServerInfo::current(SERVER_VERSION));
    doc.nodes = nodes.into_values().collect();
    doc.endpoints = endpoints;
    doc.routes = routes;
    doc.validate()?;
    route_catalog(&doc)
        .validate()
        .map_err(|e| CompatError::ConfigValidationFailed(format!("route catalog: {e}")))?;
    Ok(doc)
}

/// Convert a v2 document into the platform route model (scoring,
/// failover, telemetry all work on this).
pub fn route_catalog(doc: &v2::ProvisioningDocumentV2) -> pc::RouteCatalog {
    pc::RouteCatalog {
        nodes: doc
            .nodes
            .iter()
            .map(|n| pc::Node {
                node_id: n.node_id.clone(),
                role: if n.role == "relay" { pc::NodeRole::Relay } else { pc::NodeRole::Exit },
                status: if n.status == "draining" { pc::NodeStatus::Draining } else { pc::NodeStatus::Active },
                failure_domain: n.failure_domain.clone(),
                provider: n.provider.clone(),
                region: n.region.clone(),
                country: n.country.clone(),
                asn: n.asn.clone(),
                cost_hint: None,
            })
            .collect(),
        endpoints: doc
            .endpoints
            .iter()
            .map(|e| pc::Endpoint {
                endpoint_id: e.endpoint_id.clone(),
                node_id: e.node_id.clone(),
                transport: TransportKind::from_wire(e.params.transport()),
                host: e.host.clone(),
                port: e.port,
            })
            .collect(),
        routes: doc
            .routes
            .iter()
            .map(|r| pc::Route {
                route_id: r.route_id.clone(),
                label: r.label.clone(),
                kind: if r.kind == "relayed" { pc::RouteKind::Relayed } else { pc::RouteKind::Direct },
                hops: r.hops.clone(),
                priority: r.priority,
            })
            .collect(),
    }
}

/// Ids of this node's listeners that v2 knows; used by tests and doctor.
pub const LOCAL_ENDPOINT_IDS_V2: &[&str] =
    &[LOCAL_REALITY_ENDPOINT_ID, LOCAL_HYSTERIA2_ENDPOINT_ID, LOCAL_AMNEZIAWG_ENDPOINT_ID];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amneziawg::{derive_public_key, generate_private_key, AwgParams, AwgProfile};
    use crate::model::PeerCredential;
    use crate::secret::SecretString;

    const RPK: &str = "pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk";
    const EXIT_PK: &str = "zo060cy2M-x7cMF4FKXHbs0CloUFDTRHRboFhw5YfVk";

    fn user() -> CompatUser {
        CompatUser {
            id: "u1".into(),
            name: "alice".into(),
            enabled: true,
            vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
            hysteria2_password: SecretString::new("hy2-local-secret"),
            subscription_token_hash_hex: "00".into(),
            created_at: 0,
            expires_at: None,
            vision_off_experiment: false,
            peer_credentials: Default::default(),
            amneziawg: None,
        }
    }

    fn deployment(extra: &str, role: &str) -> DeploymentConfig {
        let text = format!(
            "schema_version = 2\nnode_id = \"n1\"\nrole = \"{role}\"\npublic_host = \"n1.example.net\"\nsubscription_host = \"n1.example.net\"\n\n[reality]\nlisten_port = 443\nhandshake_server = \"www.example.com\"\n\n[hysteria2]\nlisten_port = 443\n\n[subscription]\nlisten_port = 9100\n{extra}"
        );
        let cfg: DeploymentConfig = toml::from_str(&text).unwrap();
        cfg.validate().unwrap();
        cfg
    }

    fn ctx(cfg: &DeploymentConfig, awg: Option<AwgNodeConfig>) -> RouteContext {
        let endpoints = cfg.served_endpoints(RPK, "0a1b2c3d", Some("obfs")).unwrap();
        RouteContext::from_deployment(cfg, endpoints, awg)
    }

    fn awg_node() -> AwgNodeConfig {
        let k = generate_private_key();
        AwgNodeConfig {
            interface: "awg0".into(),
            public_host: "n1.example.net".into(),
            listen_port: 51820,
            subnet_v4: "10.66.0.0/24".parse().unwrap(),
            subnet_v6: None,
            mtu: 1380,
            persistent_keepalive: 25,
            fallback_client_dns: vec![],
            server_public_key: derive_public_key(k.expose()).unwrap(),
            server_private_key: None,
            params: AwgParams::generate(AwgProfile::Awg3),
        }
    }

    #[test]
    fn single_node_without_awg_has_reality_and_hysteria_routes() {
        let cfg = deployment("", "exit");
        let doc = provisioning_document_v2(&ctx(&cfg, None), &user()).unwrap();
        let ids: Vec<&str> = doc.routes.iter().map(|r| r.route_id.as_str()).collect();
        assert_eq!(ids, vec!["n1/vless-reality", "n1/hysteria2"]);
        assert_eq!(doc.nodes.len(), 1);
        assert_eq!(doc.nodes[0].failure_domain, "host:n1.example.net");
        route_catalog(&doc).validate().unwrap();
    }

    #[test]
    fn awg_route_appears_only_for_users_with_a_credential() {
        let cfg = deployment("", "exit");
        let node = awg_node();
        let mut u = user();
        let without = provisioning_document_v2(&ctx(&cfg, Some(node.clone())), &u).unwrap();
        assert!(without.routes.iter().all(|r| !r.route_id.ends_with("amneziawg")));

        u.amneziawg = Some(AmneziaWgProvider.issue_credentials(&node, "u1", &[]).unwrap());
        let with = provisioning_document_v2(&ctx(&cfg, Some(node.clone())), &u).unwrap();
        let awg = with.endpoints.iter().find(|e| e.endpoint_id == LOCAL_AMNEZIAWG_ENDPOINT_ID).unwrap();
        let v2::EndpointParams::AmneziaWg(p) = &awg.params else { panic!("not awg") };
        assert_eq!(p.client_private_key, u.amneziawg.as_ref().unwrap().private_key.expose());
        assert!(with.routes.iter().any(|r| r.route_id == "n1/amneziawg"));
        let json = with.to_json().unwrap();
        assert!(!json.contains("\"dns\""));
    }

    #[test]
    fn two_independent_exits_map_to_two_nodes_with_separate_credentials() {
        let extra = format!(
            "\n[[peer_endpoints]]\nid = \"nl1-reality\"\ntag = \"Netherlands\"\nhost = \"nl1.example.org\"\nport = 443\ntransport = \"vless_reality\"\nserver_name = \"www.example.org\"\nreality_public_key = \"{EXIT_PK}\"\nreality_short_id = \"9f8e\"\nfailure_domain = \"provider-b/nl\"\nprovider = \"provider-b\"\n"
        );
        let cfg = deployment(&extra, "exit");
        let mut u = user();
        u.peer_credentials.insert(
            "nl1-reality".into(),
            PeerCredential::VlessReality { uuid: "22222222-2222-4222-8222-222222222222".into() },
        );
        let doc = provisioning_document_v2(&ctx(&cfg, None), &u).unwrap();
        assert_eq!(doc.nodes.len(), 2);
        let catalog = route_catalog(&doc);
        catalog.validate().unwrap();
        let domains: BTreeSet<String> = catalog.nodes.iter().map(|n| n.failure_domain.clone()).collect();
        assert_eq!(domains.len(), 2);
        let uuids: BTreeSet<String> = doc
            .endpoints
            .iter()
            .filter_map(|e| match &e.params {
                v2::EndpointParams::VlessReality { uuid, .. } => Some(uuid.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(uuids.len(), 2, "each node must carry its own credential");

        // Without the peer credential the peer vanishes, the local node remains.
        let doc = provisioning_document_v2(&ctx(&cfg, None), &user()).unwrap();
        assert_eq!(doc.nodes.len(), 1);
    }

    #[test]
    fn relay_node_emits_two_hop_routes_and_never_its_own_listener_as_exit() {
        let extra = format!(
            "\n[[access_paths]]\nid = \"via-n1\"\nkind = \"relay\"\nvia_endpoint_id = \"reality-1\"\ncapabilities = [\"tcp\"]\n\n[[peer_endpoints]]\nid = \"de1-direct\"\ntag = \"Germany\"\nhost = \"de1.example.org\"\nport = 443\ntransport = \"vless_reality\"\nserver_name = \"www.example.org\"\nreality_public_key = \"{EXIT_PK}\"\nreality_short_id = \"9f8e\"\nfailure_domain = \"de1\"\n\n[[peer_endpoints]]\nid = \"de1-via-n1\"\ntag = \"Germany via relay\"\nhost = \"de1.example.org\"\nport = 443\ntransport = \"vless_reality\"\nserver_name = \"www.example.org\"\nreality_public_key = \"{EXIT_PK}\"\nreality_short_id = \"9f8e\"\nfailure_domain = \"de1\"\npath = \"via-n1\"\ncredential_ref = \"de1-direct\"\n"
        );
        let cfg = deployment(&extra, "relay");
        let mut u = user();
        u.peer_credentials.insert(
            "de1-direct".into(),
            PeerCredential::VlessReality { uuid: "33333333-3333-4333-8333-333333333333".into() },
        );
        let doc = provisioning_document_v2(&ctx(&cfg, None), &u).unwrap();
        let relayed = doc.routes.iter().find(|r| r.kind == "relayed").unwrap();
        assert_eq!(relayed.hops, vec!["reality-1".to_string(), "de1-direct".to_string()]);
        assert!(doc.routes.iter().all(|r| r.hops != vec!["reality-1".to_string()]), "relay listener offered as exit");
        let catalog = route_catalog(&doc);
        catalog.validate().unwrap();
        assert_eq!(catalog.node("n1").unwrap().role, pc::NodeRole::Relay);
        // Two routes share one exit endpoint (credential_ref), not two listeners.
        assert_eq!(doc.endpoints.iter().filter(|e| e.host == "de1.example.org").count(), 1);
    }

    #[test]
    fn user_with_nothing_routable_gets_no_selectable_route() {
        let extra = "\n[[access_paths]]\nid = \"via-n1\"\nkind = \"relay\"\nvia_endpoint_id = \"reality-1\"\ncapabilities = [\"tcp\"]\n";
        let cfg = deployment(extra, "relay");
        assert!(matches!(
            provisioning_document_v2(&ctx(&cfg, None), &user()),
            Err(CompatError::NoSelectableRoute)
        ));
    }

    #[test]
    fn v1_and_v2_agree_on_credentials() {
        let cfg = deployment("", "exit");
        let c = ctx(&cfg, None);
        let u = user();
        let v1 = crate::contract::provisioning_document(&u, &c.endpoints).unwrap();
        let v2 = provisioning_document_v2(&c, &u).unwrap();
        let v1_json = serde_json::to_string(&v1.endpoints).unwrap();
        for e in &v2.endpoints {
            match &e.params {
                v2::EndpointParams::VlessReality { uuid, .. } => assert!(v1_json.contains(uuid)),
                v2::EndpointParams::Hysteria2 { password, .. } => assert!(v1_json.contains(password)),
                _ => {}
            }
        }
    }
}
