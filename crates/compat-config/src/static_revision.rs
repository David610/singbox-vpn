//! Static-config revisions: the controlled, versioned way for the control
//! plane to change a small, explicitly allowlisted set of `deployment.toml`
//! fields on an already-deployed node (delivered through the existing
//! `APPLY_NODE_REVISION` job / `GET /api/agent/revision/:revision` fetch,
//! then `vpn-admin apply-revision`).
//!
//! Wire shape (the revision's `config` document):
//!
//! ```json
//! {
//!   "revision_schema": 1,
//!   "static_config": {
//!     "reality":   { "handshake_server": "www.example.com" },
//!     "hysteria2": { "up_mbps": 200, "down_mbps": 200 },
//!     "udp_probe": { "ipv4_resolvers": ["1.1.1.1"], "ipv6_resolvers": [],
//!                    "retries": 2, "timeout_ms": 2000, "delay_ms": 250 }
//!   }
//! }
//! ```
//!
//! Every section and every field is optional, but at least one field must
//! be present. A field that is present is the desired value; a field that
//! is absent is left exactly as it is on the node. `hysteria2.up_mbps`/
//! `down_mbps` must be sent together (both numbers, or both `null` to clear).
//!
//! The document is deliberately NOT a `users.json`-shaped document: an older
//! `vpn-admin` that predates this module fails to parse it (its users-store
//! parser requires `schema_version` + a `users` array) and so rejects the
//! whole revision, instead of silently applying nothing and reporting
//! success.
//!
//! Security policy (fail closed, whole-document):
//! * An unknown top-level key, an unknown `static_config` section, or an
//!   unknown field inside an allowed section rejects the ENTIRE revision.
//!   Nothing is silently dropped.
//! * Identity, role, trust-root, install-root and public-network-identity
//!   fields are FORBIDDEN here (see [`FORBIDDEN_TOP_LEVEL_FIELDS`]). Changing
//!   them needs a separate, privileged reprovision path: C-16 egress policy,
//!   reserved-probe confinement and relay forwarding policy are all derived
//!   from them at render time.
//! * Listener ports and the REALITY handshake port are recognized but not
//!   routinely revisable in schema 1 (see [`DEFERRED_SECTION_FIELDS`]): a
//!   port move also needs host-firewall, relay-peer forwarding and client
//!   profile changes that this node-local operation cannot perform or
//!   verify atomically.

use crate::deployment::DeploymentConfig;
use crate::CompatError;
use serde_json::Value;

/// The only `revision_schema` this binary understands.
pub const STATIC_REVISION_SCHEMA: u64 = 1;

/// Upper bound on the size of a static revision document. The largest
/// legitimate document (every allowlisted field, eight resolvers per family)
/// is well under 2 KiB.
pub const MAX_STATIC_REVISION_BYTES: usize = 16 * 1024;

/// Allowlist: `(section, field)` pairs a routine static revision may set.
pub const ALLOWED_FIELDS: &[(&str, &str)] = &[
    ("reality", "handshake_server"),
    ("hysteria2", "up_mbps"),
    ("hysteria2", "down_mbps"),
    ("udp_probe", "ipv4_resolvers"),
    ("udp_probe", "ipv6_resolvers"),
    ("udp_probe", "retries"),
    ("udp_probe", "timeout_ms"),
    ("udp_probe", "delay_ms"),
];

/// Top-level `deployment.toml` keys that must never be changed through the
/// routine revision path.
pub const FORBIDDEN_TOP_LEVEL_FIELDS: &[&str] = &[
    "schema_version",
    "node_id",
    "role",
    "public_host",
    "subscription_host",
    "public_ipv4",
    "public_ipv6",
    "state_dir",
    "singbox_binary",
    "access_paths",
    "peer_endpoints",
    "google_egress_hairpin",
    "subscription",
];

