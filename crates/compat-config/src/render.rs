//! Client-facing rendering: share-link URIs (VLESS/Hysteria2, consumed by
//! Hiddify/v2rayNG/NekoBox) and native sing-box subscription JSON
//! (consumed directly by Hiddify/sing-box clients). Syntax verified
//! against current sing-box docs — see `docs/COMPATIBILITY_VERSIONS.md`.
//! Never renders server-private material (`RealityServerParams`,
//! `Hysteria2ServerParams`'s TLS key path) — only `PublicParameters`.

use crate::model::{CompatEndpoint, CompatTransport, CompatUser, PublicParameters};
use crate::CompatError;
use serde_json::json;

fn percent_encode_label(label: &str) -> String {
    let mut out = String::new();
    for b in label.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// This endpoint's effective VLESS UUID: its own per-node override
/// (`CompatEndpoint::credentials`, set only by a multi-node merge) if
/// present, else the rendering `CompatUser`'s. Single-node deployments
/// always take the `user` branch — `credentials` is `None` there.
fn effective_vless_uuid<'a>(user: &'a CompatUser, endpoint: &'a CompatEndpoint) -> &'a str {
    endpoint
        .credentials
        .as_ref()
        .and_then(|c| c.vless_uuid.as_deref())
        .unwrap_or(&user.vless_uuid)
}

/// Same as [`effective_vless_uuid`], for the Hysteria2 password.
fn effective_hysteria2_password<'a>(user: &'a CompatUser, endpoint: &'a CompatEndpoint) -> &'a str {
    endpoint
        .credentials
        .as_ref()
        .and_then(|c| c.hysteria2_password.as_ref())
        .map(|s| s.expose())
        .unwrap_or_else(|| user.hysteria2_password.expose())
}

/// `vless://uuid@host:port?...&security=reality...#label`
pub fn render_vless_reality_uri(
    user: &CompatUser,
    endpoint: &CompatEndpoint,
) -> Result<String, CompatError> {
    let PublicParameters::Reality {
        public_key_hex,
        short_id,
        fingerprint,
    } = &endpoint.public_parameters
    else {
        return Err(CompatError::WrongTransportForEndpoint);
    };
    let sni = endpoint.server_name.as_deref().unwrap_or(&endpoint.host);
    Ok(format!(
        "vless://{uuid}@{host}:{port}?encryption=none&security=reality&sni={sni}&fp={fp}&pbk={pbk}&sid={sid}&type=tcp&flow=xtls-rprx-vision#{label}",
        uuid = effective_vless_uuid(user, endpoint),
        host = endpoint.host,
        port = endpoint.port,
        sni = sni,
        fp = fingerprint,
        pbk = public_key_hex,
        sid = short_id,
        label = percent_encode_label(&endpoint.label),
    ))
}

/// `hysteria2://password@host:port?...#label`
pub fn render_hysteria2_uri(
    user: &CompatUser,
    endpoint: &CompatEndpoint,
) -> Result<String, CompatError> {
    let PublicParameters::Hysteria2 { obfs_password } = &endpoint.public_parameters else {
        return Err(CompatError::WrongTransportForEndpoint);
    };
    let sni = endpoint.server_name.as_deref().unwrap_or(&endpoint.host);
    let mut uri = format!(
        "hysteria2://{password}@{host}:{port}?sni={sni}&insecure=0",
        password = effective_hysteria2_password(user, endpoint),
        host = endpoint.host,
        port = endpoint.port,
        sni = sni,
    );
    if let Some(pw) = obfs_password {
        uri.push_str(&format!("&obfs=salamander&obfs-password={pw}"));
    }
    uri.push('#');
    uri.push_str(&percent_encode_label(&endpoint.label));
    Ok(uri)
}

/// One share-link per enabled endpoint, `?format=uri` subscription body
/// (newline-separated, as consumed by v2rayNG/NekoBox-style importers).
pub fn render_uri_list(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
) -> Result<String, CompatError> {
    let mut lines = Vec::with_capacity(endpoints.len());
    for ep in endpoints {
        let uri = match ep.transport {
            crate::model::CompatTransport::VlessReality => render_vless_reality_uri(user, ep)?,
            crate::model::CompatTransport::Hysteria2 => render_hysteria2_uri(user, ep)?,
        };
        lines.push(uri);
    }
    Ok(lines.join("\n"))
}

/// Native sing-box client subscription: an `outbounds` array (one per
/// endpoint) plus a `urltest` selector so Hiddify/sing-box can
/// automatically pick whichever transport currently measures healthy —
/// spec §22. This is *not* claiming the Rust policy engine drives
/// third-party clients (it doesn't, see §55) — it's sing-box's own
/// built-in `urltest` capability, configured by us.
///
/// As of the Telegram-reliability pass, the subscription's default route
/// is NOT the `urltest` group. `urltest` only proves a fast plain-HTTPS
/// request to a Google endpoint succeeds — it says nothing about
/// Telegram, long-lived connections, media transfers, or how a transport
/// behaves under active DPI. A transport that wins that race is not
/// necessarily the right default for a censored network. Instead we add
/// a `selector` outbound (sing-box's manual proxy-group type, rendered
/// by Hiddify/NekoBox-style clients as a tappable list) with:
///   - `default`: the VLESS+REALITY endpoint's tag — REALITY remains the
///     conservative, deterministic default transport until real
///     measurements say otherwise (see docs/TELEGRAM_RESILIENCE_PLAN.md).
///   - options: every real endpoint tag, in the order supplied, plus
///     `auto` (the pre-existing `urltest` group) as an explicit opt-in.
///
/// `route.final` points at the selector, not at `auto`, so a client that
/// never touches the proxy-group UI still gets the deterministic default
/// rather than whatever `urltest` happened to prefer at import time.
/// Users who want automatic switching can still tap into `auto`
/// themselves — `auto` is not removed, only demoted from being the
/// silent default.
pub fn render_singbox_client_subscription(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
) -> Result<serde_json::Value, CompatError> {
    let mut outbounds = Vec::new();
    let mut tags = Vec::new();
    let mut reality_tag: Option<String> = None;
    for ep in endpoints {
        let tag = ep.label.clone();
        tags.push(tag.clone());
        if matches!(ep.transport, CompatTransport::VlessReality) && reality_tag.is_none() {
            reality_tag = Some(tag.clone());
        }
        let outbound = match &ep.public_parameters {
            PublicParameters::Reality {
                public_key_hex,
                short_id,
                fingerprint,
            } => json!({
                "type": "vless",
                "tag": tag,
                "server": ep.host,
                "server_port": ep.port,
                "uuid": effective_vless_uuid(user, ep),
                "flow": "xtls-rprx-vision",
                "tls": {
                    "enabled": true,
                    "server_name": ep.server_name.clone().unwrap_or_else(|| ep.host.clone()),
                    "utls": { "enabled": true, "fingerprint": fingerprint },
                    "reality": {
                        "enabled": true,
                        "public_key": public_key_hex,
                        "short_id": short_id,
                    }
                }
            }),
            PublicParameters::Hysteria2 { obfs_password } => {
                let mut ob = json!({
                    "type": "hysteria2",
                    "tag": tag,
                    "server": ep.host,
                    "server_port": ep.port,
                    "password": effective_hysteria2_password(user, ep),
                    "tls": {
                        "enabled": true,
                        "server_name": ep.server_name.clone().unwrap_or_else(|| ep.host.clone()),
                        "insecure": false,
                    }
                });
                if let Some(pw) = obfs_password {
                    ob["obfs"] = json!({ "type": "salamander", "password": pw });
                }
                ob
            }
        };
        outbounds.push(outbound);
    }

    outbounds.push(json!({
        "type": "urltest",
        "tag": "auto",
        "outbounds": tags.clone(),
        "url": "https://www.gstatic.com/generate_204",
        "interval": "1m",
    }));

    // Manual selector: what actually decides the default route. `default`
    // is the first VLESS+REALITY endpoint's tag when one is present
    // (conservative default — see this function's doc comment), falling
    // back to the first endpoint of any kind if this deployment somehow
    // has no REALITY endpoint (never expected in the standard two-
    // endpoint deployment, but the renderer must not panic on a
    // reduced/experimental endpoint set).
    let mut selector_options = tags.clone();
    selector_options.push("auto".to_string());
    let default_tag = reality_tag
        .or_else(|| tags.first().cloned())
        .unwrap_or_else(|| "auto".to_string());
    outbounds.push(json!({
        "type": "selector",
        "tag": "select",
        "outbounds": selector_options,
        "default": default_tag,
    }));

    outbounds.push(json!({ "type": "direct", "tag": "direct" }));

    Ok(json!({
        "outbounds": outbounds,
        "route": { "final": "select" }
    }))
}

/// Build the two standard endpoint labels ("Reality" / "Hysteria2") from
/// deployment values. Shared by `services/subscription` (the live HTTP
/// service, builds this once at startup into its cached `AppState`) and
/// `apps/admin`'s `doctor` (rebuilds it fresh from current disk state on
/// every run) — both MUST go through this exact function, not a
/// hand-rolled equivalent, or a coherence check comparing their outputs
/// would just be comparing two different constructions of the same
/// intent rather than actually proving agreement.
pub fn standard_endpoints(
    public_host: &str,
    reality_port: u16,
    hysteria_port: u16,
    reality_public_key_hex: &str,
    reality_short_id: &str,
    handshake_server: &str,
    hysteria_obfs_password: Option<&str>,
) -> Vec<CompatEndpoint> {
    vec![
        CompatEndpoint {
            id: "reality-1".into(),
            transport: CompatTransport::VlessReality,
            host: public_host.into(),
            port: reality_port,
            server_name: Some(handshake_server.into()),
            label: "Reality".into(),
            public_parameters: PublicParameters::Reality {
                public_key_hex: reality_public_key_hex.into(),
                short_id: reality_short_id.into(),
                fingerprint: "chrome".into(),
            },
            credentials: None,
        },
        CompatEndpoint {
            id: "hysteria2-1".into(),
            transport: CompatTransport::Hysteria2,
            host: public_host.into(),
            port: hysteria_port,
            server_name: Some(public_host.into()),
            label: "Hysteria2".into(),
            public_parameters: PublicParameters::Hysteria2 {
                obfs_password: hysteria_obfs_password.map(|s| s.to_string()),
            },
            credentials: None,
        },
    ]
}

/// Multi-node variant of [`standard_endpoints`]: builds the same
/// Reality+Hysteria2 pair for EACH node in `nodes`, labeled
/// `"<node_label> - Reality"` / `"<node_label> - Hysteria2"` so a
/// merged subscription lists all endpoints with clear, at-a-glance
/// origin (important for a nontechnical relative manually switching
/// nodes — see `docs/TELEGRAM_RESILIENCE_PLAN.md` §K). Each node's
/// `NodeCredentials` are baked into that node's two endpoints via
/// `CompatEndpoint::credentials`, so nodes never need to share one
/// `CompatUser` identity — a leaked/rotated credential on one node
/// cannot affect another (independent per-node blast radius, as
/// required for real multi-node isolation).
///
/// **Library-level scaffolding only, not yet operator-usable.** As of
/// this writing nothing in `apps/admin` or `services/subscription`
/// constructs a `NodeSpec` list or calls this function — there is no
/// CLI command or HTTP path that produces a live multi-node
/// subscription yet. This function and its tests prove the *rendering*
/// logic is correct and credential-isolated; wiring an actual second
/// node's config into a `NodeSpec` (deployment config, CLI command,
/// and/or a merge step in `services/subscription`) is separate,
/// not-yet-done work — see `docs/TELEGRAM_RESILIENCE_PLAN.md` §K
/// "What is genuinely missing."
pub struct NodeSpec<'a> {
    pub node_label: &'a str,
    pub public_host: &'a str,
    pub reality_port: u16,
    pub hysteria_port: u16,
    pub reality_public_key_hex: &'a str,
    pub reality_short_id: &'a str,
    pub handshake_server: &'a str,
    pub hysteria_obfs_password: Option<&'a str>,
    pub credentials: crate::model::EndpointCredentials,
}

