//! The versioned, first-party provisioning contract between this server
//! (`singbox-vpn`) and its primary client, `singbox-client`
//! (<https://github.com/David610/singbox-client>).
//!
//! This crate is the SINGLE canonical model of "what a client needs in
//! order to dial this deployment". Every client-facing artifact the
//! server produces — the first-party JSON contract, the
//! Hiddify-compatible `vless://`/`hysteria2://` share links, and the
//! native sing-box subscription JSON — is rendered FROM this model (see
//! `compat_config::contract` and `compat_config::render`). Credential
//! shaping lives here and nowhere else; a renderer that re-derives an
//! endpoint's credentials from somewhere else is a bug.
//!
//! # What this document deliberately does NOT contain
//!
//! The contract is *server-owned facts only*: which transports exist,
//! where they listen, and the material needed to authenticate to them.
//! Everything about how the client behaves on the device belongs to the
//! client and is structurally absent from these types:
//!
//! * DNS policy, DNS servers, DoH/DoT choices
//! * MTU, TUN/`auto_route`/`strict_route`, kill switch
//! * IPv4/IPv6 preference or family policy
//! * mobile lifecycle (background/idle/handover behaviour)
//! * `insecure` / certificate-verification opt-outs
//!
//! There is no field of any of these types that can hold such a value,
//! and [`ProvisioningDocument::validate`] additionally audits the
//! serialized form for them so a future field cannot reintroduce one
//! silently. The same audit rejects server-private material (REALITY
//! private key, TLS private key, any filesystem path).
//!
//! # Versioning rules
//!
//! * `schema_version` is an integer, always present, currently
//!   [`SCHEMA_VERSION`].
//! * **Adding** a capability value or an OPTIONAL endpoint field is a
//!   compatible change: `schema_version` stays the same. Clients MUST
//!   ignore capability values and endpoint transports they do not
//!   recognise (see [`Capability::Other`] / [`Transport::Other`]) rather
//!   than failing the whole document.
//! * **Removing or renaming** a capability value, a field, or changing
//!   the meaning of an existing field requires a `schema_version` bump.
//! * A server that is asked for a `schema_version` it does not implement
//!   MUST fail explicitly (see [`UnsupportedSchemaVersion`]) — never
//!   silently serve a different version.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The schema version this server implements and emits.
pub const SCHEMA_VERSION: u32 = 1;

/// Every schema version this server can serve. A request for anything
/// outside this list is an explicit error, never a best-effort guess.
pub const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[SCHEMA_VERSION];

/// Product identifier carried in every document. Clients may use it to
/// refuse documents that plainly did not come from this server.
pub const PRODUCT: &str = "singbox-vpn";

/// Oldest `singbox-client` release able to parse `schema_version = 1`.
pub const MINIMUM_CLIENT_VERSION: &str = "0.1.0";

/// The only VLESS flow this server ever provisions in production.
pub const VLESS_FLOW_VISION: &str = "xtls-rprx-vision";

/// Returns `true` when this server can serve the requested schema
/// version.
pub fn is_supported_schema_version(v: u32) -> bool {
    SUPPORTED_SCHEMA_VERSIONS.contains(&v)
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// A client asked for a schema version this server does not implement.
/// This is a hard, explicit failure by design: silently reinterpreting
/// the request as some other version is how contract drift becomes
/// undebuggable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedSchemaVersion {
    /// Always `"unsupported_schema_version"` — a stable machine-readable
    /// discriminator so clients need not string-match `message`.
    pub error: String,
    pub requested: u32,
    pub supported: Vec<u32>,
    pub message: String,
}

impl UnsupportedSchemaVersion {
    pub fn new(requested: u32) -> Self {
        Self {
            error: "unsupported_schema_version".to_string(),
            requested,
            supported: SUPPORTED_SCHEMA_VERSIONS.to_vec(),
            message: format!(
                "this server implements provisioning schema_version {:?}; it cannot serve the \
                 requested version {requested}. Upgrade the server, or request a supported \
                 version — the server will not guess.",
                SUPPORTED_SCHEMA_VERSIONS
            ),
        }
    }
}

impl std::fmt::Display for UnsupportedSchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UnsupportedSchemaVersion {}

/// Everything that can make a provisioning document invalid. Each
/// variant names the exact field so an operator reading a log line can
/// act on it without reading this source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error(
        "unsupported schema_version {requested} (this server implements {supported:?}) — \
         refusing to emit or accept a document whose semantics it does not know"
    )]
    UnsupportedSchemaVersion { requested: u32, supported: Vec<u32> },

    #[error("{field} must not be empty")]
    Empty { field: &'static str },

    #[error("{field} contains an invalid value: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error(
        "endpoint {endpoint_id}: malformed VLESS client id (uuid) — expected an \
         8-4-4-4-12 hex UUID, got {got:?}"
    )]
    MalformedUuid { endpoint_id: String, got: String },

    #[error(
        "endpoint {endpoint_id}: transport vless-reality requires a REALITY {missing}; it is \
         missing or empty, so no client could ever complete a handshake with this endpoint"
    )]
    MissingRealityParameter {
        endpoint_id: String,
        missing: &'static str,
    },

    #[error("endpoint {endpoint_id}: hysteria2 auth password must not be empty")]
    EmptyHysteria2Password { endpoint_id: String },

    #[error(
        "endpoint {endpoint_id}: hysteria2 obfuscation is present but its password is empty — \
         a salamander obfuscator with no password is not a valid configuration"
    )]
    EmptyObfsPassword { endpoint_id: String },

    #[error(
        "capability {capability:?} is advertised but no endpoint provides it — a client would \
         negotiate a transport this server cannot actually serve"
    )]
    CapabilityWithoutEndpoint { capability: String },

    #[error(
        "endpoint {endpoint_id} provides transport {transport:?} but that capability is not \
         advertised — the capability list must reflect what is actually configured"
    )]
    EndpointWithoutCapability {
        endpoint_id: String,
        transport: String,
    },

    #[error("duplicate endpoint id {id:?}")]
    DuplicateEndpointId { id: String },

    #[error(
        "experimental capability {capability:?} appears in `capabilities` — diagnostic and \
         experimental capabilities belong in `experimental_capabilities` and must never take \
         part in normal production capability negotiation"
    )]
    ExperimentalInProductionCapabilities { capability: String },

    #[error(
        "experimental capability {capability:?} must be named with the `{prefix}` prefix so no \
         consumer can mistake it for a production capability"
    )]
    ExperimentalCapabilityMisnamed {
        capability: String,
        prefix: &'static str,
    },

    #[error(
        "the serialized document contains the forbidden key or value {found:?} ({reason}) — \
         this contract must never carry server-private material or client-owned policy"
    )]
    ForbiddenContent { found: String, reason: &'static str },
}

// ---------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------

/// Prefix every experimental/diagnostic capability name must carry.
pub const EXPERIMENTAL_CAPABILITY_PREFIX: &str = "diag-";

/// A production capability: a transport this deployment can actually
/// serve right now, derived from real configuration, never aspirational.
///
/// Serialized as its kebab-case wire name. Unknown values round-trip
/// through [`Capability::Other`] so a `schema_version = 1` consumer
/// written today keeps working against a future server that advertises a
/// capability it has never heard of (see the crate-level versioning
/// rules).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    VlessReality,
    Hysteria2,
    /// An unrecognised capability from a newer server. Clients must skip
    /// these, not reject the document.
    Other(String),
}

impl Capability {
    pub fn as_str(&self) -> &str {
        match self {
            Capability::VlessReality => "vless-reality",
            Capability::Hysteria2 => "hysteria2",
            Capability::Other(s) => s.as_str(),
        }
    }

    pub fn from_wire(s: &str) -> Self {
        match s {
            "vless-reality" => Capability::VlessReality,
            "hysteria2" => Capability::Hysteria2,
            other => Capability::Other(other.to_string()),
        }
    }

    /// The capability a given transport satisfies.
    pub fn for_transport(t: &Transport) -> Self {
        match t {
            Transport::VlessReality => Capability::VlessReality,
            Transport::Hysteria2 => Capability::Hysteria2,
            Transport::Other(s) => Capability::Other(s.clone()),
        }
    }
}

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Capability::from_wire(&String::deserialize(d)?))
    }
}

/// A diagnostic/experimental capability. Deliberately a SEPARATE type
/// and a separate document field from [`Capability`]: these never take
/// part in normal production capability negotiation, are never defaults,
/// and always carry the `diag-` prefix so nothing can mistake one for a
/// production transport.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExperimentalCapability(pub String);

impl ExperimentalCapability {
    /// `diag-tcp-only`: the operator-triggered profile that drops every
    /// UDP-carrying option (see `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`).
    pub fn tcp_only() -> Self {
        Self("diag-tcp-only".to_string())
    }