/// Recognized static fields that schema 1 still refuses (see module docs).
pub const DEFERRED_SECTION_FIELDS: &[(&str, &str)] = &[
    ("reality", "listen_port"),
    ("reality", "handshake_port"),
    ("hysteria2", "listen_port"),
];

const MAX_RESOLVERS_PER_FAMILY: usize = 8;
const MAX_MBPS: u64 = 100_000;

/// A parsed, fully validated static revision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticRevision {
    pub handshake_server: Option<String>,
    /// `Some(Some((up, down)))` sets both, `Some(None)` clears both.
    pub hysteria2_mbps: Option<Option<(u32, u32)>>,
    pub udp_ipv4_resolvers: Option<Vec<String>>,
    pub udp_ipv6_resolvers: Option<Vec<String>>,
    pub udp_retries: Option<u64>,
    pub udp_timeout_ms: Option<u64>,
    pub udp_delay_ms: Option<u64>,
}

impl StaticRevision {
    /// Whether this revision can change what `vpn-subscription` serves to
    /// clients (it caches its endpoint set at startup, so such a change
    /// requires a restart of that service too).
    pub fn touches_served_endpoints(&self) -> bool {
        self.handshake_server.is_some() || self.hysteria2_mbps.is_some()
    }
}

fn reject(msg: impl Into<String>) -> CompatError {
    CompatError::ConfigValidationFailed(format!("static revision rejected: {}", msg.into()))
}

/// Cheap routing check: is this revision document a static revision
/// envelope (as opposed to a `users.json`-shaped dynamic revision)?
pub fn is_static_revision_document(doc: &Value) -> bool {
    doc.as_object()
        .is_some_and(|o| o.contains_key("revision_schema") || o.contains_key("static_config"))
}

