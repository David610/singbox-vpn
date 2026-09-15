//! Provisioning contract `schema_version = 2`: nodes, endpoints and routes.
//!
//! Served at `GET /v2/provision/{token}`. Version 1 is unchanged and keeps
//! being served at `/v1/provision/{token}`.
//!
//! What changes relative to v1:
//!
//! * **Routes are first-class.** A route is one endpoint (direct) or two
//!   endpoints on different nodes (relayed). Clients score and fail over
//!   between routes, not between protocols.
//! * **Nodes carry shared-fate metadata** (`failure_domain`, operator
//!   labels) instead of every endpoint repeating it.
//! * **AmneziaWG** is a transport. It has no sing-box representation, which
//!   is why it could not be added to v1 without breaking v1's
//!   `singbox_config` invariant.
//! * **No embedded Core configuration.** The first-party client builds its
//!   engine configuration from these structured parameters and keeps DNS,
//!   TUN, routes and kill switch under its own policy.
//!
//! What does not change: the document is per-user and credential-bearing
//! (never logged), it carries no server private key, no filesystem path and
//! no client-owned policy, and unknown transports are skipped by clients
//! rather than reinterpreted.

use crate::{is_uuid, ContractError, Hysteria2Obfs, RealityParams, ServerInfo, VLESS_FLOW_VISION};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 2;

/// Oldest client release able to consume v2 documents.
pub const MINIMUM_CLIENT_VERSION: &str = "0.3.0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub node_id: String,
    /// `exit` or `relay`.
    pub role: String,
    /// `active` or `draining`.
    pub status: String,
    pub failure_domain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn: Option<String>,
}

/// AmneziaWG parameters for one user on one node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmneziaWgParams {
    /// `awg-3` or `awg-2-compatible`.
    pub protocol: String,
    pub server_public_key: String,
    /// The user's tunnel private key. Credential; the document is secret.
    pub client_private_key: String,
    pub preshared_key: String,
    /// Tunnel addresses assigned to this user (`a.b.c.d/32`, `…/128`).
    pub addresses: Vec<String>,
    /// Server recommendation accounting for AWG padding; a client may use a
    /// smaller value, never a larger one.
    pub recommended_mtu: u16,
    pub persistent_keepalive: u16,
    pub jc: u16,
    pub jmin: u16,
    pub jmax: u16,
    pub s1: u16,
    pub s2: u16,
    pub s3: u16,
    pub s4: u16,
    pub h1: String,
    pub h2: String,
    pub h3: String,
    pub h4: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature_packets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_protection_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_padding_addition: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport")]
pub enum EndpointParams {
    #[serde(rename = "vless-reality")]
    VlessReality {
        server_name: String,
        uuid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flow: Option<String>,
        reality: RealityParams,
    },
    #[serde(rename = "hysteria2")]
    Hysteria2 {
        server_name: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        obfs: Option<Hysteria2Obfs>,
    },
    #[serde(rename = "amneziawg")]
    AmneziaWg(AmneziaWgParams),
    /// A transport this build does not know. Parsed so the rest of the
    /// document stays usable; routes through it must be skipped.
    #[serde(other)]
    Unknown,
}

