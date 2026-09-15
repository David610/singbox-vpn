//! Nodes, endpoints and routes — the first-class route model.
//!
//! A *route* is what a client selects: one endpoint (direct) or a chain
//! of two endpoints on different nodes (relayed). Scoring, failover and
//! telemetry all work in terms of route ids, so the platform reasons
//! about `Finland/provider-a/AWG` and `Netherlands/provider-b/REALITY`
//! rather than about protocols.
//!
//! Metadata here is non-secret. Credentials are keyed per (user,
//! endpoint) elsewhere and never appear in this model.

use crate::transport::TransportKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Exit,
    Relay,
}

/// Lifecycle states that matter to route selection. The full
/// provisioning state machine is in `lifecycle.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Being created or bootstrapped; never served.
    Provisioning,
    Active,
    /// Still served to clients that already have it, penalised, never
    /// preferred for a new session.
    Draining,
    /// Never served.
    Retired,
}

impl NodeStatus {
    pub fn is_servable(self) -> bool {
        matches!(self, NodeStatus::Active | NodeStatus::Draining)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub node_id: String,
    pub role: NodeRole,
    pub status: NodeStatus,
    /// Shared-fate identifier. Nodes with the same value are expected to
    /// fail together (same provider account, same datacenter, …).
    pub failure_domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// ISO 3166-1 alpha-2, operator-declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn: Option<String>,
    /// Relative monthly cost hint, operator-declared, unitless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_hint: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub endpoint_id: String,
    pub node_id: String,
    pub transport: TransportKind,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
    Direct,
    Relayed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub route_id: String,
    pub label: String,
    pub kind: RouteKind,
    /// Endpoint ids from the client outward. `hops.last()` is the exit.
    pub hops: Vec<String>,
    /// Operator preference, 0..=100.
    pub priority: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCatalog {
    pub nodes: Vec<Node>,
    pub endpoints: Vec<Endpoint>,
    pub routes: Vec<Route>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("duplicate {what} id {id:?}")]
    DuplicateId { what: &'static str, id: String },
    #[error("empty {what} id")]
    EmptyId { what: &'static str },
    #[error("endpoint {endpoint:?} references unknown node {node:?}")]
    UnknownNode { endpoint: String, node: String },
    #[error("route {route:?} references unknown endpoint {endpoint:?}")]
    UnknownEndpoint { route: String, endpoint: String },
    #[error("route {route:?}: {reason}")]
    InvalidRoute { route: String, reason: String },
    #[error("endpoint {endpoint:?}: {reason}")]
    InvalidEndpoint { endpoint: String, reason: String },
    #[error("duplicate listener {host}:{port}/{transport} on endpoints {first:?} and {second:?}")]
    DuplicateListener {
        host: String,
        port: u16,
        transport: String,
        first: String,
        second: String,
    },
}

/// Maximum hops a route may have. Two matches the relay implementation;
/// longer chains are a deliberate future decision, not an accident.
pub const MAX_HOPS: usize = 2;

impl RouteCatalog {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.node_id == id)
    }

    pub fn endpoint(&self, id: &str) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.endpoint_id == id)
    }

    pub fn route(&self, id: &str) -> Option<&Route> {
        self.routes.iter().find(|r| r.route_id == id)
    }

    pub fn validate(&self) -> Result<(), CatalogError> {
        unique_ids("node", self.nodes.iter().map(|n| n.node_id.as_str()))?;
        unique_ids(
            "endpoint",
            self.endpoints.iter().map(|e| e.endpoint_id.as_str()),
        )?;
        unique_ids("route", self.routes.iter().map(|r| r.route_id.as_str()))?;

        let mut listeners: BTreeMap<(String, u16, bool), &str> = BTreeMap::new();
        for e in &self.endpoints {
            if self.node(&e.node_id).is_none() {
                return Err(CatalogError::UnknownNode {
                    endpoint: e.endpoint_id.clone(),
                    node: e.node_id.clone(),
                });
            }
            if e.port == 0 || e.host.trim().is_empty() || e.host.contains(char::is_whitespace) {
                return Err(CatalogError::InvalidEndpoint {
                    endpoint: e.endpoint_id.clone(),
                    reason: "host/port is not dialable".into(),
                });
            }
            // Two endpoints may share a host:port only on different L4
            // protocols (REALITY on TCP/443 and Hysteria2 on UDP/443).
            let udp = e
                .transport
                .capabilities()
                .map(|c| c.l4 == crate::transport::L4Protocol::Udp);
            if let Some(udp) = udp {
                let key = (e.host.to_ascii_lowercase(), e.port, udp);
                if let Some(first) = listeners.insert(key, &e.endpoint_id) {
                    return Err(CatalogError::DuplicateListener {
                        host: e.host.clone(),
                        port: e.port,
                        transport: e.transport.to_string(),
                        first: first.to_string(),
                        second: e.endpoint_id.clone(),
                    });
                }
            }
        }

        for r in &self.routes {
            self.validate_route(r)?;
        }
        Ok(())
    }

    fn validate_route(&self, r: &Route) -> Result<(), CatalogError> {
        let invalid = |reason: &str| CatalogError::InvalidRoute {
            route: r.route_id.clone(),
            reason: reason.to_string(),
        };
        if r.hops.is_empty() || r.hops.len() > MAX_HOPS {
            return Err(invalid("a route has one or two hops"));
        }
        if r.priority > 100 {
            return Err(invalid("priority must be 0..=100"));
        }
        let expected_kind = if r.hops.len() == 1 {
            RouteKind::Direct
        } else {
            RouteKind::Relayed
        };
        if r.kind != expected_kind {
            return Err(invalid("kind does not match hop count"));
        }
        let mut hop_endpoints = Vec::with_capacity(r.hops.len());
        for hop in &r.hops {
            let e = self
                .endpoint(hop)
                .ok_or_else(|| CatalogError::UnknownEndpoint {
                    route: r.route_id.clone(),
                    endpoint: hop.clone(),
                })?;
            hop_endpoints.push(e);
        }
        let exit = hop_endpoints.last().expect("non-empty");
        let exit_node = self.node(&exit.node_id).expect("validated");
        if exit_node.role != NodeRole::Exit {
            return Err(invalid("the last hop must be on an exit node"));
        }
        if r.kind == RouteKind::Relayed {
            let first = hop_endpoints[0];
            if first.node_id == exit.node_id {
                return Err(invalid("relay and exit hops must be on different nodes"));
            }
            let first_node = self.node(&first.node_id).expect("validated");
            if first_node.role != NodeRole::Relay {
                return Err(invalid(
                    "the first hop of a relayed route must be on a relay node",
                ));
            }
            let (Some(fc), Some(ec)) = (
                first.transport.capabilities(),
                exit.transport.capabilities(),
            ) else {
                return Err(invalid(
                    "relayed routes require transports this build implements",
                ));
            };
            if !ec.can_chain_through(&fc) {
                return Err(invalid(
                    "exit transport cannot be chained through the first hop",
                ));
            }
        }
        Ok(())
    }

    /// Nodes a route depends on, client outward.
    pub fn route_nodes(&self, route: &Route) -> Vec<&Node> {
        route
            .hops
            .iter()
            .filter_map(|h| self.endpoint(h))
            .filter_map(|e| self.node(&e.node_id))
            .collect()
    }

    /// Union of failure domains a route shares fate with.
    pub fn failure_domains(&self, route: &Route) -> BTreeSet<String> {
        self.route_nodes(route)
            .into_iter()
            .map(|n| n.failure_domain.clone())
            .collect()
    }

    pub fn exit_endpoint(&self, route: &Route) -> Option<&Endpoint> {
        route.hops.last().and_then(|h| self.endpoint(h))
    }

    pub fn exit_transport(&self, route: &Route) -> Option<&TransportKind> {
        self.exit_endpoint(route).map(|e| &e.transport)
    }

    /// The transport of the hop the client dials directly.
    pub fn entry_transport(&self, route: &Route) -> Option<&TransportKind> {
        route
            .hops
            .first()
            .and_then(|h| self.endpoint(h))
            .map(|e| &e.transport)
    }

    /// Whether every node the route depends on is servable.
    pub fn is_servable(&self, route: &Route) -> bool {
        let nodes = self.route_nodes(route);
        nodes.len() == route.hops.len() && nodes.iter().all(|n| n.status.is_servable())
    }

    /// Whether any node on the route is draining.
    pub fn is_draining(&self, route: &Route) -> bool {
        self.route_nodes(route)
            .iter()
            .any(|n| n.status == NodeStatus::Draining)
    }

    /// Routes a client that can run `supported` transports may use.
    pub fn selectable_routes<'a>(
        &'a self,
        supported: &'a dyn Fn(&TransportKind) -> bool,
    ) -> impl Iterator<Item = &'a Route> + 'a {
        self.routes.iter().filter(move |r| {
            self.is_servable(r)
                && r.hops
                    .iter()
                    .filter_map(|h| self.endpoint(h))
                    .all(|e| supported(&e.transport))
        })
    }

    /// Other routes that share a node with `route`.
    pub fn routes_sharing_node<'a>(
        &'a self,
        route: &'a Route,
    ) -> impl Iterator<Item = &'a Route> + 'a {
        let nodes: BTreeSet<&str> = self
            .route_nodes(route)
            .into_iter()
            .map(|n| n.node_id.as_str())
            .collect();
        self.routes.iter().filter(move |other| {
            other.route_id != route.route_id
                && self
                    .route_nodes(other)
                    .iter()
                    .any(|n| nodes.contains(n.node_id.as_str()))
        })
    }
}