/// Parse and fully validate a static revision document. Fails closed on
/// anything not explicitly allowlisted.
pub fn parse_static_revision(bytes: &[u8]) -> Result<StaticRevision, CompatError> {
    if bytes.len() > MAX_STATIC_REVISION_BYTES {
        return Err(reject(format!(
            "document is {} bytes, larger than the {MAX_STATIC_REVISION_BYTES}-byte limit",
            bytes.len()
        )));
    }
    let doc: Value =
        serde_json::from_slice(bytes).map_err(|e| reject(format!("not valid JSON ({e})")))?;
    let top = doc
        .as_object()
        .ok_or_else(|| reject("document must be a JSON object"))?;

    for key in top.keys() {
        if key != "revision_schema" && key != "static_config" {
            return Err(reject(format!(
                "unknown top-level key {key:?} (only \"revision_schema\" and \"static_config\" \
                 are allowed)"
            )));
        }
    }
    match top.get("revision_schema").and_then(Value::as_u64) {
        Some(STATIC_REVISION_SCHEMA) => {}
        Some(other) => {
            return Err(reject(format!(
                "unsupported revision_schema {other} (this vpn-admin supports only \
                 {STATIC_REVISION_SCHEMA})"
            )))
        }
        None => return Err(reject("missing or non-integer revision_schema")),
    }
    let static_config = top
        .get("static_config")
        .and_then(Value::as_object)
        .ok_or_else(|| reject("missing static_config object"))?;
    if static_config.is_empty() {
        return Err(reject("static_config is empty"));
    }

    for (section, body) in static_config {
        if FORBIDDEN_TOP_LEVEL_FIELDS.contains(&section.as_str()) {
            return Err(reject(format!(
                "{section:?} is a security-sensitive field (node identity, role, trust root, \
                 install paths or public network identity) and cannot be changed by a routine \
                 static revision; it requires a privileged reprovision"
            )));
        }
        if !ALLOWED_FIELDS.iter().any(|(s, _)| s == section) {
            return Err(reject(format!("unknown static_config section {section:?}")));
        }
        let fields = body
            .as_object()
            .ok_or_else(|| reject(format!("static_config.{section} must be an object")))?;
        if fields.is_empty() {
            return Err(reject(format!("static_config.{section} is empty")));
        }
        for field in fields.keys() {
            if DEFERRED_SECTION_FIELDS
                .iter()
                .any(|(s, f)| s == section && f == field)
            {
                return Err(reject(format!(
                    "{section}.{field} is not routinely revisable in revision_schema 1 (a port \
                     change also requires host-firewall, relay-forwarding and client-profile \
                     changes this node cannot apply atomically)"
                )));
            }
            if !ALLOWED_FIELDS
                .iter()
                .any(|(s, f)| s == section && f == field)
            {
                return Err(reject(format!("unknown field {section}.{field}")));
            }
        }
    }

    let mut rev = StaticRevision::default();

    if let Some(reality) = static_config.get("reality").and_then(Value::as_object) {
        if let Some(v) = reality.get("handshake_server") {
            let host = v
                .as_str()
                .ok_or_else(|| reject("reality.handshake_server must be a string"))?;
            validate_handshake_server(host)?;
            rev.handshake_server = Some(host.to_string());
        }
    }

    if let Some(hy) = static_config.get("hysteria2").and_then(Value::as_object) {
        let up = hy.get("up_mbps");
        let down = hy.get("down_mbps");
        rev.hysteria2_mbps = match (up, down) {
            (None, None) => None,
            (Some(Value::Null), Some(Value::Null)) => Some(None),
            (Some(u), Some(d)) => Some(Some((mbps(u, "up_mbps")?, mbps(d, "down_mbps")?))),
            _ => {
                return Err(reject(
                    "hysteria2.up_mbps and hysteria2.down_mbps must be sent together (both \
                     numbers, or both null to clear)",
                ))
            }
        };
    }

    if let Some(udp) = static_config.get("udp_probe").and_then(Value::as_object) {
        if let Some(v) = udp.get("ipv4_resolvers") {
            rev.udp_ipv4_resolvers = Some(resolvers(v, "ipv4_resolvers", false)?);
        }
        if let Some(v) = udp.get("ipv6_resolvers") {
            rev.udp_ipv6_resolvers = Some(resolvers(v, "ipv6_resolvers", true)?);
        }
        if let Some(v) = udp.get("retries") {
            rev.udp_retries = Some(bounded(v, "udp_probe.retries", 1, 10)?);
        }
        if let Some(v) = udp.get("timeout_ms") {
            rev.udp_timeout_ms = Some(bounded(v, "udp_probe.timeout_ms", 100, 30_000)?);
        }
        if let Some(v) = udp.get("delay_ms") {
            rev.udp_delay_ms = Some(bounded(v, "udp_probe.delay_ms", 0, 10_000)?);
        }
    }

    Ok(rev)
}

fn bounded(v: &Value, name: &str, min: u64, max: u64) -> Result<u64, CompatError> {
    match v.as_u64() {
        Some(n) if (min..=max).contains(&n) => Ok(n),
        _ => Err(reject(format!(
            "{name} must be an integer in {min}..={max}, got {v}"
        ))),
    }
}

fn mbps(v: &Value, name: &str) -> Result<u32, CompatError> {
    bounded(v, &format!("hysteria2.{name}"), 1, MAX_MBPS).map(|n| n as u32)
}

fn resolvers(v: &Value, name: &str, ipv6: bool) -> Result<Vec<String>, CompatError> {
    let items = v
        .as_array()
        .ok_or_else(|| reject(format!("udp_probe.{name} must be an array of IP literals")))?;
    if items.len() > MAX_RESOLVERS_PER_FAMILY || (!ipv6 && items.is_empty()) {
        return Err(reject(format!(
            "udp_probe.{name} must have {}..={MAX_RESOLVERS_PER_FAMILY} entries",
            if ipv6 { 0 } else { 1 }
        )));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let s = item
            .as_str()
            .ok_or_else(|| reject(format!("udp_probe.{name} entries must be strings")))?;
        let ip: std::net::IpAddr = s
            .parse()
            .map_err(|_| reject(format!("udp_probe.{name}: {s:?} is not an IP literal")))?;
        if ip.is_ipv6() != ipv6 {
            return Err(reject(format!(
                "udp_probe.{name}: {s:?} is the wrong address family"
            )));
        }
        if !is_public_resolver_address(ip) {
            return Err(reject(format!(
                "udp_probe.{name}: {s:?} is not a public unicast address (loopback, private, \
                 link-local, unspecified, multicast and similar are refused)"
            )));
        }
        if out.contains(&ip.to_string()) {
            return Err(reject(format!("udp_probe.{name}: duplicate entry {s:?}")));
        }
        out.push(ip.to_string());
    }
    Ok(out)
}

