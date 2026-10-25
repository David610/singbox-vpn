//! Pairing bundles: the data one node hands to the other when a relay and an exit are paired.
//!
//! A bundle carries the paired node's PUBLIC connection parameters plus ONE secret (the credential a node
//! issued for the other). It exists so the operator moves a single protected document between the hosts
//! (over a pipe or a 0600 file) instead of retyping a key, a short id, a port and a UUID. Secrets are never
//! accepted on a command line and never printed: `Debug` redacts the UUID, there is no `Display`.

use crate::deployment::{DeploymentConfig, PeerEndpointSection};
use crate::model::{CompatTransport, EndpointVlessFlow};
use crate::CompatError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const EXIT_BUNDLE_SCHEMA: &str = "singbox-vpn-exit-pairing/1";
pub const HAIRPIN_BUNDLE_SCHEMA: &str = "singbox-vpn-hairpin-pairing/1";

/// Public parameters of the node that issued a bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedNode {
    pub node_id: String,
    pub host: String,
    pub port: u16,
    pub server_name: String,
    pub reality_public_key: String,
    pub reality_short_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleCredential {
    pub uuid: String,
}

impl std::fmt::Debug for BundleCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BundleCredential { uuid: <redacted> }")
    }
}

/// Issued by an EXIT for a relay user: how to reach the exit, which flow the credential is rendered with, and
/// the dedicated credential.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitPairingBundle {
    pub schema: String,
    pub exit: PairedNode,
    pub vless_flow: EndpointVlessFlow,
    pub credential: BundleCredential,
}

/// Issued by a RELAY for an exit's Google/YouTube hairpin: how to reach the relay and the hairpin principal's
/// credential.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HairpinPairingBundle {
    pub schema: String,
    pub relay: PairedNode,
    pub credential: BundleCredential,
}

fn check_node(node: &PairedNode, what: &str) -> Result<(), CompatError> {
    let bad = |field: &str, why: &str| CompatError::Parse(format!("{what} bundle: {field} {why}"));
    if crate::deployment::validate_node_id(&node.node_id).is_err() {
        return Err(bad("node_id", "is not a valid node id"));
    }
    if node.host.trim().is_empty() || node.host.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err(bad("host", "is not dialable"));
    }
    if node.host != node.host.to_ascii_lowercase() {
        return Err(bad("host", "must be lowercase"));
    }
    if node.port == 0 {
        return Err(bad("port", "must not be 0"));
    }
    if node.server_name.trim().is_empty() {
        return Err(bad("server_name", "is empty"));
    }
    crate::credentials::validate_reality_public_key_shape(&node.reality_public_key)
        .map_err(|e| bad("reality_public_key", &format!("is malformed ({e})")))?;
    let sid = &node.reality_short_id;
    if sid.is_empty()
        || !sid.len().is_multiple_of(2)
        || sid.len() > 16
        || !sid.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(bad(
            "reality_short_id",
            "must be 2-16 hex digits, even length",
        ));
    }
    Ok(())
}

fn check_uuid(c: &BundleCredential, what: &str) -> Result<(), CompatError> {
    if crate::credentials::is_uuid_v4_shaped(&c.uuid) {
        Ok(())
    } else {
        // Never echo the value: a near miss is still most of a credential.
        Err(CompatError::Parse(format!(
            "{what} bundle: credential.uuid is not an 8-4-4-4-12 hex UUID ({} characters, not shown)",
            c.uuid.chars().count()
        )))
    }
}

pub fn parse_exit_bundle(text: &str) -> Result<ExitPairingBundle, CompatError> {
    let b: ExitPairingBundle = serde_json::from_str(text)
        .map_err(|e| CompatError::Parse(format!("exit pairing bundle is not valid: {e}")))?;
    if b.schema != EXIT_BUNDLE_SCHEMA {
        return Err(CompatError::Parse(format!(
            "exit pairing bundle: unsupported schema {:?} (this vpn-admin reads {EXIT_BUNDLE_SCHEMA})",
            b.schema
        )));
    }
    check_node(&b.exit, "exit")?;
    check_uuid(&b.credential, "exit")?;
    Ok(b)
}

pub fn parse_hairpin_bundle(text: &str) -> Result<HairpinPairingBundle, CompatError> {
    let b: HairpinPairingBundle = serde_json::from_str(text)
        .map_err(|e| CompatError::Parse(format!("hairpin pairing bundle is not valid: {e}")))?;
    if b.schema != HAIRPIN_BUNDLE_SCHEMA {
        return Err(CompatError::Parse(format!(
            "hairpin pairing bundle: unsupported schema {:?} (this vpn-admin reads {HAIRPIN_BUNDLE_SCHEMA})",
            b.schema
        )));
    }
    check_node(&b.relay, "relay")?;
    check_uuid(&b.credential, "relay")?;
    Ok(b)
}

