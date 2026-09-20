//! What HIDDIFY actually runs, not what we serve.
//!
//! Hiddify does not execute an imported sing-box config. `BuildConfig`
//! (hiddify-core `v2/config/builder.go`) reads the `outbounds` array,
//! discards everything else, and rebuilds inbounds, DNS, routing and the
//! proxy groups from its own options. Asserting on the JSON we serve
//! therefore proves nothing about the user's device — which is exactly
//! how `compat=quic-reject` shipped as a "fix" for a rule Hiddify throws
//! away (`docs/YOUTUBE_FINAL_ROOT_CAUSE.md`).
//!
//! This suite closes that gap: [`hiddify_runtime`] is a faithful model of
//! the parts of that rebuild which decide what our profile turns into,
//! transcribed from the pinned upstream revisions below, and the tests
//! assert on ITS output.
//!
//! Pinned upstream revisions this model was read from:
//!   * hiddify-core `db74dfc257d5becb4b4e9dbc7257a3dcdde20692`
//!   * hiddify-app  `276a7effb0046a039220a745022563740968c0b8`
//!
//! It is a model, not the real core. It is evidence about hiddify-core's
//! source, not device acceptance — see `docs/DEVICE_ACCEPTANCE_TESTS.md`.

use compat_config::contract::{
    provisioning_document_with_mode_and_access_paths_and_options, DiagnosticMode,
};
use compat_config::deployment::DeploymentConfig;
use compat_config::model::{CompatUser, PeerCredential};
use compat_config::render::{
    render_singbox_client_subscription_with_options, standard_endpoints, CompatibilityMode,
    SelectionProfile, HIDDIFY_HIDDEN_TAG_SUFFIX,
};
use compat_config::secret::SecretString;
use provisioning_contract::AccessPath;
use serde_json::Value;

/// What Hiddify's rebuilt config routes traffic through.
#[derive(Debug)]
struct HiddifyRuntime {
    /// Tags Hiddify offers the user and builds its groups from —
    /// `tags` in `setOutbounds`.
    selectable: Vec<String>,
    /// The tag `select` defaults to, which `route.final` points at.
    /// `"balance"` means a per-connection round-robin group.
    default_route: String,
    /// Members of that round-robin group; empty when no balancer exists.
    balanced_over: Vec<String>,
    /// Route rules present in the runtime config that came from OUR
    /// config. Always empty: `setRoutingOptions` assigns
    /// `options.Route = &option.RouteOptions{...}` unconditionally.
    imported_route_rules: Vec<Value>,
}

/// hiddify-core `db74dfc`, `v2/config/builder.go`.
///
/// Transcribed behavior, in source order:
///
///  * `BuildConfig`: `options.Route` is only seeded from the import when
///    `enable-full-config` is set, and `setRoutingOptions` then
///    overwrites `options.Route` wholesale either way — so an imported
///    `route` object never reaches the runtime. Modeled as
///    `imported_route_rules: vec![]`.
///  * `setOutbounds`: `case C.TypeSelector, C.TypeURLTest: continue` —
///    our `selector`/`urltest` groups and our `route.final` are dropped,
///    so we cannot pin a route that way.
///  * `setOutbounds`: `if contains([]string{"direct","bypass","block"}, out.Tag)`
///    and the `PredefinedOutboundTags` check skip those tags.
///  * `setOutbounds`: `if !strings.Contains(out.Tag, "§hide§") { tags = append(...) }`
///    — a `§hide§` tag stays in the config (usable as a `detour`) but is
///    never selectable and never a group member.
///  * `setOutbounds`: `if len(tags) > 1 { ...; defaultSelect = balancer.Tag }`
///    else `defaultSelect = tags[0]`. The balancer's strategy is
///    hiddify-app's default `round-robin`
///    (`lib/features/settings/data/config_option_repository.dart`), and
///    `Balancer::DialContext` selects a member PER CONNECTION.
fn hiddify_runtime(config: &Value) -> HiddifyRuntime {
    const PREDEFINED: [&str; 7] = [
        "direct §hide§",
        "bypass §hide§",
        "select",
        "lowest",
        "dns-out §hide§",
        "direct-fragment §hide§",
        "🔒 WARP",
    ];
    let mut selectable = Vec::new();
    for outbound in config["outbounds"].as_array().expect("outbounds array") {
        let tag = outbound["tag"].as_str().expect("every outbound has a tag");
        let kind = outbound["type"]
            .as_str()
            .expect("every outbound has a type");
        if PREDEFINED.contains(&tag) {
            continue;
        }
        if matches!(kind, "selector" | "urltest" | "block" | "dns" | "custom") {
            continue;
        }
        if matches!(tag, "direct" | "bypass" | "block") {
            continue;
        }
        if tag.contains("§hide§") {
            continue;
        }
        selectable.push(tag.to_string());
    }
    let (default_route, balanced_over) = if selectable.len() > 1 {
        ("balance".to_string(), selectable.clone())
    } else {
        (selectable.first().cloned().unwrap_or_default(), Vec::new())
    };
    HiddifyRuntime {
        selectable,
        default_route,
        balanced_over,
        imported_route_rules: Vec::new(),
    }
}