fn is_public_resolver_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || o[0] >= 240)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_resolver_address(std::net::IpAddr::V4(mapped));
            }
            let seg0 = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg0 & 0xfe00) == 0xfc00
                || (seg0 & 0xffc0) == 0xfe80
                || seg0 == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

/// The REALITY handshake server receives every unauthenticated connection
/// the node forwards as camouflage. It must therefore be an ordinary public
/// DNS name: IP literals, single-label names and local/internal suffixes
/// are refused so a revision cannot point that forwarding at the node's
/// own or its provider's internal services.
pub fn validate_handshake_server(host: &str) -> Result<(), CompatError> {
    let bad = |why: &str| reject(format!("reality.handshake_server {host:?} {why}"));
    if host.is_empty() || host.len() > 253 {
        return Err(bad("must be 1..=253 characters"));
    }
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return Err(bad("must be a public DNS name, not an IP literal"));
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return Err(bad("must be a fully-qualified DNS name"));
    }
    for label in &labels {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(bad("is not a lowercase DNS name"));
        }
    }
    let tld = labels[labels.len() - 1];
    if tld.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad("must be a public DNS name, not an IP literal"));
    }
    const LOCAL_SUFFIXES: &[&str] = &[
        "localhost",
        "local",
        "internal",
        "lan",
        "home",
        "corp",
        "intranet",
        "localdomain",
        "arpa",
        "test",
        "invalid",
        "example",
    ];
    if LOCAL_SUFFIXES.contains(&tld) {
        return Err(bad("uses a local/reserved suffix"));
    }
    Ok(())
}

/// Outcome of planning a static revision against the node's current
/// `deployment.toml` text.
#[derive(Debug)]
pub enum StaticRevisionPlan {
    /// Every requested value already matches: nothing to write or reload.
    Unchanged,
    Changed {
        /// The full candidate `deployment.toml` text to stage.
        candidate_text: String,
        /// The candidate, already parsed and `validate()`d.
        candidate: Box<DeploymentConfig>,
    },
}

