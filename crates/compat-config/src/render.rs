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
        uuid = user.vless_uuid,
        host = endpoint.host,
        port = endpoint.port,
        sni = sni,
        fp = fingerprint,
        pbk = public_key_hex,
        sid = short_id,
        label = percent_encode_label(&endpoint.label),
    ))
}

/// Label suffix carried by every EXPERIMENTAL Vision-off artifact this
/// module renders (share link and native-JSON outbound tag alike), so a
/// tester can always tell which profile is actually selected in their
/// client's UI. Follows the existing `" (Xray)"` labeling precedent: the
/// label is never sent over the wire, it is operator/tester bookkeeping.
pub const VISION_OFF_LABEL_SUFFIX: &str = " (EXPERIMENTAL Vision-off)";

/// EXPERIMENTAL, opt-in share link identical to `render_vless_reality_uri`
/// except that the `flow` parameter is OMITTED entirely (no
/// `flow=xtls-rprx-vision`) and the label carries
/// `VISION_OFF_LABEL_SUFFIX`. Same UUID, host, port, SNI, uTLS
/// fingerprint, REALITY public key and short ID — nothing else differs.
///
/// This exists for `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.5, whose
/// requirement is a profile where ONLY the Vision flow differs from
/// production. The existing `?compat=tcp-only` mode tests "disable UDP
/// relay entirely"; this tests a materially different variable — keep
/// UDP relay (Hysteria2 stays offered, VLESS keeps its normal network
/// handling), remove only XTLS Vision, which is what changes VLESS's
/// TLS-layer behavior (Vision's padding/splitting and its XUDP-only
/// handling of relayed UDP).
///
/// This is a DIAGNOSTIC, not a fix, and it is not free: Vision is
/// exactly the mechanism that hides the "TLS in TLS" pattern of a
/// proxied TLS session from a DPI observer, so a Vision-off profile is
/// more fingerprintable. It must not become anyone's default — see
/// `docs/clients/HIDDIFY_IOS.md`.
///
/// It also requires a matching server-side opt-in for the same user
/// (`CompatUser::vision_off_experiment`): sing-box's VLESS server
/// rejects a flow that does not equal the configured per-user flow.
pub fn render_vless_reality_uri_vision_off(
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
        "vless://{uuid}@{host}:{port}?encryption=none&security=reality&sni={sni}&fp={fp}&pbk={pbk}&sid={sid}&type=tcp#{label}",
        uuid = user.vless_uuid,
        host = endpoint.host,
        port = endpoint.port,
        sni = sni,
        fp = fingerprint,
        pbk = public_key_hex,
        sid = short_id,
        label = percent_encode_label(&format!("{}{VISION_OFF_LABEL_SUFFIX}", endpoint.label)),
    ))
}