/// The relayed `[[peer_endpoints]]` entry an exit bundle describes. `path` must be the relay's access-path id.
pub fn peer_endpoint_from_exit_bundle(
    endpoint_id: &str,
    tag: &str,
    path: &str,
    bundle: &ExitPairingBundle,
) -> PeerEndpointSection {
    PeerEndpointSection {
        id: endpoint_id.to_string(),
        tag: tag.to_string(),
        host: bundle.exit.host.clone(),
        port: bundle.exit.port,
        transport: CompatTransport::VlessReality,
        server_name: Some(bundle.exit.server_name.clone()),
        reality_public_key: Some(bundle.exit.reality_public_key.clone()),
        reality_short_id: Some(bundle.exit.reality_short_id.clone()),
        reality_fingerprint: "chrome".to_string(),
        obfs_password: None,
        failure_domain: format!("exit:{}", bundle.exit.node_id),
        region: None,
        provider: None,
        asn: None,
        path: path.to_string(),
        credential_ref: None,
        vless_flow: bundle.vless_flow,
        extra: Default::default(),
    }
}

/// Whether an existing declaration is the same route as the one a bundle describes (public parameters only).
pub fn same_route(existing: &PeerEndpointSection, wanted: &PeerEndpointSection) -> bool {
    existing.host == wanted.host
        && existing.port == wanted.port
        && existing.transport == wanted.transport
        && existing.server_name == wanted.server_name
        && existing.reality_public_key == wanted.reality_public_key
        && existing.reality_short_id == wanted.reality_short_id
        && existing.path == wanted.path
        && existing.vless_flow == wanted.vless_flow
        && existing.credential_ref == wanted.credential_ref
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeerEndpointOutcome {
    /// An identical declaration was already present; nothing written.
    AlreadyDeclared,
    /// Appended; the backup of the previous file is returned.
    Declared(PathBuf),
}

/// Declare `wanted` in `deployment.toml` by appending one `[[peer_endpoints]]` block. A different declaration
/// with the same id is REFUSED (never silently replaced). Only `peer_endpoints` may change; the result must
/// load and validate before it is committed atomically.
pub fn apply_peer_endpoint_toml(
    path: &Path,
    wanted: &PeerEndpointSection,
) -> Result<PeerEndpointOutcome, CompatError> {
    let original = std::fs::read_to_string(path).map_err(|e| CompatError::Io(e.to_string()))?;
    let old: DeploymentConfig =
        toml::from_str(&original).map_err(|e| CompatError::Parse(e.to_string()))?;
    old.validate()?;
    if let Some(existing) = old.peer_endpoints.iter().find(|p| p.id == wanted.id) {
        return if same_route(existing, wanted) {
            Ok(PeerEndpointOutcome::AlreadyDeclared)
        } else {
            Err(CompatError::Parse(format!(
                "peer endpoint {:?} is already declared with different parameters; refusing to replace it \
                 (declare a new id, or remove the old [[peer_endpoints]] block deliberately)",
                wanted.id
            )))
        };
    }
    let mut block = format!(
        "\n[[peer_endpoints]]\nid = {:?}\ntag = {:?}\nhost = {:?}\nport = {}\ntransport = \"vless_reality\"\n",
        wanted.id, wanted.tag, wanted.host, wanted.port
    );
    if let Some(v) = &wanted.server_name {
        block.push_str(&format!("server_name = {v:?}\n"));
    }
    if let Some(v) = &wanted.reality_public_key {
        block.push_str(&format!("reality_public_key = {v:?}\n"));
    }
    if let Some(v) = &wanted.reality_short_id {
        block.push_str(&format!("reality_short_id = {v:?}\n"));
    }
    block.push_str(&format!(
        "failure_domain = {:?}\npath = {:?}\n",
        wanted.failure_domain, wanted.path
    ));
    if wanted.vless_flow == EndpointVlessFlow::None {
        block.push_str("vless_flow = \"none\"\n");
    }
    let mut patched = original.trim_end().to_string();
    patched.push('\n');
    patched.push_str(&block);
    let new: DeploymentConfig =
        toml::from_str(&patched).map_err(|e| CompatError::Parse(e.to_string()))?;
    new.validate()?;
    let mut a = serde_json::to_value(&old).map_err(|e| CompatError::Parse(e.to_string()))?;
    let mut b = serde_json::to_value(&new).map_err(|e| CompatError::Parse(e.to_string()))?;
    a.as_object_mut().unwrap().remove("peer_endpoints");
    b.as_object_mut().unwrap().remove("peer_endpoints");
    if a != b || new.peer_endpoints.len() != old.peer_endpoints.len() + 1 {
        return Err(CompatError::Parse(
            "peer endpoint patch would change another operator setting; refusing".into(),
        ));
    }
    let backup = crate::migrate::backup_before_mutate(path)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map_err(|e| CompatError::Io(e.to_string()))?
            .permissions()
            .mode()
            & 0o777
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    crate::migrate::atomic_write(path, patched.as_bytes(), mode)?;
    Ok(PeerEndpointOutcome::Declared(backup))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    const UUID: &str = "11111111-2222-4333-8444-555555555555";

    fn node(id: &str) -> PairedNode {
        PairedNode {
            node_id: id.into(),
            host: "de1.example.test".into(),
            port: 443,
            server_name: "www.example.com".into(),
            reality_public_key: KEY.into(),
            reality_short_id: "0a1b2c3d".into(),
        }
    }

    fn exit_bundle() -> ExitPairingBundle {
        ExitPairingBundle {
            schema: EXIT_BUNDLE_SCHEMA.into(),
            exit: node("de1"),
            vless_flow: EndpointVlessFlow::None,
            credential: BundleCredential { uuid: UUID.into() },
        }
    }

    #[test]
    fn exit_bundle_round_trips_and_debug_never_shows_the_credential() {
        let text = serde_json::to_string(&exit_bundle()).unwrap();
        let parsed = parse_exit_bundle(&text).unwrap();
        assert_eq!(parsed.exit, node("de1"));
        assert_eq!(parsed.vless_flow, EndpointVlessFlow::None);
        assert!(!format!("{parsed:?}").contains(UUID));
    }

    #[test]
    fn malformed_bundles_are_refused_without_echoing_the_secret() {
        let mut b = exit_bundle();
        b.credential.uuid = "not-a-uuid-but-secret-looking".into();
        let err = parse_exit_bundle(&serde_json::to_string(&b).unwrap())
            .unwrap_err()
            .to_string();
        assert!(!err.contains("secret-looking"), "{err}");
        for mutate in [
            |b: &mut ExitPairingBundle| b.schema = "other/1".into(),
            |b: &mut ExitPairingBundle| b.exit.port = 0,
            |b: &mut ExitPairingBundle| b.exit.host = "De1.Example.Test".into(),
            |b: &mut ExitPairingBundle| b.exit.reality_short_id = "xyz".into(),
            |b: &mut ExitPairingBundle| b.exit.reality_public_key = "short".into(),
            |b: &mut ExitPairingBundle| b.exit.host = "a b".into(),
        ] {
            let mut b = exit_bundle();
            mutate(&mut b);
            assert!(parse_exit_bundle(&serde_json::to_string(&b).unwrap()).is_err());
        }
        // unknown keys are refused rather than ignored
        let extra = serde_json::to_string(&exit_bundle())
            .unwrap()
            .replace("\"schema\"", "\"extra\":1,\"schema\"");
        assert!(parse_exit_bundle(&extra).is_err());
    }

    #[test]
    fn hairpin_bundle_parses_and_checks_the_relay_node() {
        let b = HairpinPairingBundle {
            schema: HAIRPIN_BUNDLE_SCHEMA.into(),
            relay: node("ru1"),
            credential: BundleCredential { uuid: UUID.into() },
        };
        assert!(parse_hairpin_bundle(&serde_json::to_string(&b).unwrap()).is_ok());
        let mut bad = b.clone();
        bad.relay.port = 0;
        assert!(parse_hairpin_bundle(&serde_json::to_string(&bad).unwrap()).is_err());
        // an exit bundle is not a hairpin bundle
        assert!(parse_hairpin_bundle(&serde_json::to_string(&exit_bundle()).unwrap()).is_err());
    }

    #[test]
    fn peer_endpoint_carries_the_bundle_flow_and_exit_identity() {
        let e = peer_endpoint_from_exit_bundle(
            "de1-via-ru1",
            "Germany · via Russia",
            "via-ru1",
            &exit_bundle(),
        );
        assert_eq!(e.vless_flow, EndpointVlessFlow::None);
        assert_eq!(e.failure_domain, "exit:de1");
        assert_eq!(
            e.credential_ref, None,
            "a relayed route owns its dedicated credential"
        );
        assert_eq!(e.path, "via-ru1");
    }
}