/// Compute the candidate `deployment.toml` for `rev` without touching disk.
///
/// Refuses (with the original untouched) if the current file does not parse
/// or validate, if the candidate does not validate, or if the candidate
/// differs from the original anywhere other than the allowlisted fields —
/// the last check is a structural guarantee that role, identity, trust root
/// and every other forbidden field are byte-for-byte preserved regardless
/// of any bug in the patching code above it.
pub fn plan_static_revision(
    original_text: &str,
    rev: &StaticRevision,
) -> Result<StaticRevisionPlan, CompatError> {
    let original: DeploymentConfig = toml::from_str(original_text).map_err(|e| {
        CompatError::Parse(format!(
            "current deployment.toml does not parse ({e}); refusing static revision"
        ))
    })?;
    original.validate()?;

    let mut table: toml::Table = toml::from_str(original_text)
        .map_err(|e| CompatError::Parse(format!("current deployment.toml: {e}")))?;

    if let Some(host) = &rev.handshake_server {
        section_mut(&mut table, "reality")?
            .insert("handshake_server".into(), toml::Value::String(host.clone()));
    }
    if let Some(mbps) = rev.hysteria2_mbps {
        let hy = section_mut(&mut table, "hysteria2")?;
        match mbps {
            Some((up, down)) => {
                hy.insert("up_mbps".into(), toml::Value::Integer(i64::from(up)));
                hy.insert("down_mbps".into(), toml::Value::Integer(i64::from(down)));
            }
            None => {
                hy.remove("up_mbps");
                hy.remove("down_mbps");
            }
        }
    }
    let touches_udp = rev.udp_ipv4_resolvers.is_some()
        || rev.udp_ipv6_resolvers.is_some()
        || rev.udp_retries.is_some()
        || rev.udp_timeout_ms.is_some()
        || rev.udp_delay_ms.is_some();
    if touches_udp {
        let udp = section_mut(&mut table, "udp_probe")?;
        let strings = |v: &Vec<String>| {
            toml::Value::Array(v.iter().cloned().map(toml::Value::String).collect())
        };
        if let Some(v) = &rev.udp_ipv4_resolvers {
            udp.insert("ipv4_resolvers".into(), strings(v));
        }
        if let Some(v) = &rev.udp_ipv6_resolvers {
            udp.insert("ipv6_resolvers".into(), strings(v));
        }
        for (key, value) in [
            ("retries", rev.udp_retries),
            ("timeout_ms", rev.udp_timeout_ms),
            ("delay_ms", rev.udp_delay_ms),
        ] {
            if let Some(n) = value {
                let n = i64::try_from(n).map_err(|_| reject(format!("{key} out of range")))?;
                udp.insert(key.into(), toml::Value::Integer(n));
            }
        }
    }

    let candidate_text = toml::to_string(&table)
        .map_err(|e| CompatError::Parse(format!("serializing candidate deployment.toml: {e}")))?;
    let candidate: DeploymentConfig = toml::from_str(&candidate_text).map_err(|e| {
        CompatError::Parse(format!(
            "candidate deployment.toml failed to reparse ({e}) — this is a bug, not applying"
        ))
    })?;
    candidate.validate()?;

    let original_json = normalized(&original)?;
    let candidate_json = normalized(&candidate)?;
    if original_json == candidate_json {
        return Ok(StaticRevisionPlan::Unchanged);
    }
    if strip_allowlisted(original_json) != strip_allowlisted(candidate_json) {
        return Err(CompatError::ConfigValidationFailed(
            "static revision would change a field outside the allowlist — refusing (this is a \
             bug in the static-revision patcher)"
                .into(),
        ));
    }
    Ok(StaticRevisionPlan::Changed {
        candidate_text,
        candidate: Box::new(candidate),
    })
}

fn section_mut<'a>(
    table: &'a mut toml::Table,
    name: &str,
) -> Result<&'a mut toml::Table, CompatError> {
    table
        .entry(name.to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| CompatError::Parse(format!("deployment.toml [{name}] is not a table")))
}

fn normalized(cfg: &DeploymentConfig) -> Result<Value, CompatError> {
    serde_json::to_value(cfg).map_err(|e| CompatError::Parse(e.to_string()))
}