/// `?format=uri&compat=vision-off` subscription body: every VLESS+REALITY
/// endpoint rendered via `render_vless_reality_uri_vision_off` (no `flow`
/// parameter, EXPERIMENTAL label suffix); every Hysteria2 endpoint
/// rendered unchanged via `render_hysteria2_uri` (Hysteria2 has no flow
/// concept, and this experiment deliberately does NOT remove UDP —
/// that's what `compat=tcp-only` is for).
pub fn render_vision_off_uri_list(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
) -> Result<String, CompatError> {
    let mut lines = Vec::with_capacity(endpoints.len());
    for ep in endpoints {
        let uri = match ep.transport {
            crate::model::CompatTransport::VlessReality => {
                render_vless_reality_uri_vision_off(user, ep)?
            }
            crate::model::CompatTransport::Hysteria2 => render_hysteria2_uri(user, ep)?,
        };
        lines.push(uri);
    }
    Ok(lines.join("\n"))
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
        password = user.hysteria2_password.expose(),
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

/// Explicit, opt-in A/B share link for Xray-core-oriented clients
/// (`?format=xray` — see `services/subscription/src/lib.rs`). This exists
/// for the 2026-08-19 Russia connectivity investigation
/// (`docs/RUSSIA_PRODUCTION_INVESTIGATION.md`): production self-tests pass
/// (a throwaway upstream sing-box client completes REALITY locally) but
/// real Russian clients see `REALITY: processed invalid connection`. One
/// untested variable is the client's TLS/REALITY implementation itself —
/// Hiddify on some platforms can bundle an Xray-core engine instead of
/// sing-box's own core, and the two are independent REALITY
/// implementations. This function renders the SAME UUID, public key,
/// short_id, SNI, host, and port as `render_vless_reality_uri` — it
/// changes NOTHING about server-side credentials or the wire format
/// (encryption=none, security=reality, type=tcp, flow=xtls-rprx-vision,
/// fp/pbk/sid/sni — that is already the standard `vless://` share-link
/// syntax both sing-box-core and Xray-core clients accept). It only
/// distinguishes the link with a separate, explicit "(Xray)" label suffix
/// so a Russian tester can import this link specifically to exercise
/// Hiddify's Xray-core engine, and the server operator can distinguish
/// which core connected in `sing-box` logs / `vpn-investigate.sh client`
/// output (endpoint label is not sent over the wire — the distinction is
/// operational bookkeeping for the person running the A/B test, not a
/// protocol-level difference).
///
/// **UNVERIFIED**: whether current Hiddify iOS/Android actually exposes a
/// distinct `core=xray` import path, an `xvless://` URI scheme, or any
/// other Xray-specific import syntax was NOT confirmed — this session had
/// no web access to check current Hiddify release notes/source. Rather
/// than invent unverified syntax, this deliberately emits the standard
/// `vless://` share-link (which both cores already consume identically)
/// under a distinct label, so it is safe to ship and test today. If real
/// Russian testing later shows Hiddify needs a different URI shape to
/// force its Xray-core engine specifically, that shape must be verified
/// against real Hiddify source/docs before being added here — see
/// `docs/RUSSIA_PRODUCTION_INVESTIGATION.md`.
pub fn render_vless_reality_uri_xray_labeled(
    user: &CompatUser,
    endpoint: &CompatEndpoint,
) -> Result<String, CompatError> {
    let mut xray_endpoint = endpoint.clone();
    xray_endpoint.label = format!("{} (Xray)", endpoint.label);
    render_vless_reality_uri(user, &xray_endpoint)
}

/// `?format=xray` subscription body: every VLESS+REALITY endpoint rendered
/// via `render_vless_reality_uri_xray_labeled` (same credentials, "(Xray)"
/// label suffix); every Hysteria2 endpoint rendered unchanged via
/// `render_hysteria2_uri` (Hysteria2's URI syntax is not sing-box/Xray-core
/// specific, so no separate labeling is needed there). See
/// `render_vless_reality_uri_xray_labeled`'s doc comment for the full
/// rationale and the explicit UNVERIFIED Hiddify-syntax caveat.
pub fn render_xray_uri_list(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
) -> Result<String, CompatError> {
    let mut lines = Vec::with_capacity(endpoints.len());
    for ep in endpoints {
        let uri = match ep.transport {
            crate::model::CompatTransport::VlessReality => {
                render_vless_reality_uri_xray_labeled(user, ep)?
            }
            crate::model::CompatTransport::Hysteria2 => render_hysteria2_uri(user, ep)?,
        };
        lines.push(uri);
    }
    Ok(lines.join("\n"))
}

/// Which endpoint the manual `select` outbound defaults to. This picks
/// ONLY the default — every profile still lists every real endpoint tag
/// plus `auto` (urltest) in the selector, so a user can always override
/// by hand regardless of profile (see `render_singbox_client_subscription`'s
/// doc comment for why `urltest` alone is never a safe silent default).
///
/// There is no data-driven "smart" auto mode here: `crates/network-state`
/// and `crates/failure-classifier` currently track only boolean
/// success/failure, not latency or throughput (see
/// docs/PERFORMANCE_OPTIMIZATION_PLAN.md), so `Auto` below means exactly
/// what sing-box's own `urltest` group means — a plain-HTTPS
/// latency/success race — not a throughput- or censorship-aware
/// selector. Advertising more than that without the underlying
/// measurements would be a false claim, not a feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectionProfile {
    /// Deterministic REALITY default (unchanged pre-existing behavior).
    /// The only profile safe to run as a fleet-wide default under active
    /// DPI — see docs/TELEGRAM_RESILIENCE_PLAN.md.
    #[default]
    Reliability,
    /// Deterministic Hysteria2 default. Opt-in only: Hysteria2/QUIC is
    /// more exposed to UDP blocking/throttling than REALITY's TCP/443
    /// disguise, so this trades some of that resilience for the
    /// generally higher throughput UDP/QUIC gets when it isn't blocked.
    Performance,
    /// Defaults the selector itself to sing-box's `auto` (urltest) group
    /// — a plain-HTTPS latency/success race between transports, nothing
    /// more (see this enum's doc comment). Still fully overridable by
    /// hand in the client.
    Auto,
}

impl SelectionProfile {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "reliability" => Some(Self::Reliability),
            "performance" => Some(Self::Performance),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// Explicit, opt-in compatibility mode for the generated sing-box
/// subscription — orthogonal to `SelectionProfile` (which endpoint the
/// selector *defaults* to). Where `SelectionProfile` never changes which
/// endpoints exist, `CompatibilityMode` can.
///
/// This exists for one specific, narrow symptom: on some iOS/Hiddify
/// installs, Safari can play YouTube over the normal subscription while
/// the native YouTube app cannot, over both VLESS+REALITY and Hysteria2.
/// The suspected cause is the YouTube app's own application-level
/// QUIC/UDP behavior, not a server-side or REALITY-key problem (server
/// protocol diagnostics already pass — see
/// `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`). `TcpOnly` gives an opt-in
/// way to test that theory: it removes every UDP-carrying option from
/// the profile the client can select, rather than adding an unverifiable
/// `route.rules` reject rule (see that same document for why the latter
/// was investigated and deliberately not shipped — Hiddify's actual
/// handling of imported `route.rules` cannot be verified from this
/// environment). Forcing the transport itself to be TCP-only is
/// enforced by construction (there is no UDP outbound left to fall back
/// to), not by a routing rule the client might silently ignore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CompatibilityMode {
    /// Unchanged, pre-existing behavior: both VLESS+REALITY and
    /// Hysteria2 are offered, exactly as before this mode existed.
    #[default]
    Normal,
    /// Hiddify/iOS YouTube-app compatibility mode. Hysteria2 (UDP/443,
    /// QUIC-based end to end) is dropped from the profile entirely, and
    /// the VLESS+REALITY outbound is rendered with `"network": "tcp"` —
    /// disabling sing-box VLESS's UDP-over-TCP relay for that outbound.
    /// REALITY's own transport connection is already TCP/443 by design
    /// (see `docs/CLIENT_PROTOCOL_BEHAVIOR.md`'s "UDP / TCP behavior"
    /// section); this only additionally forbids UDP relay *through* it.
    /// Everything else about the REALITY endpoint (UUID, flow, TLS,
    /// uTLS fingerprint, public key, short ID) is unchanged.
    TcpOnly,
    /// EXPERIMENTAL diagnostic for
    /// `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.5: the VLESS+REALITY
    /// outbound is rendered with NO `flow` field (instead of
    /// `xtls-rprx-vision`) and its tag carries
    /// `VISION_OFF_LABEL_SUFFIX`. Nothing else changes — same UUID, same
    /// REALITY public key/short ID, same SNI/uTLS fingerprint, same
    /// host/port, Hysteria2 still offered, no `network` restriction, no
    /// route rules. That is the point: `TcpOnly` tests "remove UDP relay
    /// entirely", this tests the materially different variable "keep UDP
    /// relay, remove only XTLS Vision".
    ///
    /// Two things make this mode more than a client-side toggle, and
    /// both are deliberate:
    ///  - It needs a matching per-user server-side opt-in
    ///    (`CompatUser::vision_off_experiment`, `server.rs`), because
    ///    sing-box's VLESS server rejects any flow that does not equal
    ///    the configured per-user flow ("flow mismatch").
    ///  - Vision is what conceals the TLS-in-TLS pattern of a proxied
    ///    TLS session, so a Vision-off profile is more fingerprintable
    ///    to DPI. Diagnostic only; never a default.
    VisionOff,
}