    /// `diag-vision-off`: the per-user opt-in profile that omits the
    /// XTLS Vision flow (see `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md`).
    /// More fingerprintable than production; diagnostic only.
    pub fn vision_off() -> Self {
        Self("diag-vision-off".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------

/// The wire transport of an endpoint. Same forward-compatibility rule as
/// [`Capability`]: an unknown transport deserializes into
/// [`Transport::Other`] and must be skipped by the client, not treated
/// as a parse failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    VlessReality,
    Hysteria2,
    Other(String),
}

impl Transport {
    pub fn as_str(&self) -> &str {
        match self {
            Transport::VlessReality => "vless-reality",
            Transport::Hysteria2 => "hysteria2",
            Transport::Other(s) => s.as_str(),
        }
    }
}

/// Public REALITY parameters. There is intentionally no field here that
/// could hold the server's REALITY *private* key — that value lives only
/// in `compat_config::model::RealityServerParams`, which this crate does
/// not and must not depend on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealityParams {
    /// REALITY public key (`pbk` in share-link syntax).
    pub public_key: String,
    /// REALITY short id (`sid`).
    pub short_id: String,
    /// uTLS ClientHello fingerprint to imitate (`fp`), e.g. `chrome`.
    pub fingerprint: String,
}

/// Hysteria2 Salamander obfuscation, present only when the deployment
/// actually configured it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hysteria2Obfs {
    /// Always `"salamander"` — the only obfuscator this server configures.
    #[serde(rename = "type")]
    pub obfs_type: String,
    pub password: String,
}

impl Hysteria2Obfs {
    pub fn salamander(password: impl Into<String>) -> Self {
        Self {
            obfs_type: "salamander".to_string(),
            password: password.into(),
        }
    }
}

/// Transport-specific parameters, including the endpoint's auth
/// material. Modelled as a tagged union rather than a bag of optional
/// fields so that "a REALITY endpoint without REALITY parameters" is not
/// representable at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport")]
pub enum TransportParams {
    #[serde(rename = "vless-reality")]
    VlessReality {
        /// VLESS client id. Per-user credential.
        uuid: String,
        /// XTLS flow. `Some(VLESS_FLOW_VISION)` in production; `None`
        /// only in the `diag-vision-off` diagnostic profile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flow: Option<String>,
        reality: RealityParams,
    },
    #[serde(rename = "hysteria2")]
    Hysteria2 {
        /// Hysteria2 auth password. Per-user credential.
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        obfs: Option<Hysteria2Obfs>,
    },
}

impl TransportParams {
    pub fn transport(&self) -> Transport {
        match self {
            TransportParams::VlessReality { .. } => Transport::VlessReality,
            TransportParams::Hysteria2 { .. } => Transport::Hysteria2,
        }
    }
}

/// One dialable listener.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Stable identifier, unique within a document (e.g. `reality-1`).
    pub id: String,
    /// Human-facing label shown in the client's endpoint picker.
    pub tag: String,
    /// Dialable host (DNS name or literal address).
    pub host: String,
    pub port: u16,
    /// TLS SNI / REALITY server name to present.
    pub server_name: String,
    #[serde(flatten)]
    pub params: TransportParams,
}

impl Endpoint {
    pub fn transport(&self) -> Transport {
        self.params.transport()
    }
}

// ---------------------------------------------------------------------
// Document
// ---------------------------------------------------------------------

/// Who is serving this document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Always [`PRODUCT`] for documents this server emits.
    pub product: String,
    /// Server release version.
    pub version: String,
    /// Oldest client release able to consume this document.
    pub minimum_client_version: String,
}

impl ServerInfo {
    pub fn current(version: impl Into<String>) -> Self {
        Self {
            product: PRODUCT.to_string(),
            version: version.into(),
            minimum_client_version: MINIMUM_CLIENT_VERSION.to_string(),
        }
    }
}

/// The versioned provisioning document — the whole contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisioningDocument {
    pub schema_version: u32,
    pub server: ServerInfo,
    /// Production transports this deployment can actually serve right
    /// now. Derived from real configuration; a transport that is not
    /// configured simply does not appear, and has no endpoint.
    pub capabilities: Vec<Capability>,
    /// Diagnostic/experimental modes available to this user. Never part
    /// of normal capability negotiation, never a default. Empty in a
    /// normal production document.
    #[serde(default)]
    pub experimental_capabilities: Vec<ExperimentalCapability>,
    pub endpoints: Vec<Endpoint>,
}