impl EndpointParams {
    pub fn transport(&self) -> &'static str {
        match self {
            EndpointParams::VlessReality { .. } => "vless-reality",
            EndpointParams::Hysteria2 { .. } => "hysteria2",
            EndpointParams::AmneziaWg(_) => "amneziawg",
            EndpointParams::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub endpoint_id: String,
    pub node_id: String,
    pub host: String,
    pub port: u16,
    #[serde(flatten)]
    pub params: EndpointParams,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub route_id: String,
    pub label: String,
    /// `direct` or `relayed`.
    pub kind: String,
    /// Endpoint ids from the client outward; the last is the exit.
    pub hops: Vec<String>,
    /// Operator preference 0..=100.
    pub priority: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisioningDocumentV2 {
    pub schema_version: u32,
    pub server: ServerInfo,
    pub nodes: Vec<Node>,
    pub endpoints: Vec<Endpoint>,
    pub routes: Vec<Route>,
}

fn invalid(field: &'static str, reason: impl Into<String>) -> ContractError {
    ContractError::Invalid { field, reason: reason.into() }
}

/// Keys that express client-owned policy or server-private material and
/// must never appear anywhere in a v2 document.
const FORBIDDEN_KEYS: &[&str] = &[
    "dns", "mtu", "tun", "inbounds", "auto_route", "strict_route", "kill_switch", "insecure",
    "ipv4", "ipv6", "private_key", "privatekey", "private-key", "server_private_key",
    "singbox_config",
];

/// The one private-key-shaped field v2 permits: the user's own AWG key.
const ALLOWED_PRIVATE_KEY_FIELD: &str = "client_private_key";

const FORBIDDEN_VALUE_FRAGMENTS: &[&str] = &["-----BEGIN", ".pem", "/etc/", "/var/", "/opt/"];

impl ProvisioningDocumentV2 {
    pub fn new(server: ServerInfo) -> Self {
        ProvisioningDocumentV2 {
            schema_version: SCHEMA_VERSION,
            server: ServerInfo { minimum_client_version: MINIMUM_CLIENT_VERSION.into(), ..server },
            nodes: Vec::new(),
            endpoints: Vec::new(),
            routes: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ContractError::UnsupportedSchemaVersion {
                requested: self.schema_version,
                supported: vec![SCHEMA_VERSION],
            });
        }
        if self.server.product != crate::PRODUCT {
            return Err(invalid("server.product", format!("{:?} is not {:?}", self.server.product, crate::PRODUCT)));
        }
        if self.routes.is_empty() {
            return Err(ContractError::Empty { field: "routes" });
        }

        let mut node_ids = BTreeSet::new();
        for n in &self.nodes {
            non_empty("node.node_id", &n.node_id)?;
            non_empty("node.failure_domain", &n.failure_domain)?;
            if !node_ids.insert(n.node_id.as_str()) {
                return Err(invalid("node.node_id", format!("duplicate {:?}", n.node_id)));
            }
            if !matches!(n.role.as_str(), "exit" | "relay") {
                return Err(invalid("node.role", format!("{:?}", n.role)));
            }
            if !matches!(n.status.as_str(), "active" | "draining") {
                return Err(invalid("node.status", format!("{:?} (only servable nodes appear)", n.status)));
            }
        }

        let mut endpoint_ids = BTreeSet::new();
        for e in &self.endpoints {
            non_empty("endpoint.endpoint_id", &e.endpoint_id)?;
            if !endpoint_ids.insert(e.endpoint_id.as_str()) {
                return Err(invalid("endpoint.endpoint_id", format!("duplicate {:?}", e.endpoint_id)));
            }
            if !node_ids.contains(e.node_id.as_str()) {
                return Err(invalid("endpoint.node_id", format!("{:?} is not a node", e.node_id)));
            }
            non_empty("endpoint.host", &e.host)?;
            if e.host.chars().any(|c| c.is_whitespace() || c == '/') {
                return Err(invalid("endpoint.host", format!("{:?} is not dialable", e.host)));
            }
            if e.port == 0 {
                return Err(invalid("endpoint.port", "port 0 is not dialable"));
            }
            validate_params(e)?;
        }

        let mut route_ids = BTreeSet::new();
        for r in &self.routes {
            non_empty("route.route_id", &r.route_id)?;
            non_empty("route.label", &r.label)?;
            if !route_ids.insert(r.route_id.as_str()) {
                return Err(invalid("route.route_id", format!("duplicate {:?}", r.route_id)));
            }
            if r.priority > 100 {
                return Err(invalid("route.priority", "must be 0..=100"));
            }
            let expected = match r.hops.len() {
                1 => "direct",
                2 => "relayed",
                _ => return Err(invalid("route.hops", format!("{:?} must have one or two hops", r.route_id))),
            };
            if r.kind != expected {
                return Err(invalid("route.kind", format!("{:?} does not match its hop count", r.route_id)));
            }
            for hop in &r.hops {
                if !endpoint_ids.contains(hop.as_str()) {
                    return Err(invalid("route.hops", format!("{:?} references unknown endpoint {hop:?}", r.route_id)));
                }
            }
        }

        // Every node and endpoint must be reachable from some route: the
        // document never ships credentials that nothing can use.
        let used: BTreeSet<&str> = self.routes.iter().flat_map(|r| r.hops.iter().map(String::as_str)).collect();
        if let Some(e) = self.endpoints.iter().find(|e| !used.contains(e.endpoint_id.as_str())) {
            return Err(invalid("endpoint.endpoint_id", format!("{:?} is not used by any route", e.endpoint_id)));
        }
        let used_nodes: BTreeSet<&str> = self.endpoints.iter().map(|e| e.node_id.as_str()).collect();
        if let Some(n) = self.nodes.iter().find(|n| !used_nodes.contains(n.node_id.as_str())) {
            return Err(invalid("node.node_id", format!("{:?} has no endpoints", n.node_id)));
        }

        audit(&serde_json::to_value(self).map_err(|e| invalid("document", e.to_string()))?, None)
    }

    pub fn to_json(&self) -> Result<String, ContractError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| invalid("document", e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, ContractError> {
        let raw: serde_json::Value =
            serde_json::from_str(s).map_err(|e| invalid("document", format!("could not be parsed: {e}")))?;
        // Audit the document as received. Typed parsing silently drops
        // unknown keys, so auditing only the re-serialized struct would
        // accept a document that smuggles client policy (`dns`, …) or a
        // server private key in a field this build does not model.
        audit(&raw, None)?;
        let doc: ProvisioningDocumentV2 =
            serde_json::from_value(raw).map_err(|e| invalid("document", format!("could not be parsed: {e}")))?;
        doc.validate()?;
        Ok(doc)
    }
}

fn non_empty(field: &'static str, v: &str) -> Result<(), ContractError> {
    if v.trim().is_empty() {
        Err(ContractError::Empty { field })
    } else {
        Ok(())
    }
}

fn is_wg_key(s: &str) -> bool {
    s.len() == 44
        && s.ends_with('=')
        && s[..43].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
}

fn is_range(s: &str) -> bool {
    let mut parts = s.split('-');
    let (Some(a), b, None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    match (a.parse::<u32>(), b.map(str::parse::<u32>)) {
        (Ok(_), None) => true,
        (Ok(lo), Some(Ok(hi))) => lo <= hi,
        _ => false,
    }
}

fn validate_params(e: &Endpoint) -> Result<(), ContractError> {
    match &e.params {
        EndpointParams::VlessReality { server_name, uuid, flow, reality } => {
            non_empty("endpoint.server_name", server_name)?;
            if !is_uuid(uuid) {
                return Err(ContractError::MalformedUuid { endpoint_id: e.endpoint_id.clone(), got: uuid.clone() });
            }
            if flow.as_deref().is_some_and(|f| f != VLESS_FLOW_VISION) {
                return Err(invalid("endpoint.flow", format!("{flow:?}")));
            }
            non_empty("endpoint.reality.public_key", &reality.public_key)?;
            non_empty("endpoint.reality.fingerprint", &reality.fingerprint)?;
            let sid = &reality.short_id;
            if sid.is_empty() || sid.len() > 16 || sid.len() % 2 != 0 || !sid.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(invalid("endpoint.reality.short_id", format!("{sid:?}")));
            }
        }
        EndpointParams::Hysteria2 { server_name, password, obfs } => {
            non_empty("endpoint.server_name", server_name)?;
            non_empty("endpoint.password", password)?;
            if let Some(o) = obfs {
                if o.obfs_type != "salamander" {
                    return Err(invalid("endpoint.obfs.type", format!("{:?}", o.obfs_type)));
                }
                non_empty("endpoint.obfs.password", &o.password)?;
            }
        }
        EndpointParams::AmneziaWg(p) => {
            if !matches!(p.protocol.as_str(), "awg-3" | "awg-2-compatible") {
                return Err(invalid("endpoint.protocol", format!("{:?}", p.protocol)));
            }
            for (field, key) in [
                ("endpoint.server_public_key", &p.server_public_key),
                ("endpoint.client_private_key", &p.client_private_key),
                ("endpoint.preshared_key", &p.preshared_key),
            ] {
                if !is_wg_key(key) {
                    return Err(invalid(field, "not a 32-byte base64 key"));
                }
            }
            if p.server_public_key == p.client_private_key {
                return Err(invalid("endpoint.client_private_key", "equals the server public key"));
            }
            if p.addresses.is_empty() {
                return Err(ContractError::Empty { field: "endpoint.addresses" });
            }
            for a in &p.addresses {
                let (ip, prefix) = a.split_once('/').ok_or_else(|| invalid("endpoint.addresses", a.clone()))?;
                let ok = match ip.parse::<std::net::IpAddr>() {
                    Ok(std::net::IpAddr::V4(_)) => prefix == "32",
                    Ok(std::net::IpAddr::V6(_)) => prefix == "128",
                    Err(_) => false,
                };
                if !ok {
                    return Err(invalid("endpoint.addresses", a.clone()));
                }
            }
            if !(1280..=1500).contains(&p.recommended_mtu) {
                return Err(invalid("endpoint.recommended_mtu", "must be 1280..=1500"));
            }
            if p.jmin > p.jmax {
                return Err(invalid("endpoint.jmin", "must be <= jmax"));
            }
            for (field, r) in [("endpoint.h1", &p.h1), ("endpoint.h2", &p.h2), ("endpoint.h3", &p.h3), ("endpoint.h4", &p.h4)] {
                if !is_range(r) {
                    return Err(invalid(field, r.clone()));
                }
            }
            match (&p.header_protection_key, p.protocol.as_str()) {
                (Some(k), "awg-3") if is_wg_key(k) => {}
                (None, "awg-2-compatible") => {}
                _ => return Err(invalid("endpoint.header_protection_key", "does not match protocol")),
            }
            if p.signature_packets.len() > 5 {
                return Err(invalid("endpoint.signature_packets", "at most five"));
            }
        }
        EndpointParams::Unknown => {}
    }
    Ok(())
}

fn audit(value: &serde_json::Value, key: Option<&str>) -> Result<(), ContractError> {
    match value {
        serde_json::Value::Object(map) => {
            let is_awg = map.get("transport").and_then(|t| t.as_str()) == Some("amneziawg");
            for (k, v) in map {
                let lowered = k.to_ascii_lowercase();
                let allowed_private = is_awg && lowered == ALLOWED_PRIVATE_KEY_FIELD;
                if !allowed_private
                    && (FORBIDDEN_KEYS.contains(&lowered.as_str()) || lowered.contains("private"))
                {
                    return Err(ContractError::ForbiddenContent {
                        found: k.clone(),
                        reason: "client-owned policy or server-private material",
                    });
                }
                audit(v, Some(k))?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items.iter().try_for_each(|i| audit(i, key)),
        serde_json::Value::String(s) => {
            if let Some(f) = FORBIDDEN_VALUE_FRAGMENTS.iter().find(|f| s.contains(**f)) {
                return Err(ContractError::ForbiddenContent {
                    found: (*f).to_string(),
                    reason: "PEM material or a server filesystem path",
                });
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "yH6jKnw0cqg8x7dS3wA3m1F5y3wXk5a1v9QW2m8Zp1E=";
    const KEY_B: &str = "zI7kLox1drh9y8eT4xB4n2G6z4xYl6b2w0RX3n9aq2F=";
    const KEY_C: &str = "aJ8lMpy2esi0z9fU5yC5o3H7a5yZm7c3x1SY4o0br3G=";

    fn awg() -> AmneziaWgParams {
        AmneziaWgParams {
            protocol: "awg-3".into(),
            server_public_key: KEY_A.into(),
            client_private_key: KEY_B.into(),
            preshared_key: KEY_C.into(),
            addresses: vec!["10.66.0.2/32".into(), "fd66::2/128".into()],
            recommended_mtu: 1380,
            persistent_keepalive: 25,
            jc: 4,
            jmin: 40,
            jmax: 300,
            s1: 20,
            s2: 30,
            s3: 20,
            s4: 16,
            h1: "100000-200000".into(),
            h2: "300000-400000".into(),
            h3: "500000-600000".into(),
            h4: "700000-800000".into(),
            signature_packets: vec![],
            header_protection_key: Some(KEY_A.into()),
            content_padding_addition: Some("0-16".into()),
        }
    }

    pub(crate) fn doc() -> ProvisioningDocumentV2 {
        let mut d = ProvisioningDocumentV2::new(ServerInfo::current("1.1.0"));
        d.nodes = vec![
            Node { node_id: "fi1".into(), role: "exit".into(), status: "active".into(), failure_domain: "provider-a/fi".into(), provider: Some("provider-a".into()), region: None, country: Some("FI".into()), asn: None },
            Node { node_id: "ru1".into(), role: "relay".into(), status: "active".into(), failure_domain: "provider-c/ru".into(), provider: None, region: None, country: None, asn: None },
        ];
        d.endpoints = vec![
            Endpoint { endpoint_id: "fi1-awg".into(), node_id: "fi1".into(), host: "fi1.example.net".into(), port: 51820, params: EndpointParams::AmneziaWg(awg()) },
            Endpoint {
                endpoint_id: "fi1-reality".into(),
                node_id: "fi1".into(),
                host: "fi1.example.net".into(),
                port: 443,
                params: EndpointParams::VlessReality {
                    server_name: "www.example.com".into(),
                    uuid: "00000000-0000-4000-8000-000000000001".into(),
                    flow: Some(VLESS_FLOW_VISION.into()),
                    reality: RealityParams { public_key: "pk".into(), short_id: "0a1b".into(), fingerprint: "chrome".into() },
                },
            },
            Endpoint {
                endpoint_id: "ru1-reality".into(),
                node_id: "ru1".into(),
                host: "ru1.example.net".into(),
                port: 443,
                params: EndpointParams::VlessReality {
                    server_name: "www.example.org".into(),
                    uuid: "00000000-0000-4000-8000-000000000002".into(),
                    flow: Some(VLESS_FLOW_VISION.into()),
                    reality: RealityParams { public_key: "pk2".into(), short_id: "0c1d".into(), fingerprint: "chrome".into() },
                },
            },
        ];
        d.routes = vec![
            Route { route_id: "fi1/amneziawg".into(), label: "Finland · AmneziaWG".into(), kind: "direct".into(), hops: vec!["fi1-awg".into()], priority: 50 },
            Route { route_id: "fi1/reality".into(), label: "Finland · REALITY".into(), kind: "direct".into(), hops: vec!["fi1-reality".into()], priority: 50 },
            Route { route_id: "fi1/reality-via-ru1".into(), label: "Finland via Russia".into(), kind: "relayed".into(), hops: vec!["ru1-reality".into(), "fi1-reality".into()], priority: 20 },
        ];
        d
    }

    #[test]
    fn valid_document_round_trips() {
        let d = doc();
        let json = d.to_json().unwrap();
        assert_eq!(ProvisioningDocumentV2::from_json(&json).unwrap(), d);
        assert!(json.contains("\"schema_version\": 2"));
        assert!(json.contains("\"transport\": \"amneziawg\""));
        assert!(!json.contains("singbox_config"));
    }

    #[test]
    fn unknown_transports_parse_and_are_marked_unknown() {
        let json = doc().to_json().unwrap().replace("\"transport\": \"amneziawg\"", "\"transport\": \"future-proto\"");
        let parsed: ProvisioningDocumentV2 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.endpoints[0].params, EndpointParams::Unknown);
    }

    #[test]
    fn v1_documents_are_refused() {
        let mut d = doc();
        d.schema_version = 1;
        assert!(matches!(d.validate(), Err(ContractError::UnsupportedSchemaVersion { .. })));
    }

    #[test]
    fn structural_rules() {
        let broken = |f: &dyn Fn(&mut ProvisioningDocumentV2)| {
            let mut d = doc();
            f(&mut d);
            d.validate().is_err()
        };
        assert!(broken(&|d| d.nodes.push(d.nodes[0].clone())));
        assert!(broken(&|d| d.endpoints[0].node_id = "ghost".into()));
        assert!(broken(&|d| d.routes[0].hops = vec!["ghost".into()]));
        assert!(broken(&|d| d.routes[2].kind = "direct".into()));
        assert!(broken(&|d| d.routes[0].hops = vec!["fi1-awg".into(), "fi1-reality".into(), "ru1-reality".into()]));
        assert!(broken(&|d| d.routes.truncate(0)));
        assert!(broken(&|d| { d.routes.remove(0); }), "unused endpoint must be refused");
        assert!(broken(&|d| d.nodes[0].status = "retired".into()));
        assert!(broken(&|d| d.routes[0].priority = 101));
    }

    #[test]
    fn awg_parameter_rules() {
        let broken = |f: &dyn Fn(&mut AmneziaWgParams)| {
            let mut d = doc();
            if let EndpointParams::AmneziaWg(p) = &mut d.endpoints[0].params {
                f(p);
            }
            d.validate().is_err()
        };
        assert!(broken(&|p| p.client_private_key = "short".into()));
        assert!(broken(&|p| p.addresses = vec!["10.66.0.2/24".into()]));
        assert!(broken(&|p| p.addresses.clear()));
        assert!(broken(&|p| p.h1 = "9-1".into()));
        assert!(broken(&|p| p.header_protection_key = None));
        assert!(broken(&|p| p.protocol = "awg-1".into()));
        assert!(broken(&|p| p.recommended_mtu = 9000));
        assert!(broken(&|p| p.jmin = 400));
    }

    #[test]
    fn audit_refuses_policy_and_server_secrets_but_allows_the_client_awg_key() {
        doc().validate().unwrap();
        let mut v = serde_json::to_value(doc()).unwrap();
        v["endpoints"][1]["private_key"] = "x".into();
        assert!(audit(&v, None).is_err());
        let mut v = serde_json::to_value(doc()).unwrap();
        v["dns"] = serde_json::json!({"servers": []});
        assert!(audit(&v, None).is_err());
        let mut v = serde_json::to_value(doc()).unwrap();
        v["endpoints"][1]["client_private_key"] = KEY_B.into();
        assert!(audit(&v, None).is_err(), "only AWG endpoints may carry client_private_key");
        let mut v = serde_json::to_value(doc()).unwrap();
        v["nodes"][0]["provider"] = "/etc/vpn/compat".into();
        assert!(audit(&v, None).is_err());
    }
}