fn user() -> CompatUser {
    let mut peer_credentials = std::collections::BTreeMap::new();
    peer_credentials.insert(
        "de1-direct".to_string(),
        PeerCredential::VlessReality {
            uuid: "22222222-2222-4222-8222-222222222222".to_string(),
        },
    );
    CompatUser {
        id: "u1".into(),
        name: "test".into(),
        enabled: true,
        vless_uuid: "11111111-1111-4111-8111-111111111111".into(),
        hysteria2_password: SecretString::new("hy2"),
        subscription_token_hash_hex: "hash".into(),
        created_at: 0,
        expires_at: None,
        vision_off_experiment: false,
        google_egress_hairpin: false,
        peer_credentials,
    }
}

fn direct_endpoints() -> Vec<compat_config::model::CompatEndpoint> {
    standard_endpoints(
        "de1.example.com",
        443,
        8443,
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
        "11223344",
        "www.cloudflare.com",
        None,
    )
}

fn direct_profile(mode: CompatibilityMode) -> Value {
    render_singbox_client_subscription_with_options(
        &user(),
        &direct_endpoints(),
        SelectionProfile::default(),
        mode,
    )
    .expect("render direct profile")
}

const PRIVACY_DEPLOYMENT: &str = r#"
schema_version = 2
node_id = "ru1"
role = "relay"
public_host = "ru1.example.com"
subscription_host = "ru1.example.com"

[reality]
listen_port = 443
handshake_server = "www.cloudflare.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 9100

[[access_paths]]
id = "relay-ru1"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]

[[peer_endpoints]]
id = "de1-direct"
tag = "Germany · Direct"
host = "de1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "direct"

[[peer_endpoints]]
id = "de1-via-ru1"
tag = "Germany · via Russia"
host = "de1.example.com"
port = 443
transport = "vless_reality"
server_name = "www.cloudflare.com"
reality_public_key = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
reality_short_id = "0a1b2c3d"
failure_domain = "exit:de1"
path = "relay-ru1"
credential_ref = "de1-direct"
"#;

fn privacy_profile(mode: CompatibilityMode) -> Value {
    let cfg: DeploymentConfig = toml::from_str(PRIVACY_DEPLOYMENT).unwrap();
    cfg.validate().unwrap();
    let mut endpoints = standard_endpoints(
        &cfg.public_host,
        443,
        443,
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
        "11223344",
        "www.cloudflare.com",
        None,
    );
    endpoints.retain(|endpoint| endpoint.id == "reality-1");
    endpoints.extend(
        cfg.peer_endpoints
            .iter()
            .map(|peer| peer.to_compat_endpoint().unwrap()),
    );
    let paths: Vec<AccessPath> = cfg
        .access_paths
        .iter()
        .map(|path| path.to_contract_access_path().unwrap())
        .collect();
    provisioning_document_with_mode_and_access_paths_and_options(
        &user(),
        &endpoints,
        DiagnosticMode::None,
        &paths,
        SelectionProfile::default(),
        mode,
    )
    .expect("render privacy+ profile")
    .singbox_config
    .expect("privacy+ profile embeds a Core config")
}

// --- the defect ------------------------------------------------------

/// The falsification, kept as an executable assertion rather than a
/// paragraph: the profile we have always served does NOT give a Hiddify
/// user a route. It gives them a per-connection round-robin across every
/// endpoint in the profile, because `select`'s default becomes the
/// `balance` group as soon as more than one tag survives the rebuild.
///
/// This is what a YouTube session meets: one playback opens many parallel
/// connections to `*.googlevideo.com`, and each one is dialed through a
/// different member.
#[test]
fn normal_profile_is_round_robin_balanced_by_current_hiddify() {
    let runtime = hiddify_runtime(&direct_profile(CompatibilityMode::Normal));
    assert!(
        runtime.selectable.len() > 1,
        "fixture must be a multi-route profile for this to mean anything, got {:?}",
        runtime.selectable
    );
    assert_eq!(
        runtime.default_route, "balance",
        "hiddify-core makes the balancer the default route whenever len(tags) > 1"
    );
    assert_eq!(runtime.balanced_over, runtime.selectable);
}

/// `compat=quic-reject` cannot do anything on a Hiddify device: the rule
/// is real, valid sing-box, present in what we serve — and discarded
/// before the core ever sees it. Recorded as a test so nobody ships a
/// second route-rule "fix" for this client.
#[test]
fn quic_reject_route_rule_never_reaches_the_hiddify_runtime() {
    let served = direct_profile(CompatibilityMode::QuicReject);
    assert!(
        served["route"]["rules"].is_array(),
        "we really do serve the rule — the defect is downstream of us"
    );
    let runtime = hiddify_runtime(&served);
    assert!(
        runtime.imported_route_rules.is_empty(),
        "hiddify-core's setRoutingOptions overwrites options.Route wholesale"
    );
}

// --- the fix ---------------------------------------------------------