impl CompatibilityMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "normal" => Some(Self::Normal),
            "tcp-only" => Some(Self::TcpOnly),
            "vision-off" => Some(Self::VisionOff),
            _ => None,
        }
    }
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
    render_singbox_client_subscription_with_profile(user, endpoints, SelectionProfile::default())
}

/// Same as `render_singbox_client_subscription`, with the manual
/// selector's default chosen by `profile` instead of always REALITY —
/// see `SelectionProfile`'s doc comment for exactly what each variant
/// does and does not change.
pub fn render_singbox_client_subscription_with_profile(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
    profile: SelectionProfile,
) -> Result<serde_json::Value, CompatError> {
    render_singbox_client_subscription_with_options(
        user,
        endpoints,
        profile,
        CompatibilityMode::default(),
    )
}

/// Same as `render_singbox_client_subscription_with_profile`, additionally
/// taking a `CompatibilityMode`. `CompatibilityMode::Normal` reproduces the
/// exact prior behavior unchanged; `CompatibilityMode::TcpOnly` drops
/// Hysteria2 endpoints before the selector/urltest groups are built and
/// forces `"network": "tcp"` on the VLESS+REALITY outbound;
/// `CompatibilityMode::VisionOff` instead keeps every endpoint and every
/// other field and only drops the VLESS+REALITY outbound's `flow`
/// (labeling its tag EXPERIMENTAL) — see `CompatibilityMode`'s doc
/// comment for why each exists.
pub fn render_singbox_client_subscription_with_options(
    user: &CompatUser,
    endpoints: &[CompatEndpoint],
    profile: SelectionProfile,
    compat_mode: CompatibilityMode,
) -> Result<serde_json::Value, CompatError> {
    let filtered: Vec<&CompatEndpoint> = endpoints
        .iter()
        .filter(|ep| {
            !(compat_mode == CompatibilityMode::TcpOnly
                && matches!(ep.transport, CompatTransport::Hysteria2))
        })
        .collect();

    let mut outbounds = Vec::new();
    let mut tags = Vec::new();
    let mut reality_tag: Option<String> = None;
    let mut hysteria2_tag: Option<String> = None;
    for ep in filtered {
        // Only the VLESS+REALITY endpoint is labeled in VisionOff mode —
        // Hysteria2 has no flow concept and is rendered unchanged there.
        let tag = if compat_mode == CompatibilityMode::VisionOff
            && matches!(ep.transport, CompatTransport::VlessReality)
        {
            format!("{}{VISION_OFF_LABEL_SUFFIX}", ep.label)
        } else {
            ep.label.clone()
        };
        tags.push(tag.clone());
        if matches!(ep.transport, CompatTransport::VlessReality) && reality_tag.is_none() {
            reality_tag = Some(tag.clone());
        }
        if matches!(ep.transport, CompatTransport::Hysteria2) && hysteria2_tag.is_none() {
            hysteria2_tag = Some(tag.clone());
        }
        let outbound = match &ep.public_parameters {
            PublicParameters::Reality {
                public_key_hex,
                short_id,
                fingerprint,
            } => {
                let mut ob = json!({
                    "type": "vless",
                    "tag": tag,
                    "server": ep.host,
                    "server_port": ep.port,
                    "uuid": user.vless_uuid,
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
                });
                if compat_mode == CompatibilityMode::TcpOnly {
                    ob["network"] = json!("tcp");
                }
                if compat_mode == CompatibilityMode::VisionOff {
                    // Removed, not set to "" — an absent `flow` is
                    // sing-box's own default for a VLESS outbound, so
                    // this profile differs from the normal one by
                    // exactly one thing: Vision is not requested.
                    if let Some(obj) = ob.as_object_mut() {
                        obj.remove("flow");
                    }
                }
                ob
            }
            PublicParameters::Hysteria2 { obfs_password } => {
                let mut ob = json!({
                    "type": "hysteria2",
                    "tag": tag,
                    "server": ep.host,
                    "server_port": ep.port,
                    "password": user.hysteria2_password.expose(),
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
    // is chosen by `profile` (see `SelectionProfile`'s doc comment):
    // Reliability picks REALITY, Performance picks Hysteria2, Auto picks
    // the `auto` (urltest) group itself. Every profile falls back to the
    // first endpoint of any kind, then to `auto`, if its preferred
    // transport isn't present in this deployment's endpoint set — the
    // renderer must not panic on a reduced/experimental endpoint set.
    let mut selector_options = tags.clone();
    selector_options.push("auto".to_string());
    let default_tag = match profile {
        SelectionProfile::Reliability => reality_tag
            .or_else(|| tags.first().cloned())
            .unwrap_or_else(|| "auto".to_string()),
        SelectionProfile::Performance => hysteria2_tag
            .or(reality_tag)
            .or_else(|| tags.first().cloned())
            .unwrap_or_else(|| "auto".to_string()),
        SelectionProfile::Auto => "auto".to_string(),
    };
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
        },
    ]
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
            vision_off_experiment: false,
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

    /// docs/CLIENT_PROTOCOL_BEHAVIOR.md's DNS statement depends on this
    /// staying true: the generated subscription must never silently
    /// start expressing DNS/routing opinions it can't actually enforce
    /// or verify (the client app owns that entirely). Lock it in so a
    /// future change can't add a `dns` block without deliberately
    /// updating that doc's claims.
    #[test]
    fn client_subscription_has_no_dns_block_and_no_inbounds() {
        let doc =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        assert!(
            doc.get("dns").is_none(),
            "generated subscription must not claim to control DNS — see docs/CLIENT_PROTOCOL_BEHAVIOR.md"
        );
        assert!(
            doc.get("inbounds").is_none(),
            "generated subscription must not define a TUN/inbound — full-device tunneling is entirely client-controlled, see docs/CLIENT_PROTOCOL_BEHAVIOR.md"
        );
    }

    /// Locks in the P10 decision recorded in
    /// `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`: a `route.rules` array
    /// (e.g. an application-level UDP/443 reject rule to force
    /// QUIC-preferring apps onto TCP) was investigated and deliberately
    /// NOT added, because this project cannot verify Hiddify actually
    /// preserves an imported subscription's route rules from this
    /// environment — shipping it anyway would be exactly the kind of
    /// false confidence this project's diagnostics are trying to
    /// eliminate. If a future change adds `route.rules` for a
    /// "Compatibility" profile or otherwise, it must deliberately update
    /// this test and that document together, not silently regress the
    /// "subscription expresses no unverifiable routing opinion" boundary
    /// `client_subscription_has_no_dns_block_and_no_inbounds` already
    /// covers for `dns`/`inbounds`.
    #[test]
    fn client_subscription_never_emits_route_rules() {
        let doc =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        assert!(
            doc["route"].get("rules").is_none(),
            "generated subscription must not express route rules the client's actual behavior \
             cannot be verified against — see docs/COMPATIBILITY_QUIC_EXPERIMENT.md"
        );
    }

    /// Byte-shape contract for fields that clients are otherwise prone to
    /// silently supplying themselves.  Their absence is intentional: this
    /// outbounds-only document cannot control a client's TUN, DNS, MTU, mux,
    /// or platform routing policy.
    #[test]
    fn client_subscription_profile_shape_is_explicit_and_minimal() {
        let doc =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let reality = outbounds.iter().find(|o| o["type"] == "vless").unwrap();
        assert_eq!(reality["uuid"], "11111111-1111-4111-8111-111111111111");
        assert_eq!(reality["flow"], "xtls-rprx-vision");
        assert_eq!(reality["tls"]["server_name"], "www.google.com");
        assert_eq!(reality["tls"]["utls"]["fingerprint"], "chrome");
        assert_eq!(reality["tls"]["reality"]["public_key"], "abc123");
        assert_eq!(reality["tls"]["reality"]["short_id"], "0a1b2c3d");
        let urltest = outbounds.iter().find(|o| o["type"] == "urltest").unwrap();
        assert_eq!(urltest["url"], "https://www.gstatic.com/generate_204");

        let encoded = serde_json::to_string(&doc).unwrap();
        for forbidden in [
            "multiplex",
            "mux",
            "fragment",
            "padding",
            "packet_encoding",
            "tcp_fast_open",
            "tcp_keep_alive",
            "auto_route",
            "strict_route",
            "mtu",
            "inbounds",
            "dns",
        ] {
            assert!(
                !encoded.contains(&format!("\"{forbidden}\"")),
                "renderer unexpectedly emitted client-owned field {forbidden}: {encoded}"
            );
        }
    }

    /// Production responsibility-boundary contract. Optional compatibility
    /// modes may change offered transports, but none may turn the subscription
    /// into server-controlled client policy.
    #[test]
    fn every_subscription_mode_omits_client_owned_policy() {
        for mode in [
            CompatibilityMode::Normal,
            CompatibilityMode::TcpOnly,
            CompatibilityMode::VisionOff,
        ] {
            let doc = render_singbox_client_subscription_with_options(
                &user(),
                &[reality_endpoint(), hysteria_endpoint()],
                SelectionProfile::Reliability,
                mode,
            )
            .unwrap();

            assert!(doc.get("dns").is_none(), "{mode:?} emitted DNS policy");
            assert!(
                doc.get("inbounds").is_none(),
                "{mode:?} emitted client inbound/TUN policy"
            );
            assert!(
                doc["route"].get("rules").is_none(),
                "{mode:?} emitted route rules"
            );

            let encoded = serde_json::to_string(&doc).unwrap();
            for field in ["mtu", "auto_route", "strict_route"] {
                assert!(
                    !encoded.contains(&format!("\"{field}\"")),
                    "{mode:?} emitted client-owned field {field}: {encoded}"
                );
            }
        }
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

    /// docs/COMPATIBILITY_SECURITY_REVIEW.md's "As a DPI/censor" section
    /// names this exact scenario ("UDP blocked entirely? Hysteria2 fails;
    /// VLESS+REALITY (TCP/443) keeps working") as a documented but
    /// previously untested claim — this is the structural-level proof
    /// that's actually achievable without real network/namespace testing
    /// (not available in this environment): a profile with Hysteria2
    /// entirely absent (modeling "Hysteria2 is unreachable on this
    /// network, only REALITY endpoints remain in a filtered/reduced
    /// endpoint set") still produces a complete, valid, REALITY-default
    /// profile — the profile does not become unusable just because one
    /// transport is gone. This does NOT prove real UDP blocking on a
    /// real network leaves REALITY reachable — that remains an open
    /// manual test (see docs/DEVICE_ACCEPTANCE_TESTS.md's IPv4/IPv6 and
    /// network-switch rows) — it proves the config-generation layer
    /// never conflates "one transport unavailable" with "whole profile
    /// broken".
    #[test]
    fn hysteria2_unavailable_reality_only_profile_remains_fully_usable() {
        let doc = render_singbox_client_subscription(&user(), &[reality_endpoint()]).unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let types: Vec<&str> = outbounds
            .iter()
            .map(|o| o["type"].as_str().unwrap())
            .collect();
        assert!(types.contains(&"vless"), "REALITY outbound still present");
        assert!(
            !types.contains(&"hysteria2"),
            "no hysteria2 outbound when it's genuinely not offered"
        );
        let selector = outbounds
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present even with only one transport");
        assert_eq!(
            selector["default"], "Germany - Reality",
            "REALITY remains the deterministic default with no other transport in play"
        );
        assert_eq!(
            doc["route"]["final"], "select",
            "route still points at a usable selector, not an empty/broken group"
        );
    }

    #[test]
    fn performance_profile_defaults_selector_to_hysteria2() {
        let doc = render_singbox_client_subscription_with_profile(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::Performance,
        )
        .unwrap();
        let selector = doc["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present");
        assert_eq!(selector["default"], "Germany - Hysteria2");
        // still fully overridable — REALITY and auto remain listed.
        let options: Vec<&str> = selector["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(options.contains(&"Germany - Reality"));
        assert!(options.contains(&"auto"));
    }

    #[test]
    fn auto_profile_defaults_selector_to_urltest_group() {
        let doc = render_singbox_client_subscription_with_profile(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::Auto,
        )
        .unwrap();
        let selector = doc["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present");
        assert_eq!(selector["default"], "auto");
        // route.final still points at the manual selector, not directly
        // at urltest — the selector's default merely equals "auto" here,
        // so a client tapping the selector UI still sees every option.
        assert_eq!(doc["route"]["final"], "select");
    }

    #[test]
    fn reliability_profile_matches_default_profile_behavior() {
        let explicit = render_singbox_client_subscription_with_profile(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::Reliability,
        )
        .unwrap();
        let implicit =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        assert_eq!(explicit, implicit);
    }

    #[test]
    fn selection_profile_parse_rejects_unknown_values() {
        assert_eq!(
            SelectionProfile::parse("reliability"),
            Some(SelectionProfile::Reliability)
        );
        assert_eq!(
            SelectionProfile::parse("performance"),
            Some(SelectionProfile::Performance)
        );
        assert_eq!(
            SelectionProfile::parse("auto"),
            Some(SelectionProfile::Auto)
        );
        assert_eq!(SelectionProfile::parse("bogus"), None);
    }

    #[test]
    fn label_with_spaces_is_percent_encoded() {
        let uri = render_vless_reality_uri(&user(), &reality_endpoint()).unwrap();
        assert!(uri.ends_with("Germany%20-%20Reality"));
    }

    // --- Xray-core-oriented A/B share link (?format=xray) ---

    #[test]
    fn xray_labeled_uri_has_xray_suffix_and_same_credentials_as_normal_uri() {
        let normal = render_vless_reality_uri(&user(), &reality_endpoint()).unwrap();
        let xray = render_vless_reality_uri_xray_labeled(&user(), &reality_endpoint()).unwrap();
        assert_ne!(
            normal, xray,
            "xray-labeled URI must differ from the normal one (label only)"
        );
        assert!(
            xray.starts_with("vless://11111111-1111-4111-8111-111111111111@vpn.example.com:443?"),
            "same UUID/host/port: {xray}"
        );
        assert!(xray.contains("security=reality"));
        assert!(xray.contains("encryption=none"));
        assert!(xray.contains("type=tcp"));
        assert!(xray.contains("sni=www.google.com"));
        assert!(xray.contains("fp=chrome"));
        assert!(xray.contains("pbk=abc123"));
        assert!(xray.contains("sid=0a1b2c3d"));
        assert!(xray.contains("flow=xtls-rprx-vision"));
        assert!(
            xray.ends_with("Germany%20-%20Reality%20%28Xray%29"),
            "label must be distinctly suffixed (percent-encoded parens): {xray}"
        );
        assert!(!xray.to_lowercase().contains("private"));
    }

    #[test]
    fn xray_labeled_uri_never_leaks_private_key() {
        let xray = render_vless_reality_uri_xray_labeled(&user(), &reality_endpoint()).unwrap();
        assert!(!xray.to_lowercase().contains("private"));
        assert!(!xray.to_lowercase().contains("private_key"));
    }

    #[test]
    fn xray_labeled_uri_rejects_wrong_transport() {
        assert!(render_vless_reality_uri_xray_labeled(&user(), &hysteria_endpoint()).is_err());
    }

    #[test]
    fn xray_uri_list_labels_only_reality_endpoints() {
        let list =
            render_xray_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("vless://"));
        assert!(
            lines[0].contains("%28Xray%29"),
            "REALITY line must carry the percent-encoded Xray label: {}",
            lines[0]
        );
        assert!(lines[1].starts_with("hysteria2://"));
        assert!(
            !lines[1].contains("Xray"),
            "Hysteria2 line must be unchanged/unlabeled: {}",
            lines[1]
        );
    }

    #[test]
    fn xray_uri_list_never_leaks_private_key() {
        let list =
            render_xray_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        assert!(!list.to_lowercase().contains("private"));
    }

    #[test]
    fn xray_uri_list_uses_exact_same_credentials_as_default_uri_list_besides_label() {
        let normal = render_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        let xray =
            render_xray_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        // Strip only the trailing label fragment (after '#') and compare —
        // everything before it (uuid/host/port/query params) must be identical.
        let strip_label = |uri: &str| uri.split('#').next().unwrap().to_string();
        let normal_lines: Vec<String> = normal.lines().map(strip_label).collect();
        let xray_lines: Vec<String> = xray.lines().map(strip_label).collect();
        assert_eq!(
            normal_lines, xray_lines,
            "only the fragment/label may differ between the default and Xray-oriented URI lists"
        );
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

    // --- CompatibilityMode::TcpOnly ---

    #[test]
    fn compatibility_mode_parse_rejects_unknown_values() {
        assert_eq!(
            CompatibilityMode::parse("normal"),
            Some(CompatibilityMode::Normal)
        );
        assert_eq!(
            CompatibilityMode::parse("tcp-only"),
            Some(CompatibilityMode::TcpOnly)
        );
        assert_eq!(CompatibilityMode::parse("garbage"), None);
    }

    #[test]
    fn normal_mode_via_with_options_is_identical_to_existing_renderer() {
        let via_options = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::Normal,
        )
        .unwrap();
        let existing =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        assert_eq!(
            via_options, existing,
            "CompatibilityMode::Normal must reproduce the pre-existing subscription exactly"
        );
    }

    #[test]
    fn normal_mode_keeps_vless_and_hysteria2_and_no_network_field() {
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
        let vless = outbounds.iter().find(|o| o["type"] == "vless").unwrap();
        assert!(
            vless.get("network").is_none(),
            "normal mode must not unexpectedly acquire network=tcp on the VLESS outbound"
        );
        assert!(
            doc["route"].get("rules").is_none(),
            "normal mode must not gain route.rules"
        );
    }

    #[test]
    fn tcp_only_mode_sets_vless_network_tcp_and_drops_hysteria2() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let types: Vec<&str> = outbounds
            .iter()
            .map(|o| o["type"].as_str().unwrap())
            .collect();
        assert!(types.contains(&"vless"), "REALITY outbound still present");
        assert!(
            !types.contains(&"hysteria2"),
            "TcpOnly must drop Hysteria2 — it depends on UDP end to end"
        );
        let vless = outbounds.iter().find(|o| o["type"] == "vless").unwrap();
        assert_eq!(vless["network"], "tcp");
        // REALITY parameters must be otherwise unchanged.
        assert_eq!(vless["uuid"], "11111111-1111-4111-8111-111111111111");
        assert_eq!(vless["flow"], "xtls-rprx-vision");
        assert_eq!(vless["tls"]["reality"]["public_key"], "abc123");
        assert_eq!(vless["tls"]["reality"]["short_id"], "0a1b2c3d");
        assert_eq!(vless["tls"]["utls"]["fingerprint"], "chrome");
    }

    #[test]
    fn tcp_only_mode_does_not_add_packet_encoding_xudp() {
        // xudp is sing-box's normal VLESS UDP behavior — it does not
        // address the symptom this mode targets, and must not be added
        // as a supposed fix.
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        let encoded = serde_json::to_string(&doc).unwrap();
        assert!(!encoded.contains("packet_encoding"));
    }

    #[test]
    fn tcp_only_mode_selector_lists_only_remaining_tags_and_defaults_to_reality() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let selector = outbounds
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present");
        assert_eq!(selector["default"], "Germany - Reality");
        let options: Vec<&str> = selector["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(options.contains(&"Germany - Reality"));
        assert!(
            !options.contains(&"Germany - Hysteria2"),
            "selector must not reference a dropped Hysteria2 tag: {options:?}"
        );
        assert!(options.contains(&"auto"), "auto/urltest must stay valid");

        let urltest = outbounds.iter().find(|o| o["type"] == "urltest").unwrap();
        let urltest_options: Vec<&str> = urltest["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            urltest_options,
            vec!["Germany - Reality"],
            "urltest group must not reference a dropped Hysteria2 tag"
        );
    }

    #[test]
    fn tcp_only_mode_route_final_is_valid_and_emits_no_route_rules() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        assert_eq!(doc["route"]["final"], "select");
        assert!(
            doc["route"].get("rules").is_none(),
            "TcpOnly must not add a route.rules UDP/443 reject rule — enforcement is via the \
             outbound's own network field, not an unverifiable client-side routing rule, see \
             docs/COMPATIBILITY_QUIC_EXPERIMENT.md"
        );
    }

    #[test]
    fn tcp_only_mode_emits_no_dns_block_or_tun_inbound() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        assert!(doc.get("dns").is_none());
        assert!(doc.get("inbounds").is_none());
    }

    #[test]
    fn tcp_only_mode_leaks_no_private_reality_material() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        let encoded = serde_json::to_string(&doc).unwrap();
        assert!(!encoded.to_lowercase().contains("private"));
        assert!(!encoded.to_lowercase().contains("private_key"));
    }

    #[test]
    fn tcp_only_mode_reality_only_deployment_still_produces_valid_profile() {
        // A reduced endpoint set with only REALITY (no Hysteria2 offered
        // at all) must still work under TcpOnly — same defensive contract
        // as the existing `hysteria2_unavailable_reality_only_profile_
        // remains_fully_usable` test, now also exercised under TcpOnly.
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::TcpOnly,
        )
        .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        assert!(outbounds.iter().any(|o| o["type"] == "vless"));
        let selector = outbounds.iter().find(|o| o["type"] == "selector").unwrap();
        assert_eq!(selector["default"], "Germany - Reality");
        assert_eq!(doc["route"]["final"], "select");
    }

    // --- CompatibilityMode::VisionOff (EXPERIMENTAL, §9.5 diagnostic) ---

    #[test]
    fn compatibility_mode_parse_accepts_vision_off_and_still_rejects_unknown_values() {
        assert_eq!(
            CompatibilityMode::parse("vision-off"),
            Some(CompatibilityMode::VisionOff)
        );
        // Near-misses must not silently resolve to the experimental mode.
        assert_eq!(CompatibilityMode::parse("vision_off"), None);
        assert_eq!(CompatibilityMode::parse("visionoff"), None);
        assert_eq!(CompatibilityMode::parse("garbage"), None);
    }

    /// The critical guarantee: adding VisionOff must not have changed the
    /// pre-existing default output by a single byte. Mirrors
    /// `normal_mode_via_with_options_is_identical_to_existing_renderer`.
    #[test]
    fn normal_mode_output_is_byte_for_byte_unchanged_by_the_vision_off_mode_existing() {
        let via_options = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::Normal,
        )
        .unwrap();
        let existing =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        assert_eq!(
            serde_json::to_string(&via_options).unwrap(),
            serde_json::to_string(&existing).unwrap(),
            "CompatibilityMode::Normal must still serialize byte-for-byte identically to the \
             pre-existing renderer"
        );
        let encoded = serde_json::to_string(&existing).unwrap();
        assert!(
            encoded.contains("\"flow\":\"xtls-rprx-vision\""),
            "the normal profile must still request Vision: {encoded}"
        );
        assert!(
            !encoded.contains("EXPERIMENTAL"),
            "the normal profile must never carry the experimental label: {encoded}"
        );
    }

    #[test]
    fn vision_off_mode_omits_flow_and_changes_nothing_else_about_the_reality_outbound() {
        let normal = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::Normal,
        )
        .unwrap();
        let vision_off = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::VisionOff,
        )
        .unwrap();

        let find = |doc: &serde_json::Value, ty: &str| {
            doc["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["type"] == ty)
                .cloned()
                .expect("outbound present")
        };
        let normal_vless = find(&normal, "vless");
        let mut vision_off_vless = find(&vision_off, "vless");

        assert!(
            vision_off_vless.get("flow").is_none(),
            "vision-off must omit the flow field entirely: {vision_off_vless}"
        );
        assert_eq!(normal_vless["flow"], "xtls-rprx-vision");
        assert!(
            vision_off_vless.get("network").is_none(),
            "vision-off must NOT restrict the network — that's compat=tcp-only's job"
        );

        // Everything except `flow` and the deliberately-labeled tag must
        // be identical to the normal profile's REALITY outbound.
        let mut expected = normal_vless.clone();
        expected.as_object_mut().unwrap().remove("flow");
        expected["tag"] = json!(format!("Germany - Reality{VISION_OFF_LABEL_SUFFIX}"));
        assert_eq!(
            vision_off_vless, expected,
            "only `flow` (removed) and the EXPERIMENTAL tag suffix may differ"
        );

        // ... and the tag difference really is only the suffix.
        assert_eq!(
            vision_off_vless["tag"],
            "Germany - Reality (EXPERIMENTAL Vision-off)"
        );
        vision_off_vless["tag"] = json!("Germany - Reality");
        expected["tag"] = json!("Germany - Reality");
        assert_eq!(vision_off_vless, expected);
    }

    #[test]
    fn vision_off_mode_keeps_hysteria2_and_udp_capability_unlike_tcp_only() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::VisionOff,
        )
        .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let types: Vec<&str> = outbounds
            .iter()
            .map(|o| o["type"].as_str().unwrap())
            .collect();
        assert!(
            types.contains(&"hysteria2"),
            "vision-off tests a different variable than tcp-only: UDP transports stay offered"
        );
        // Hysteria2's own outbound must be byte-identical to normal mode.
        let normal =
            render_singbox_client_subscription(&user(), &[reality_endpoint(), hysteria_endpoint()])
                .unwrap();
        let hy2 = |doc: &serde_json::Value| {
            doc["outbounds"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["type"] == "hysteria2")
                .cloned()
                .unwrap()
        };
        assert_eq!(hy2(&doc), hy2(&normal));
    }

    #[test]
    fn vision_off_mode_selector_and_urltest_reference_the_labeled_reality_tag() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::VisionOff,
        )
        .unwrap();
        let outbounds = doc["outbounds"].as_array().unwrap();
        let selector = outbounds
            .iter()
            .find(|o| o["type"] == "selector")
            .expect("selector outbound present");
        assert_eq!(
            selector["default"], "Germany - Reality (EXPERIMENTAL Vision-off)",
            "the selector must default to a tag that actually exists"
        );
        let options: Vec<&str> = selector["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(options.contains(&"Germany - Reality (EXPERIMENTAL Vision-off)"));
        assert!(options.contains(&"Germany - Hysteria2"));
        assert!(options.contains(&"auto"));

        let urltest = outbounds.iter().find(|o| o["type"] == "urltest").unwrap();
        let urltest_options: Vec<&str> = urltest["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            urltest_options,
            vec![
                "Germany - Reality (EXPERIMENTAL Vision-off)",
                "Germany - Hysteria2"
            ],
            "no group may reference a tag no outbound carries"
        );
        assert_eq!(doc["route"]["final"], "select");
    }

    #[test]
    fn vision_off_mode_adds_no_route_rules_dns_inbounds_or_client_owned_fields() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::VisionOff,
        )
        .unwrap();
        assert!(doc["route"].get("rules").is_none());
        assert!(doc.get("dns").is_none());
        assert!(doc.get("inbounds").is_none());
        let encoded = serde_json::to_string(&doc).unwrap();
        for forbidden in ["packet_encoding", "multiplex", "mux", "fragment", "mtu"] {
            assert!(
                !encoded.contains(&format!("\"{forbidden}\"")),
                "vision-off unexpectedly emitted {forbidden}: {encoded}"
            );
        }
    }

    #[test]
    fn vision_off_mode_leaks_no_private_reality_material() {
        let doc = render_singbox_client_subscription_with_options(
            &user(),
            &[reality_endpoint(), hysteria_endpoint()],
            SelectionProfile::default(),
            CompatibilityMode::VisionOff,
        )
        .unwrap();
        let encoded = serde_json::to_string(&doc).unwrap().to_lowercase();
        assert!(!encoded.contains("private"));
    }

    #[test]
    fn vision_off_uri_omits_flow_and_keeps_every_other_parameter_identical() {
        let normal = render_vless_reality_uri(&user(), &reality_endpoint()).unwrap();
        let vision_off = render_vless_reality_uri_vision_off(&user(), &reality_endpoint()).unwrap();
        assert!(
            !vision_off.contains("flow="),
            "vision-off share link must carry no flow parameter: {vision_off}"
        );
        assert!(normal.contains("flow=xtls-rprx-vision"));
        assert!(
            vision_off
                .starts_with("vless://11111111-1111-4111-8111-111111111111@vpn.example.com:443?"),
            "same UUID/host/port: {vision_off}"
        );
        assert!(vision_off.contains("encryption=none"));
        assert!(vision_off.contains("security=reality"));
        assert!(vision_off.contains("type=tcp"));
        assert!(vision_off.contains("sni=www.google.com"));
        assert!(vision_off.contains("fp=chrome"));
        assert!(vision_off.contains("pbk=abc123"));
        assert!(vision_off.contains("sid=0a1b2c3d"));
        assert!(
            vision_off.ends_with("Germany%20-%20Reality%20%28EXPERIMENTAL%20Vision-off%29"),
            "label must be distinctly suffixed (percent-encoded): {vision_off}"
        );
        assert!(!vision_off.to_lowercase().contains("private"));

        // Query strings must be identical apart from the removed `flow`.
        let strip = |uri: &str| {
            uri.split('#')
                .next()
                .unwrap()
                .replace("&flow=xtls-rprx-vision", "")
        };
        assert_eq!(strip(&normal), strip(&vision_off));
    }

    #[test]
    fn vision_off_uri_rejects_wrong_transport() {
        assert!(render_vless_reality_uri_vision_off(&user(), &hysteria_endpoint()).is_err());
    }

    #[test]
    fn vision_off_uri_list_labels_only_reality_endpoints_and_leaves_hysteria2_unchanged() {
        let normal = render_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        let list = render_vision_off_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()])
            .unwrap();
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("vless://"));
        assert!(!lines[0].contains("flow="));
        assert!(lines[0].contains("%28EXPERIMENTAL%20Vision-off%29"));
        assert_eq!(
            lines[1],
            normal.lines().nth(1).unwrap(),
            "the Hysteria2 line must be byte-identical to the normal list"
        );
        assert!(!list.to_lowercase().contains("private"));
    }

    /// The normal share-link list must be untouched by the existence of
    /// the vision-off one — same guarantee the native-JSON path gets.
    #[test]
    fn normal_uri_list_is_unchanged_and_still_requests_vision() {
        let normal = render_uri_list(&user(), &[reality_endpoint(), hysteria_endpoint()]).unwrap();
        assert!(normal
            .lines()
            .next()
            .unwrap()
            .contains("flow=xtls-rprx-vision"));
        assert!(!normal.contains("EXPERIMENTAL"));
    }
}