/// Substrings that must never appear anywhere in a serialized
/// provisioning document, with the reason each is forbidden. Checked by
/// [`ProvisioningDocument::validate`] against the serialized form, so a
/// field added in the future cannot reintroduce one of these without the
/// contract tests failing.
const FORBIDDEN_CONTENT: &[(&str, &str)] = &[
    ("private_key", "server-private key material"),
    ("privatekey", "server-private key material"),
    ("private-key", "server-private key material"),
    ("-----BEGIN", "PEM-encoded key or certificate"),
    (".pem", "TLS/server filesystem path"),
    ("/etc/", "server filesystem path"),
    ("/var/", "server filesystem path"),
    ("/opt/", "server filesystem path"),
    ("insecure", "certificate-verification opt-out"),
    ("\"dns\"", "client-owned DNS policy"),
    ("\"mtu\"", "client-owned MTU"),
    ("\"tun\"", "client-owned TUN configuration"),
    ("auto_route", "client-owned routing policy"),
    ("strict_route", "client-owned routing policy"),
    ("kill_switch", "client-owned kill switch"),
    ("kill-switch", "client-owned kill switch"),
    ("\"inbounds\"", "client-owned listener configuration"),
    ("ipv6_", "client-owned IP-family policy"),
    ("\"ipv4\"", "client-owned IP-family policy"),
    ("\"ipv6\"", "client-owned IP-family policy"),
];

impl ProvisioningDocument {
    /// Build a document at the current [`SCHEMA_VERSION`]. The result is
    /// NOT validated — call [`ProvisioningDocument::validate`] (or use
    /// [`ProvisioningDocument::to_json`], which validates first).
    pub fn new(
        server: ServerInfo,
        capabilities: Vec<Capability>,
        endpoints: Vec<Endpoint>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            server,
            capabilities,
            experimental_capabilities: Vec::new(),
            endpoints,
        }
    }

    pub fn with_experimental_capabilities(
        mut self,
        experimental: Vec<ExperimentalCapability>,
    ) -> Self {
        self.experimental_capabilities = experimental;
        self
    }

    /// True when this document advertises `capability` as a production
    /// capability. This is the "REALITY available yes/no" /
    /// "Hysteria2 available yes/no" question, answered from real
    /// configuration.
    pub fn supports(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
    }

    /// Full validation. Every rule here is a rule a client may rely on:
    /// a document that validates is one every supported client can dial.
    pub fn validate(&self) -> Result<(), ContractError> {
        if !is_supported_schema_version(self.schema_version) {
            return Err(ContractError::UnsupportedSchemaVersion {
                requested: self.schema_version,
                supported: SUPPORTED_SCHEMA_VERSIONS.to_vec(),
            });
        }

        non_empty("server.product", &self.server.product)?;
        non_empty("server.version", &self.server.version)?;
        non_empty(
            "server.minimum_client_version",
            &self.server.minimum_client_version,
        )?;

        for cap in &self.capabilities {
            if cap.as_str().starts_with(EXPERIMENTAL_CAPABILITY_PREFIX) {
                return Err(ContractError::ExperimentalInProductionCapabilities {
                    capability: cap.as_str().to_string(),
                });
            }
            non_empty("capabilities[]", cap.as_str())?;
        }
        for cap in &self.experimental_capabilities {
            if !cap.as_str().starts_with(EXPERIMENTAL_CAPABILITY_PREFIX) {
                return Err(ContractError::ExperimentalCapabilityMisnamed {
                    capability: cap.as_str().to_string(),
                    prefix: EXPERIMENTAL_CAPABILITY_PREFIX,
                });
            }
        }

        let mut seen_ids: Vec<&str> = Vec::with_capacity(self.endpoints.len());
        for ep in &self.endpoints {
            ep.validate()?;
            if seen_ids.contains(&ep.id.as_str()) {
                return Err(ContractError::DuplicateEndpointId { id: ep.id.clone() });
            }
            seen_ids.push(&ep.id);

            let cap = Capability::for_transport(&ep.transport());
            if !self.capabilities.contains(&cap) {
                return Err(ContractError::EndpointWithoutCapability {
                    endpoint_id: ep.id.clone(),
                    transport: ep.transport().as_str().to_string(),
                });
            }
        }

        for cap in &self.capabilities {
            let served = self
                .endpoints
                .iter()
                .any(|ep| &Capability::for_transport(&ep.transport()) == cap);
            if !served {
                return Err(ContractError::CapabilityWithoutEndpoint {
                    capability: cap.as_str().to_string(),
                });
            }
        }

        self.audit_serialized()
    }

    /// Defence in depth behind the type system: serialize and confirm no
    /// forbidden key or value made it into the wire form. The types
    /// above cannot express any of these today; this makes a future
    /// field that could an immediate, loud test failure rather than a
    /// silent credential or policy leak.
    fn audit_serialized(&self) -> Result<(), ContractError> {
        let encoded = serde_json::to_string(self).map_err(|e| ContractError::Invalid {
            field: "document",
            reason: format!("could not be serialized: {e}"),
        })?;
        let lowered = encoded.to_ascii_lowercase();
        for (needle, reason) in FORBIDDEN_CONTENT {
            if lowered.contains(&needle.to_ascii_lowercase()) {
                return Err(ContractError::ForbiddenContent {
                    found: (*needle).to_string(),
                    reason,
                });
            }
        }
        Ok(())
    }

    /// Validate, then serialize. This is the ONLY serialization path the
    /// server uses for client-facing output: an invalid document is
    /// never emitted, so the checks above cannot be bypassed by
    /// forgetting to call `validate`.
    pub fn to_json(&self) -> Result<String, ContractError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| ContractError::Invalid {
            field: "document",
            reason: format!("could not be serialized: {e}"),
        })
    }

    /// Parse and validate. Rejects an unsupported `schema_version`
    /// explicitly rather than best-effort reinterpreting it.
    pub fn from_json(s: &str) -> Result<Self, ContractError> {
        let doc: ProvisioningDocument =
            serde_json::from_str(s).map_err(|e| ContractError::Invalid {
                field: "document",
                reason: format!("could not be parsed: {e}"),
            })?;
        doc.validate()?;
        Ok(doc)
    }
}