/// `compat=hiddify-pinned` must leave Hiddify exactly one tag to build
/// groups from, so `len(tags) > 1` is false and no balancer is created.
#[test]
fn hiddify_pinned_direct_profile_gives_hiddify_exactly_one_route() {
    let runtime = hiddify_runtime(&direct_profile(CompatibilityMode::HiddifyPinned));
    assert_eq!(
        runtime.selectable.len(),
        1,
        "expected a single pinned route, got {:?}",
        runtime.selectable
    );
    assert_eq!(runtime.default_route, runtime.selectable[0]);
    assert!(
        runtime.balanced_over.is_empty(),
        "a pinned profile must not produce a balancer at all"
    );
}

/// The pinned route is the REALITY endpoint under the default
/// `profile=reliability` — the same endpoint the normal profile's own
/// selector defaulted to, so pinning changes how many routes exist, not
/// which one a user ends up on.
#[test]
fn hiddify_pinned_keeps_the_route_the_normal_profile_already_defaulted_to() {
    let normal = direct_profile(CompatibilityMode::Normal);
    let normal_default = normal["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outbound| outbound["tag"] == "select")
        .expect("selector")["default"]
        .as_str()
        .unwrap()
        .to_string();
    let runtime = hiddify_runtime(&direct_profile(CompatibilityMode::HiddifyPinned));
    assert_eq!(runtime.selectable, vec![normal_default]);
}

/// Pinning is a routing change, never a credential change: the surviving
/// outbound must be byte-identical to the one the normal profile serves.
/// If this ever fails, the mode has started shaping credentials, which is
/// `crate::contract`'s job and nobody else's.
#[test]
fn hiddify_pinned_leaves_the_surviving_outbound_byte_identical_to_normal() {
    let pinned = direct_profile(CompatibilityMode::HiddifyPinned);
    let pinned_outbound = pinned["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outbound| outbound["type"] == "vless")
        .expect("pinned vless outbound")
        .clone();
    let normal = direct_profile(CompatibilityMode::Normal);
    let normal_outbound = normal["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outbound| outbound["tag"] == pinned_outbound["tag"])
        .expect("same tag in the normal profile")
        .clone();
    assert_eq!(pinned_outbound, normal_outbound);
}

/// Privacy+: the balancer defect is also a security defect. Its member
/// list includes the direct exit and the relay's own first hop, so a
/// share of every session leaves the enforced RU->DE path — the client's
/// real IP reaches the exit directly, which is precisely what
/// `docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md` forbids.
#[test]
fn privacy_plus_normal_profile_lets_hiddify_balance_onto_a_direct_exit() {
    let runtime = hiddify_runtime(&privacy_profile(CompatibilityMode::Normal));
    assert_eq!(runtime.default_route, "balance");
    assert!(
        runtime
            .balanced_over
            .iter()
            .any(|tag| tag == "Germany · Direct"),
        "the defect being fixed: {:?}",
        runtime.balanced_over
    );
}

/// The pinned Privacy+ profile keeps only the relayed exit, and the RU
/// first hop survives as a `detour` target that Hiddify cannot select or
/// balance onto.
#[test]
fn hiddify_pinned_privacy_plus_keeps_only_the_relayed_exit() {
    let config = privacy_profile(CompatibilityMode::HiddifyPinned);
    let runtime = hiddify_runtime(&config);
    assert_eq!(runtime.selectable, vec!["Germany · via Russia".to_string()]);
    assert!(runtime.balanced_over.is_empty());

    let outbounds = config["outbounds"].as_array().unwrap();
    let exit = outbounds
        .iter()
        .find(|outbound| outbound["tag"] == "Germany · via Russia")
        .expect("relayed exit outbound");
    let detour = exit["detour"]
        .as_str()
        .expect("relayed exit keeps a detour");
    assert!(
        detour.ends_with(HIDDIFY_HIDDEN_TAG_SUFFIX),
        "the first hop must be hidden from Hiddify's group builder, got {detour:?}"
    );
    let first_hop = outbounds
        .iter()
        .find(|outbound| outbound["tag"] == detour)
        .expect("the hidden first hop must still exist as a dialer");
    assert_eq!(
        first_hop["server"], "ru1.example.com",
        "the detour must still point at the relay, not at the exit"
    );
    assert!(
        !outbounds
            .iter()
            .any(|outbound| outbound["tag"] == "Germany · Direct"),
        "a direct exit must not remain in a pinned Privacy+ profile at all"
    );
}

/// Not an assertion — a dump, so `cargo test -- --nocapture
/// dump_pinned_profiles` shows exactly what a reviewer would be asked to
/// import. No credential in these fixtures is real.
#[test]
fn dump_pinned_profiles() {
    eprintln!(
        "--- direct, hiddify-pinned ---\n{}",
        serde_json::to_string_pretty(&direct_profile(CompatibilityMode::HiddifyPinned)).unwrap()
    );
    eprintln!(
        "--- privacy+, hiddify-pinned ---\n{}",
        serde_json::to_string_pretty(&privacy_profile(CompatibilityMode::HiddifyPinned)).unwrap()
    );
}