pub fn multi_node_endpoints(nodes: &[NodeSpec<'_>]) -> Vec<CompatEndpoint> {
    let mut out = Vec::with_capacity(nodes.len() * 2);
    for (i, node) in nodes.iter().enumerate() {
        out.push(CompatEndpoint {
            id: format!("reality-{i}"),
            transport: CompatTransport::VlessReality,
            host: node.public_host.into(),
            port: node.reality_port,
            server_name: Some(node.handshake_server.into()),
            label: format!("{} - Reality", node.node_label),
            public_parameters: PublicParameters::Reality {
                public_key_hex: node.reality_public_key_hex.into(),
                short_id: node.reality_short_id.into(),
                fingerprint: "chrome".into(),
            },
            credentials: Some(node.credentials.clone()),
        });
        out.push(CompatEndpoint {
            id: format!("hysteria2-{i}"),
            transport: CompatTransport::Hysteria2,
            host: node.public_host.into(),
            port: node.hysteria_port,
            server_name: Some(node.public_host.into()),
            label: format!("{} - Hysteria2", node.node_label),
            public_parameters: PublicParameters::Hysteria2 {
                obfs_password: node.hysteria_obfs_password.map(|s| s.to_string()),
            },
            credentials: Some(node.credentials.clone()),
        });
    }
    out
}

/// SHA-256 hex digest over a canonical serialization of `endpoints` —
/// specifically the CLIENT-VISIBLE material (public key, short_id, obfs
/// password, host/port/SNI), never a server-private value (this crate's
/// `CompatEndpoint`/`PublicParameters` types structurally cannot hold a
/// private key — see `model.rs`).
///
/// Exists so a value computed from files on disk (what a FRESH read
/// would produce right now) can be compared against a value reported by
/// an ALREADY-RUNNING `vpn-subscription` process over its own
/// `/internal/state-fingerprint` endpoint (`services/subscription/src/
/// lib.rs`) — the only way to actually detect the incident class this
/// whole mechanism exists for: a running process serving stale
/// in-memory state it cached at its own startup, which no amount of
/// re-reading the current files from a *different* process (`vpn-admin`)
/// can observe. A hash, not the raw values, crosses that boundary: it
/// proves agreement/disagreement without ever transmitting or logging
/// the underlying key material itself.
pub fn endpoints_fingerprint(endpoints: &[CompatEndpoint]) -> String {
    let json = serde_json::to_string(endpoints).unwrap_or_default();
    crate::credentials::hash_token(&json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CompatTransport;
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
        }
    }

    fn reality_endpoint() -> CompatEndpoint {
        CompatEndpoint {
            id: "ep-reality".into(),
            transport: CompatTransport::VlessReality,
            host: "vpn.example.com".into(),
            port: 443,
            server_name: Some("www.google.com".into()),
            label: "Germany - Reality".into(),
            public_parameters: PublicParameters::Reality {
                public_key_hex: "abc123".into(),
                short_id: "0a1b2c3d".into(),
                fingerprint: "chrome".into(),
            },
            credentials: None,
        }
    }

    fn hysteria_endpoint() -> CompatEndpoint {
        CompatEndpoint {
            id: "ep-hy2".into(),
            transport: CompatTransport::Hysteria2,
            host: "vpn.example.com".into(),
            port: 443,
            server_name: Some("vpn.example.com".into()),
            label: "Germany - Hysteria2".into(),
            public_parameters: PublicParameters::Hysteria2 {
                obfs_password: None,
            },
            credentials: None,
        }
    }

    #[test]
    fn vless_uri_contains_required_reality_fields_and_no_private_key() {
        let uri = render_vless_reality_uri(&user(), &reality_endpoint()).unwrap();
        assert!(
            uri.starts_with("vless://11111111-1111-4111-8111-111111111111@vpn.example.com:443?")
        );
        assert!(uri.contains("security=reality"));
        assert!(uri.contains("pbk=abc123"));
        assert!(uri.contains("sid=0a1b2c3d"));
        assert!(uri.contains("flow=xtls-rprx-vision"));
        assert!(!uri.contains("private"));
    }

    #[test]
    fn hysteria2_uri_contains_password_and_sni() {
        let uri = render_hysteria2_uri(&user(), &hysteria_endpoint()).unwrap();
        assert!(uri.starts_with("hysteria2://hy2pass@vpn.example.com:443?"));
        assert!(uri.contains("sni=vpn.example.com"));
    }

    #[test]
    fn rendering_wrong_transport_for_endpoint_errors() {
        assert!(render_vless_reality_uri(&user(), &hysteria_endpoint()).is_err());
        assert!(render_hysteria2_uri(&user(), &reality_endpoint()).is_err());
    }

    #[test]
    fn uri_list_contains_both_transports() {
        let list = render_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("vless://"));
        assert!(lines[1].starts_with("hysteria2://"));
    }

    #[test]
    fn singbox_subscription_has_both_outbounds_and_urltest_selector() {
        let doc =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let types: Vec<&str> = outbounds
            .iter()
            .map(|o| o["type"].as_str().unwrap())
            .collect();
        assert!(types.contains(&"vless"));
        assert!(types.contains(&"hysteria2"));
        assert!(types.contains(&"urltest"));
        assert!(types.contains(&"selector"));
        let json_str = serde_json::to_string(&doc).unwrap();
        assert!(!json_str.to_lowercase().contains("private_key"));
    }

    #[test]
    fn route_final_points_at_manual_selector_not_urltest() {
        let doc =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        assert_eq!(doc["route"]["final"], "select");
    }

    #[test]
    fn selector_default_is_reality_and_lists_hysteria2_and_auto() {
        let doc =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let selector = outbounds
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present");
        assert_eq!(selector["tag"], "select");
        assert_eq!(
            selector["default"], "Germany - Reality",
            "REALITY must remain the deterministic default until measurements say otherwise"
        );
        let options: Vec<&str> = selector["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(options.contains(&"Germany - Reality"));
        assert!(options.contains(&"Germany - Hysteria2"));
        assert!(
            options.contains(&"auto"),
            "auto (urltest) must stay selectable, not be removed"
        );
    }

    #[test]
    fn selector_default_falls_back_to_first_endpoint_when_no_reality_endpoint_present() {
        // Defensive case: a reduced/experimental endpoint set with no
        // VLESS+REALITY endpoint at all must not panic and must still
        // produce a valid default rather than an empty/missing one.
        let doc = render_singbox_client_subscription(&user(), &[hysteria_endpoint()]).unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let selector = outbounds
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present");
        assert_eq!(selector["default"], "Germany - Hysteria2");
    }

    #[test]
    fn label_with_spaces_is_percent_encoded() {
        let uri = render_vless_reality_uri(&user(), &reality_endpoint()).unwrap();
        assert!(uri.ends_with("Germany%20-%20Reality"));
    }

    #[test]
    fn standard_endpoints_produces_reality_and_hysteria2() {
        let eps = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "pubkey",
            "short1",
            "www.google.com",
            None,
        );
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].transport, CompatTransport::VlessReality);
        assert_eq!(eps[1].transport, CompatTransport::Hysteria2);
        let PublicParameters::Hysteria2 { obfs_password } = &eps[1].public_parameters else {
            panic!("expected Hysteria2 parameters");
        };
        assert_eq!(
            obfs_password, &None,
            "no obfs password passed in must mean obfuscation stays disabled, not silently on"
        );
    }

    #[test]
    fn standard_endpoints_threads_hysteria2_obfs_password_into_uri_and_native_json() {
        let eps = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "pubkey",
            "short1",
            "www.google.com",
            Some("obfs-secret"),
        );
        let PublicParameters::Hysteria2 { obfs_password } = &eps[1].public_parameters else {
            panic!("expected Hysteria2 parameters");
        };
        assert_eq!(obfs_password.as_deref(), Some("obfs-secret"));

        let uri = render_hysteria2_uri(&user(), &eps[1]).unwrap();
        assert!(
            uri.contains("obfs=salamander&obfs-password=obfs-secret"),
            "share-link URI must carry the obfuscation params: {uri}"
        );

        let native = render_singbox_client_subscription(&user(), &eps).unwrap();
        let hy2_outbound = native["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["type"] == "hysteria2")
            .expect("hysteria2 outbound present");
        assert_eq!(hy2_outbound["obfs"]["type"], "salamander");
        assert_eq!(hy2_outbound["obfs"]["password"], "obfs-secret");
    }

    #[test]
    fn endpoints_fingerprint_is_deterministic_and_sensitive_to_key_material() {
        let a = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "pubkeyA",
            "short1",
            "www.google.com",
            None,
        );
        let a_again = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "pubkeyA",
            "short1",
            "www.google.com",
            None,
        );
        let b = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "pubkeyB", // different public key — simulates a stale-vs-current split
            "short1",
            "www.google.com",
            None,
        );
        assert_eq!(
            endpoints_fingerprint(&a),
            endpoints_fingerprint(&a_again),
            "same endpoint state must always fingerprint identically"
        );
        assert_ne!(
            endpoints_fingerprint(&a),
            endpoints_fingerprint(&b),
            "a different REALITY public key must change the fingerprint — this is the \
             property the live subscription/server coherence check in `vpn-admin doctor` \
             depends on to detect a stale running vpn-subscription process"
        );
    }

    fn node_a_credentials() -> crate::model::EndpointCredentials {
        crate::model::EndpointCredentials {
            vless_uuid: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
            hysteria2_password: Some(SecretString::new("node-a-hy2-pass")),
        }
    }

    fn node_b_credentials() -> crate::model::EndpointCredentials {
        crate::model::EndpointCredentials {
            vless_uuid: Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into()),
            hysteria2_password: Some(SecretString::new("node-b-hy2-pass")),
        }
    }

    fn two_node_endpoints() -> Vec<CompatEndpoint> {
        multi_node_endpoints(&[
            NodeSpec {
                node_label: "Node A",
                public_host: "node-a.example.com",
                reality_port: 443,
                hysteria_port: 443,
                reality_public_key_hex: "pubkey-a",
                reality_short_id: "shorta",
                handshake_server: "www.google.com",
                hysteria_obfs_password: Some("obfs-a"),
                credentials: node_a_credentials(),
            },
            NodeSpec {
                node_label: "Node B",
                public_host: "node-b.example.com",
                reality_port: 443,
                hysteria_port: 443,
                reality_public_key_hex: "pubkey-b",
                reality_short_id: "shortb",
                handshake_server: "www.bing.com",
                hysteria_obfs_password: Some("obfs-b"),
                credentials: node_b_credentials(),
            },
        ])
    }

    #[test]
    fn multi_node_endpoints_produces_four_distinctly_labeled_endpoints() {
        let eps = two_node_endpoints();
        assert_eq!(eps.len(), 4);
        let labels: Vec<&str> = eps.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Node A - Reality",
                "Node A - Hysteria2",
                "Node B - Reality",
                "Node B - Hysteria2",
            ]
        );
    }

    #[test]
    fn multi_node_subscription_uses_each_nodes_own_credentials_not_the_shared_user() {
        let eps = two_node_endpoints();
        // The rendering `CompatUser`'s own credentials must never leak into
        // a multi-node endpoint's rendered output — each node's
        // `EndpointCredentials` override must win.
        let shared_user = user();
        let doc = render_singbox_client_subscription(&shared_user, &eps).unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();

        let vless: Vec<&serde_json::Value> =
            outbounds.iter().filter(|o| o["type"] == "vless").collect();
        assert_eq!(vless.len(), 2);
        assert_eq!(vless[0]["uuid"], "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        assert_eq!(vless[1]["uuid"], "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        assert_ne!(vless[0]["uuid"], shared_user.vless_uuid);

        let hy2: Vec<&serde_json::Value> = outbounds
            .iter()
            .filter(|o| o["type"] == "hysteria2")
            .collect();
        assert_eq!(hy2.len(), 2);
        assert_eq!(hy2[0]["password"], "node-a-hy2-pass");
        assert_eq!(hy2[1]["password"], "node-b-hy2-pass");
        assert_ne!(hy2[0]["password"], "hy2pass"); // shared_user's own password

        // URI rendering must use the same per-node override, not the
        // shared user, so a Hiddify URI-list import is credential-isolated
        // the same way the native JSON is.
        let reality_a_uri = render_vless_reality_uri(&shared_user, &eps[0]).unwrap();
        assert!(reality_a_uri.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
        let hy2_b_uri = render_hysteria2_uri(&shared_user, &eps[3]).unwrap();
        assert!(hy2_b_uri.contains("node-b-hy2-pass"));
    }

    #[test]
    fn removing_node_b_endpoints_does_not_change_node_a_rendered_output() {
        let eps = two_node_endpoints();
        let shared_user = user();
        let with_both = render_singbox_client_subscription(&shared_user, &eps).unwrap();

        let node_a_only: Vec<CompatEndpoint> = eps
            .iter()
            .filter(|e| e.label.starts_with("Node A"))
            .cloned()
            .collect();
        let with_a_only = render_singbox_client_subscription(&shared_user, &node_a_only).unwrap();

        let find_vless = |doc: &serde_json::Value, tag: &str| -> serde_json::Value {
            doc["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["tag"] == tag)
                .unwrap()
                .clone()
        };
        assert_eq!(
            find_vless(&with_both, "Node A - Reality"),
            find_vless(&with_a_only, "Node A - Reality"),
            "Node A's rendered outbound must be identical whether or not Node B is present"
        );
    }

    #[test]
    fn removing_node_a_endpoints_does_not_change_node_b_rendered_output() {
        let eps = two_node_endpoints();
        let shared_user = user();
        let with_both = render_singbox_client_subscription(&shared_user, &eps).unwrap();

        let node_b_only: Vec<CompatEndpoint> = eps
            .iter()
            .filter(|e| e.label.starts_with("Node B"))
            .cloned()
            .collect();
        let with_b_only = render_singbox_client_subscription(&shared_user, &node_b_only).unwrap();

        let find_hy2 = |doc: &serde_json::Value, tag: &str| -> serde_json::Value {
            doc["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["tag"] == tag)
                .unwrap()
                .clone()
        };
        assert_eq!(
            find_hy2(&with_both, "Node B - Hysteria2"),
            find_hy2(&with_b_only, "Node B - Hysteria2"),
            "Node B's rendered outbound must be identical whether or not Node A is present"
        );
    }

    #[test]
    fn multi_node_selector_defaults_to_first_nodes_reality_and_lists_all_four_plus_auto() {
        let eps = two_node_endpoints();
        let doc = render_singbox_client_subscription(&user(), &eps).unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let selector = outbounds.iter().find(|o| o["type"] == "selector").unwrap();
        assert_eq!(selector["default"], "Node A - Reality");
        let options: Vec<&str> = selector["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            options,
            vec![
                "Node A - Reality",
                "Node A - Hysteria2",
                "Node B - Reality",
                "Node B - Hysteria2",
                "auto",
            ]
        );
        let urltest = outbounds.iter().find(|o| o["type"] == "urltest").unwrap();
        let urltest_options: Vec<&str> = urltest["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            urltest_options.len(),
            4,
            "auto group must race all four endpoints"
        );
    }

    #[test]
    fn multi_node_endpoints_never_serialize_credentials_field_when_absent() {
        // Single-node deployments (standard_endpoints) must keep producing
        // byte-identical serialized output to before this field existed —
        // `skip_serializing_if` must actually omit it, not serialize `null`.
        let eps = standard_endpoints(
            "vpn.example.com",
            443,
            443,
            "pubkey",
            "short1",
            "www.google.com",
            None,
        );
        let json = serde_json::to_string(&eps).unwrap();
        assert!(
            !json.contains("credentials"),
            "single-node endpoints must not serialize a `credentials` key at all: {json}"
        );
    }
}