impl Endpoint {
    pub fn validate(&self) -> Result<(), ContractError> {
        non_empty("endpoint.id", &self.id)?;
        non_empty("endpoint.tag", &self.tag)?;
        non_empty("endpoint.host", &self.host)?;
        non_empty("endpoint.server_name", &self.server_name)?;
        if self.host.chars().any(|c| c.is_whitespace() || c == '/') {
            return Err(ContractError::Invalid {
                field: "endpoint.host",
                reason: format!("{:?} is not a dialable host", self.host),
            });
        }
        if self.port == 0 {
            return Err(ContractError::Invalid {
                field: "endpoint.port",
                reason: "port 0 is not dialable".to_string(),
            });
        }

        match &self.params {
            TransportParams::VlessReality {
                uuid,
                flow,
                reality,
            } => {
                if !is_uuid(uuid) {
                    return Err(ContractError::MalformedUuid {
                        endpoint_id: self.id.clone(),
                        got: uuid.clone(),
                    });
                }
                if let Some(flow) = flow {
                    if flow != VLESS_FLOW_VISION {
                        return Err(ContractError::Invalid {
                            field: "endpoint.flow",
                            reason: format!(
                                "{flow:?} is not a flow this server provisions (expected \
                                 {VLESS_FLOW_VISION:?}, or absent for the diag-vision-off profile)"
                            ),
                        });
                    }
                }
                if reality.public_key.trim().is_empty() {
                    return Err(ContractError::MissingRealityParameter {
                        endpoint_id: self.id.clone(),
                        missing: "public_key",
                    });
                }
                if reality.short_id.trim().is_empty() {
                    return Err(ContractError::MissingRealityParameter {
                        endpoint_id: self.id.clone(),
                        missing: "short_id",
                    });
                }
                if reality.fingerprint.trim().is_empty() {
                    return Err(ContractError::MissingRealityParameter {
                        endpoint_id: self.id.clone(),
                        missing: "fingerprint",
                    });
                }
                if !reality.short_id.chars().all(|c| c.is_ascii_hexdigit())
                    || reality.short_id.len() % 2 != 0
                    || reality.short_id.len() > 16
                {
                    return Err(ContractError::Invalid {
                        field: "endpoint.reality.short_id",
                        reason: format!(
                            "{:?} is not an even-length hex string of at most 16 characters",
                            reality.short_id
                        ),
                    });
                }
            }
            TransportParams::Hysteria2 { password, obfs } => {
                if password.trim().is_empty() {
                    return Err(ContractError::EmptyHysteria2Password {
                        endpoint_id: self.id.clone(),
                    });
                }
                if let Some(obfs) = obfs {
                    if obfs.password.trim().is_empty() {
                        return Err(ContractError::EmptyObfsPassword {
                            endpoint_id: self.id.clone(),
                        });
                    }
                    if obfs.obfs_type != "salamander" {
                        return Err(ContractError::Invalid {
                            field: "endpoint.obfs.type",
                            reason: format!(
                                "{:?} is not an obfuscator this server configures (expected \
                                 \"salamander\")",
                                obfs.obfs_type
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn non_empty(field: &'static str, value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::Empty { field });
    }
    Ok(())
}

/// 8-4-4-4-12 lowercase-or-uppercase hex, the only shape sing-box's
/// VLESS inbound accepts as a client id.
fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    if groups.len() != 5 {
        return false;
    }
    let expected = [8usize, 4, 4, 4, 12];
    groups
        .iter()
        .zip(expected)
        .all(|(g, len)| g.len() == len && g.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn reality_endpoint() -> Endpoint {
        Endpoint {
            id: "reality-1".into(),
            tag: "Reality".into(),
            host: "vpn.example.com".into(),
            port: 443,
            server_name: "www.example-decoy.com".into(),
            params: TransportParams::VlessReality {
                uuid: "11111111-1111-4111-8111-111111111111".into(),
                flow: Some(VLESS_FLOW_VISION.into()),
                reality: RealityParams {
                    public_key: "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake".into(),
                    short_id: "0a1b2c3d".into(),
                    fingerprint: "chrome".into(),
                },
            },
        }
    }

    pub(crate) fn hysteria2_endpoint() -> Endpoint {
        Endpoint {
            id: "hysteria2-1".into(),
            tag: "Hysteria2".into(),
            host: "vpn.example.com".into(),
            port: 443,
            server_name: "vpn.example.com".into(),
            params: TransportParams::Hysteria2 {
                password: "fake-hysteria2-password".into(),
                obfs: None,
            },
        }
    }

    fn doc(endpoints: Vec<Endpoint>) -> ProvisioningDocument {
        let caps = endpoints
            .iter()
            .map(|e| Capability::for_transport(&e.transport()))
            .collect();
        ProvisioningDocument::new(ServerInfo::current("0.1.2"), caps, endpoints)
    }

    #[test]
    fn both_transports_document_is_valid() {
        let d = doc(vec![reality_endpoint(), hysteria2_endpoint()]);
        d.validate().expect("valid");
        assert!(d.supports(&Capability::VlessReality));
        assert!(d.supports(&Capability::Hysteria2));
    }

    #[test]
    fn reality_only_document_reports_hysteria2_unavailable() {
        let d = doc(vec![reality_endpoint()]);
        d.validate().expect("valid");
        assert!(d.supports(&Capability::VlessReality));
        assert!(
            !d.supports(&Capability::Hysteria2),
            "a transport with no configured endpoint must not be advertised"
        );
    }

    #[test]
    fn schema_version_is_explicit_and_current() {
        let d = doc(vec![reality_endpoint()]);
        assert_eq!(d.schema_version, 1);
        let v: serde_json::Value = serde_json::from_str(&d.to_json().unwrap()).unwrap();
        assert_eq!(v["schema_version"], 1);
    }

    #[test]
    fn unsupported_schema_version_is_rejected_not_reinterpreted() {
        let mut d = doc(vec![reality_endpoint()]);
        d.schema_version = 2;
        assert_eq!(
            d.validate(),
            Err(ContractError::UnsupportedSchemaVersion {
                requested: 2,
                supported: vec![1]
            })
        );
        assert!(
            d.to_json().is_err(),
            "must not serialize an unknown version"
        );
    }

    #[test]
    fn unsupported_schema_version_error_body_is_machine_readable() {
        let e = UnsupportedSchemaVersion::new(9);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["error"], "unsupported_schema_version");
        assert_eq!(v["requested"], 9);
        assert_eq!(v["supported"], serde_json::json!([1]));
        assert!(v["message"].as_str().unwrap().contains("9"));
    }

    #[test]
    fn missing_reality_public_key_is_rejected() {
        let mut ep = reality_endpoint();
        if let TransportParams::VlessReality { reality, .. } = &mut ep.params {
            reality.public_key = String::new();
        }
        assert_eq!(
            doc(vec![ep]).validate(),
            Err(ContractError::MissingRealityParameter {
                endpoint_id: "reality-1".into(),
                missing: "public_key"
            })
        );
    }

    #[test]
    fn missing_reality_short_id_is_rejected() {
        let mut ep = reality_endpoint();
        if let TransportParams::VlessReality { reality, .. } = &mut ep.params {
            reality.short_id = "   ".into();
        }
        assert_eq!(
            doc(vec![ep]).validate(),
            Err(ContractError::MissingRealityParameter {
                endpoint_id: "reality-1".into(),
                missing: "short_id"
            })
        );
    }

    #[test]
    fn non_hex_short_id_is_rejected() {
        let mut ep = reality_endpoint();
        if let TransportParams::VlessReality { reality, .. } = &mut ep.params {
            reality.short_id = "zzzz".into();
        }
        assert!(matches!(
            doc(vec![ep]).validate(),
            Err(ContractError::Invalid { field, .. }) if field == "endpoint.reality.short_id"
        ));
    }

    #[test]
    fn malformed_uuid_is_rejected() {
        for bad in [
            "not-a-uuid",
            "11111111-1111-4111-8111",
            "11111111111141118111111111111111",
            "gggggggg-1111-4111-8111-111111111111",
            "",
        ] {
            let mut ep = reality_endpoint();
            if let TransportParams::VlessReality { uuid, .. } = &mut ep.params {
                *uuid = bad.into();
            }
            assert!(
                matches!(
                    doc(vec![ep]).validate(),
                    Err(ContractError::MalformedUuid { .. })
                ),
                "expected {bad:?} to be rejected as a VLESS client id"
            );
        }
    }

    #[test]
    fn empty_hysteria2_password_is_rejected() {
        let mut ep = hysteria2_endpoint();
        if let TransportParams::Hysteria2 { password, .. } = &mut ep.params {
            *password = "".into();
        }
        assert_eq!(
            doc(vec![ep]).validate(),
            Err(ContractError::EmptyHysteria2Password {
                endpoint_id: "hysteria2-1".into()
            })
        );
    }

    #[test]
    fn empty_obfs_password_is_rejected() {
        let mut ep = hysteria2_endpoint();
        if let TransportParams::Hysteria2 { obfs, .. } = &mut ep.params {
            *obfs = Some(Hysteria2Obfs::salamander(""));
        }
        assert_eq!(
            doc(vec![ep]).validate(),
            Err(ContractError::EmptyObfsPassword {
                endpoint_id: "hysteria2-1".into()
            })
        );
    }

    #[test]
    fn unknown_obfuscator_is_rejected() {
        let mut ep = hysteria2_endpoint();
        if let TransportParams::Hysteria2 { obfs, .. } = &mut ep.params {
            *obfs = Some(Hysteria2Obfs {
                obfs_type: "not-salamander".into(),
                password: "x".into(),
            });
        }
        assert!(matches!(
            doc(vec![ep]).validate(),
            Err(ContractError::Invalid { field, .. }) if field == "endpoint.obfs.type"
        ));
    }

    #[test]
    fn capability_advertised_without_an_endpoint_is_rejected() {
        let mut d = doc(vec![reality_endpoint()]);
        d.capabilities.push(Capability::Hysteria2);
        assert_eq!(
            d.validate(),
            Err(ContractError::CapabilityWithoutEndpoint {
                capability: "hysteria2".into()
            })
        );
    }

    #[test]
    fn endpoint_without_its_capability_is_rejected() {
        let mut d = doc(vec![reality_endpoint(), hysteria2_endpoint()]);
        d.capabilities.retain(|c| c != &Capability::Hysteria2);
        assert_eq!(
            d.validate(),
            Err(ContractError::EndpointWithoutCapability {
                endpoint_id: "hysteria2-1".into(),
                transport: "hysteria2".into()
            })
        );
    }

    #[test]
    fn duplicate_endpoint_ids_are_rejected() {
        let mut second = reality_endpoint();
        second.tag = "Reality copy".into();
        assert_eq!(
            doc(vec![reality_endpoint(), second]).validate(),
            Err(ContractError::DuplicateEndpointId {
                id: "reality-1".into()
            })
        );
    }

    #[test]
    fn zero_port_and_malformed_host_are_rejected() {
        let mut ep = reality_endpoint();
        ep.port = 0;
        assert!(matches!(
            doc(vec![ep]).validate(),
            Err(ContractError::Invalid { field, .. }) if field == "endpoint.port"
        ));

        let mut ep = reality_endpoint();
        ep.host = "https://vpn.example.com".into();
        assert!(matches!(
            doc(vec![ep]).validate(),
            Err(ContractError::Invalid { field, .. }) if field == "endpoint.host"
        ));
    }

    #[test]
    fn unexpected_flow_is_rejected() {
        let mut ep = reality_endpoint();
        if let TransportParams::VlessReality { flow, .. } = &mut ep.params {
            *flow = Some("xtls-rprx-direct".into());
        }
        assert!(matches!(
            doc(vec![ep]).validate(),
            Err(ContractError::Invalid { field, .. }) if field == "endpoint.flow"
        ));
    }

    #[test]
    fn experimental_capability_may_not_appear_in_production_capabilities() {
        let mut d = doc(vec![reality_endpoint()]);
        d.capabilities
            .push(Capability::Other("diag-vision-off".into()));
        assert_eq!(
            d.validate(),
            Err(ContractError::ExperimentalInProductionCapabilities {
                capability: "diag-vision-off".into()
            })
        );
    }

    #[test]
    fn experimental_capability_must_carry_the_diag_prefix() {
        let d = doc(vec![reality_endpoint()])
            .with_experimental_capabilities(vec![ExperimentalCapability("vision-off".into())]);
        assert_eq!(
            d.validate(),
            Err(ContractError::ExperimentalCapabilityMisnamed {
                capability: "vision-off".into(),
                prefix: "diag-"
            })
        );
    }

    #[test]
    fn experimental_capabilities_are_a_separate_field_from_production_ones() {
        let d = doc(vec![reality_endpoint()]).with_experimental_capabilities(vec![
            ExperimentalCapability::tcp_only(),
            ExperimentalCapability::vision_off(),
        ]);
        d.validate().expect("valid");
        let v: serde_json::Value = serde_json::from_str(&d.to_json().unwrap()).unwrap();
        assert_eq!(v["capabilities"], serde_json::json!(["vless-reality"]));
        assert_eq!(
            v["experimental_capabilities"],
            serde_json::json!(["diag-tcp-only", "diag-vision-off"])
        );
    }

    #[test]
    fn unknown_capability_and_transport_round_trip_for_forward_compatibility() {
        // A `schema_version = 1` consumer must survive a future server
        // advertising something it has never heard of. Adding a
        // capability value is a COMPATIBLE change (see crate docs).
        assert_eq!(
            Capability::from_wire("some-future-transport"),
            Capability::Other("some-future-transport".into())
        );
        let encoded = serde_json::to_string(&Capability::Other("x-new".into())).unwrap();
        assert_eq!(encoded, "\"x-new\"");
        let decoded: Capability = serde_json::from_str("\"x-new\"").unwrap();
        assert_eq!(decoded, Capability::Other("x-new".into()));
    }

    #[test]
    fn json_round_trip_preserves_every_field() {
        let original = doc(vec![
            reality_endpoint(),
            Endpoint {
                params: TransportParams::Hysteria2 {
                    password: "fake-hysteria2-password".into(),
                    obfs: Some(Hysteria2Obfs::salamander("fake-obfs-password")),
                },
                ..hysteria2_endpoint()
            },
        ]);
        let json = original.to_json().unwrap();
        let parsed = ProvisioningDocument::from_json(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn from_json_rejects_an_unsupported_schema_version() {
        let json = r#"{
            "schema_version": 99,
            "server": {"product":"singbox-vpn","version":"0.1.2","minimum_client_version":"0.1.0"},
            "capabilities": [],
            "experimental_capabilities": [],
            "endpoints": []
        }"#;
        assert_eq!(
            ProvisioningDocument::from_json(json),
            Err(ContractError::UnsupportedSchemaVersion {
                requested: 99,
                supported: vec![1]
            })
        );
    }

    /// The point of the whole contract: it is a server-facts document.
    /// Nothing client-owned, nothing server-private.
    #[test]
    fn serialized_document_contains_no_forbidden_key_or_value() {
        let d = doc(vec![reality_endpoint(), hysteria2_endpoint()]);
        let json = d.to_json().unwrap().to_ascii_lowercase();
        for (needle, _) in FORBIDDEN_CONTENT {
            assert!(
                !json.contains(&needle.to_ascii_lowercase()),
                "forbidden content {needle:?} present in {json}"
            );
        }
    }

    #[test]
    fn a_document_carrying_forbidden_content_fails_validation() {
        // Simulates a future field (or a caller-supplied tag) smuggling
        // a server path into the contract — the audit must catch it even
        // though no typed field is meant to hold one.
        let mut ep = reality_endpoint();
        ep.tag = "/etc/vpn/reality/private_key".into();
        assert!(matches!(
            doc(vec![ep]).validate(),
            Err(ContractError::ForbiddenContent { .. })
        ));
    }

    #[test]
    fn vision_off_endpoint_omits_flow_entirely_rather_than_emptying_it() {
        let mut ep = reality_endpoint();
        if let TransportParams::VlessReality { flow, .. } = &mut ep.params {
            *flow = None;
        }
        let d = doc(vec![ep]);
        d.validate().expect("valid");
        let v: serde_json::Value = serde_json::from_str(&d.to_json().unwrap()).unwrap();
        assert!(
            v["endpoints"][0].get("flow").is_none(),
            "an absent flow must be absent, not an empty string"
        );
    }

    #[test]
    fn transport_is_an_internally_tagged_discriminator_on_the_endpoint() {
        let d = doc(vec![reality_endpoint(), hysteria2_endpoint()]);
        let v: serde_json::Value = serde_json::from_str(&d.to_json().unwrap()).unwrap();
        assert_eq!(v["endpoints"][0]["transport"], "vless-reality");
        assert_eq!(v["endpoints"][1]["transport"], "hysteria2");
    }
}