fn unique_ids<'a>(
    what: &'static str,
    ids: impl Iterator<Item = &'a str>,
) -> Result<(), CatalogError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.trim().is_empty() {
            return Err(CatalogError::EmptyId { what });
        }
        if !seen.insert(id) {
            return Err(CatalogError::DuplicateId {
                what,
                id: id.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn node(id: &str, role: NodeRole, domain: &str) -> Node {
        Node {
            node_id: id.into(),
            role,
            status: NodeStatus::Active,
            failure_domain: domain.into(),
            provider: None,
            region: None,
            country: None,
            asn: None,
            cost_hint: None,
        }
    }

    pub fn endpoint(id: &str, node: &str, transport: TransportKind, port: u16) -> Endpoint {
        Endpoint {
            endpoint_id: id.into(),
            node_id: node.into(),
            transport,
            host: format!("{node}.example.net"),
            port,
        }
    }

    pub fn direct(id: &str, endpoint: &str, priority: u8) -> Route {
        Route {
            route_id: id.into(),
            label: id.into(),
            kind: RouteKind::Direct,
            hops: vec![endpoint.into()],
            priority,
        }
    }

    /// Two exits on independent providers, each with all three
    /// transports, plus one relay.
    pub fn two_node_catalog() -> RouteCatalog {
        RouteCatalog {
            nodes: vec![
                node("fi1", NodeRole::Exit, "provider-a/fi"),
                node("nl1", NodeRole::Exit, "provider-b/nl"),
                node("ru1", NodeRole::Relay, "provider-c/ru"),
            ],
            endpoints: vec![
                endpoint("fi1-awg", "fi1", TransportKind::AmneziaWg, 51820),
                endpoint("fi1-reality", "fi1", TransportKind::VlessReality, 443),
                endpoint("fi1-hy2", "fi1", TransportKind::Hysteria2, 443),
                endpoint("nl1-awg", "nl1", TransportKind::AmneziaWg, 51820),
                endpoint("nl1-reality", "nl1", TransportKind::VlessReality, 443),
                endpoint("nl1-hy2", "nl1", TransportKind::Hysteria2, 443),
                endpoint("ru1-reality", "ru1", TransportKind::VlessReality, 443),
            ],
            routes: vec![
                direct("fi1/awg", "fi1-awg", 60),
                direct("fi1/reality", "fi1-reality", 50),
                direct("fi1/hy2", "fi1-hy2", 40),
                direct("nl1/awg", "nl1-awg", 55),
                direct("nl1/reality", "nl1-reality", 45),
                direct("nl1/hy2", "nl1-hy2", 35),
                Route {
                    route_id: "nl1/reality-via-ru1".into(),
                    label: "Netherlands via Russia".into(),
                    kind: RouteKind::Relayed,
                    hops: vec!["ru1-reality".into(), "nl1-reality".into()],
                    priority: 20,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn two_node_catalog_is_valid() {
        two_node_catalog().validate().unwrap();
    }

    #[test]
    fn one_node_and_n_node_catalogs_are_valid() {
        let one = RouteCatalog {
            nodes: vec![node("a", NodeRole::Exit, "a")],
            endpoints: vec![endpoint("a-reality", "a", TransportKind::VlessReality, 443)],
            routes: vec![direct("a/reality", "a-reality", 50)],
        };
        one.validate().unwrap();

        let mut many = RouteCatalog::default();
        for i in 0..32 {
            let id = format!("n{i}");
            many.nodes.push(node(&id, NodeRole::Exit, &format!("d{i}")));
            let ep = format!("{id}-awg");
            many.endpoints
                .push(endpoint(&ep, &id, TransportKind::AmneziaWg, 51820));
            many.routes.push(direct(&format!("{id}/awg"), &ep, 50));
        }
        many.validate().unwrap();
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut c = two_node_catalog();
        c.nodes.push(node("fi1", NodeRole::Exit, "x"));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::DuplicateId { what: "node", .. })
        ));

        let mut c = two_node_catalog();
        c.endpoints
            .push(endpoint("fi1-awg", "nl1", TransportKind::AmneziaWg, 1));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::DuplicateId {
                what: "endpoint",
                ..
            })
        ));

        let mut c = two_node_catalog();
        c.routes.push(direct("fi1/awg", "nl1-awg", 1));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::DuplicateId { what: "route", .. })
        ));
    }

    #[test]
    fn same_port_is_allowed_only_across_l4_protocols() {
        let c = two_node_catalog();
        // fi1-reality (TCP/443) and fi1-hy2 (UDP/443) coexist already.
        c.validate().unwrap();
        let mut c = c;
        c.endpoints
            .push(endpoint("fi1-awg2", "fi1", TransportKind::AmneziaWg, 443));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::DuplicateListener { .. })
        ));
    }

    #[test]
    fn relayed_routes_are_constrained() {
        let mut c = two_node_catalog();
        c.routes.push(Route {
            route_id: "bad-udp".into(),
            label: "x".into(),
            kind: RouteKind::Relayed,
            hops: vec!["ru1-reality".into(), "nl1-awg".into()],
            priority: 10,
        });
        assert!(matches!(
            c.validate(),
            Err(CatalogError::InvalidRoute { .. })
        ));

        let mut c = two_node_catalog();
        c.routes.push(Route {
            route_id: "same-node".into(),
            label: "x".into(),
            kind: RouteKind::Relayed,
            hops: vec!["nl1-reality".into(), "nl1-reality".into()],
            priority: 10,
        });
        assert!(matches!(
            c.validate(),
            Err(CatalogError::InvalidRoute { .. })
        ));

        let mut c = two_node_catalog();
        c.routes.push(direct("relay-as-exit", "ru1-reality", 10));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::InvalidRoute { .. })
        ));

        let mut c = two_node_catalog();
        c.routes.push(Route {
            route_id: "three".into(),
            label: "x".into(),
            kind: RouteKind::Relayed,
            hops: vec![
                "ru1-reality".into(),
                "fi1-reality".into(),
                "nl1-reality".into(),
            ],
            priority: 10,
        });
        assert!(matches!(
            c.validate(),
            Err(CatalogError::InvalidRoute { .. })
        ));
    }

    #[test]
    fn dangling_references_are_refused() {
        let mut c = two_node_catalog();
        c.endpoints
            .push(endpoint("ghost", "nowhere", TransportKind::VlessReality, 1));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::UnknownNode { .. })
        ));

        let mut c = two_node_catalog();
        c.routes.push(direct("ghost-route", "nope", 1));
        assert!(matches!(
            c.validate(),
            Err(CatalogError::UnknownEndpoint { .. })
        ));
    }

    #[test]
    fn failure_domains_and_shared_nodes() {
        let c = two_node_catalog();
        let via = c.route("nl1/reality-via-ru1").unwrap();
        let domains = c.failure_domains(via);
        assert_eq!(
            domains.into_iter().collect::<Vec<_>>(),
            vec!["provider-b/nl".to_string(), "provider-c/ru".to_string()]
        );
        let sharing: Vec<_> = c
            .routes_sharing_node(c.route("fi1/awg").unwrap())
            .map(|r| r.route_id.as_str())
            .collect();
        assert_eq!(sharing, vec!["fi1/reality", "fi1/hy2"]);
    }

    #[test]
    fn retired_and_provisioning_nodes_are_not_selectable() {
        let mut c = two_node_catalog();
        c.nodes[0].status = NodeStatus::Retired;
        c.nodes[1].status = NodeStatus::Draining;
        let all = |_: &TransportKind| true;
        let ids: Vec<_> = c
            .selectable_routes(&all)
            .map(|r| r.route_id.clone())
            .collect();
        assert!(ids.iter().all(|id| !id.starts_with("fi1")));
        assert!(ids.iter().any(|id| id.starts_with("nl1")));
        assert!(c.is_draining(c.route("nl1/awg").unwrap()));
    }

    #[test]
    fn clients_without_awg_never_see_awg_routes() {
        let c = two_node_catalog();
        let no_awg = |t: &TransportKind| *t != TransportKind::AmneziaWg;
        assert!(c
            .selectable_routes(&no_awg)
            .all(|r| c.exit_transport(r) != Some(&TransportKind::AmneziaWg)));
    }
}