fn strip_allowlisted(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        for (section, field) in ALLOWED_FIELDS {
            if let Some(sec) = obj.get_mut(*section) {
                if sec.is_null() {
                    // An absent optional section (e.g. `udp_probe = None`)
                    // becoming present is itself an allowlisted change.
                    obj.remove(*section);
                    continue;
                }
                if let Some(sec) = sec.as_object_mut() {
                    sec.remove(*field);
                }
            }
        }
        if obj
            .get("udp_probe")
            .and_then(Value::as_object)
            .is_some_and(|o| o.is_empty())
        {
            obj.remove("udp_probe");
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
schema_version = 2
node_id = "de-fra-1"
role = "exit"
public_host = "vpn.example.com"
subscription_host = "sub.example.com"

[reality]
listen_port = 443
handshake_server = "www.google.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100
"#;

    fn doc(v: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&v).unwrap()
    }

    fn parse(v: serde_json::Value) -> Result<StaticRevision, CompatError> {
        parse_static_revision(&doc(v))
    }

    #[test]
    fn accepts_every_allowlisted_field() {
        let rev = parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {
                "reality": {"handshake_server": "www.microsoft.com"},
                "hysteria2": {"up_mbps": 200, "down_mbps": 300},
                "udp_probe": {"ipv4_resolvers": ["9.9.9.9"], "ipv6_resolvers": [],
                              "retries": 3, "timeout_ms": 1500, "delay_ms": 0}
            }
        }))
        .unwrap();
        assert_eq!(rev.handshake_server.as_deref(), Some("www.microsoft.com"));
        assert_eq!(rev.hysteria2_mbps, Some(Some((200, 300))));
        assert_eq!(rev.udp_ipv4_resolvers, Some(vec!["9.9.9.9".to_string()]));
        assert_eq!(rev.udp_ipv6_resolvers, Some(vec![]));
        assert!(rev.touches_served_endpoints());
    }

    #[test]
    fn rejects_every_forbidden_field() {
        for field in FORBIDDEN_TOP_LEVEL_FIELDS {
            let err = parse(serde_json::json!({
                "revision_schema": 1,
                "static_config": { *field: "relay" }
            }))
            .unwrap_err()
            .to_string();
            assert!(err.contains("security-sensitive"), "{field}: {err}");
        }
    }

    #[test]
    fn rejects_deferred_port_fields() {
        for (section, field) in DEFERRED_SECTION_FIELDS {
            let err = parse(serde_json::json!({
                "revision_schema": 1,
                "static_config": { *section: { *field: 8443 } }
            }))
            .unwrap_err()
            .to_string();
            assert!(err.contains("not routinely revisable"), "{err}");
        }
    }

    #[test]
    fn unknown_field_rejects_the_whole_document_not_just_the_field() {
        let err = parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {"udp_probe": {"retries": 3, "exfiltrate": true}}
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("unknown field udp_probe.exfiltrate"), "{err}");
        assert!(parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {"firewall": {"open": true}}
        }))
        .is_err());
        assert!(parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {"udp_probe": {"retries": 3}},
            "users": []
        }))
        .is_err());
    }

    #[test]
    fn rejects_unknown_or_missing_schema_and_oversized_documents() {
        for schema in [
            serde_json::json!(2),
            serde_json::json!(0),
            serde_json::json!("1"),
        ] {
            assert!(parse(serde_json::json!({
                "revision_schema": schema,
                "static_config": {"udp_probe": {"retries": 3}}
            }))
            .is_err());
        }
        assert!(
            parse(serde_json::json!({"static_config": {"udp_probe": {"retries": 3}}})).is_err()
        );
        let big = vec![b' '; MAX_STATIC_REVISION_BYTES + 1];
        assert!(parse_static_revision(&big)
            .unwrap_err()
            .to_string()
            .contains("limit"));
    }

    #[test]
    fn rejects_invalid_values() {
        let bad = [
            serde_json::json!({"hysteria2": {"up_mbps": 100}}),
            serde_json::json!({"hysteria2": {"up_mbps": 0, "down_mbps": 10}}),
            serde_json::json!({"hysteria2": {"up_mbps": null, "down_mbps": 10}}),
            serde_json::json!({"udp_probe": {"retries": 0}}),
            serde_json::json!({"udp_probe": {"timeout_ms": 999999}}),
            serde_json::json!({"udp_probe": {"ipv4_resolvers": []}}),
            serde_json::json!({"udp_probe": {"ipv4_resolvers": ["127.0.0.1"]}}),
            serde_json::json!({"udp_probe": {"ipv4_resolvers": ["169.254.169.254"]}}),
            serde_json::json!({"udp_probe": {"ipv4_resolvers": ["10.0.0.1"]}}),
            serde_json::json!({"udp_probe": {"ipv4_resolvers": ["2606:4700:4700::1111"]}}),
            serde_json::json!({"udp_probe": {"ipv6_resolvers": ["::1"]}}),
            serde_json::json!({"udp_probe": {"ipv6_resolvers": ["::ffff:10.0.0.1"]}}),
            serde_json::json!({"udp_probe": {"ipv4_resolvers": ["1.1.1.1", "1.1.1.1"]}}),
            serde_json::json!({"reality": {"handshake_server": "127.0.0.1"}}),
            serde_json::json!({"reality": {"handshake_server": "localhost"}}),
            serde_json::json!({"reality": {"handshake_server": "metadata.google.internal"}}),
            serde_json::json!({"reality": {"handshake_server": "WWW.GOOGLE.COM"}}),
            serde_json::json!({"reality": {"handshake_server": 7}}),
            serde_json::json!({}),
            serde_json::json!({"udp_probe": {}}),
        ];
        for static_config in bad {
            assert!(
                parse(serde_json::json!({"revision_schema": 1, "static_config": static_config.clone()}))
                    .is_err(),
                "{static_config} must be rejected"
            );
        }
    }

    #[test]
    fn plan_changes_only_allowlisted_fields_and_preserves_role_and_identity() {
        let rev = parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {
                "hysteria2": {"up_mbps": 200, "down_mbps": 300},
                "udp_probe": {"ipv4_resolvers": ["9.9.9.9"]}
            }
        }))
        .unwrap();
        let StaticRevisionPlan::Changed { candidate, .. } =
            plan_static_revision(BASE, &rev).unwrap()
        else {
            panic!("expected a change");
        };
        assert_eq!(candidate.role.as_str(), "exit");
        assert_eq!(candidate.node_id, "de-fra-1");
        assert_eq!(candidate.public_host, "vpn.example.com");
        assert_eq!(candidate.hysteria2.up_mbps, Some(200));
        assert_eq!(candidate.hysteria2.down_mbps, Some(300));
        assert_eq!(candidate.hysteria2.listen_port, 443);
        let udp = candidate.udp_probe.unwrap();
        assert_eq!(udp.ipv4_resolvers, vec!["9.9.9.9".to_string()]);
        assert_eq!(
            udp.retries, 2,
            "untouched udp_probe fields keep their defaults"
        );
    }

    #[test]
    fn plan_is_unchanged_when_values_already_match() {
        let rev = parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {"reality": {"handshake_server": "www.google.com"}}
        }))
        .unwrap();
        assert!(matches!(
            plan_static_revision(BASE, &rev).unwrap(),
            StaticRevisionPlan::Unchanged
        ));
    }

    #[test]
    fn plan_clears_hysteria2_bandwidth_with_nulls() {
        let base = BASE.replace(
            "[hysteria2]\nlisten_port = 443\n",
            "[hysteria2]\nlisten_port = 443\nup_mbps = 50\ndown_mbps = 60\n",
        );
        assert!(base.contains("up_mbps = 50"));
        let rev = parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {"hysteria2": {"up_mbps": null, "down_mbps": null}}
        }))
        .unwrap();
        let StaticRevisionPlan::Changed { candidate, .. } =
            plan_static_revision(&base, &rev).unwrap()
        else {
            panic!("expected a change");
        };
        assert_eq!(candidate.hysteria2.up_mbps, None);
        assert_eq!(candidate.hysteria2.down_mbps, None);
    }

    #[test]
    fn plan_refuses_an_invalid_current_file() {
        let rev = parse(serde_json::json!({
            "revision_schema": 1,
            "static_config": {"udp_probe": {"retries": 3}}
        }))
        .unwrap();
        assert!(plan_static_revision("not = [valid", &rev).is_err());
    }

    #[test]
    fn users_document_is_not_routed_as_static() {
        assert!(!is_static_revision_document(
            &serde_json::json!({"schema_version": 1, "users": []})
        ));
        assert!(!is_static_revision_document(&serde_json::json!([])));
        assert!(is_static_revision_document(
            &serde_json::json!({"revision_schema": 1})
        ));
    }
}
