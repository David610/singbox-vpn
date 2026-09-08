//! Real `sing-box check` validation of the client subscription each
//! `CompatibilityMode` generates — in particular `QuicReject`'s
//! `route.rules` entry.
//!
//! Why this is not covered by the unit tests in `render.rs`: those assert
//! the JSON *contains* `action: "reject"`, `method: "default"` and
//! `no_drop: true`. That cannot catch a field the real sing-box schema
//! rejects, renames, or silently ignores — and this mode's entire value
//! depends on those exact three fields being honored, because
//! `method: "drop"` (or a `no_drop` that does not parse) degrades it back
//! into the silent UDP black hole it exists to replace. See
//! `docs/YOUTUBE_FINAL_ROOT_CAUSE.md` §3.
//!
//! Same skip policy as the other interop suites: a contributor without the
//! pinned binary is not blocked, but CI sets
//! `SINGBOX_VPN_REQUIRE_REAL_INTEROP=1`, which turns every skip into a
//! hard failure. A skipped test is not a pass.

mod common;

use common::SingBox;
use compat_config::model::{CompatEndpoint, CompatTransport, CompatUser, PublicParameters};
use compat_config::render::{
    render_singbox_client_subscription_with_options, CompatibilityMode, SelectionProfile,
};
use compat_config::SecretString;

fn skip_or_fail(reason: &str) {
    if std::env::var("SINGBOX_VPN_REQUIRE_REAL_INTEROP").is_ok() {
        panic!(
            "SINGBOX_VPN_REQUIRE_REAL_INTEROP is set, so this suite must really run, but: \
             {reason}. Refusing to report a skip as a pass."
        );
    }
    eprintln!("skipping: {reason}");
}

fn test_user() -> CompatUser {
    CompatUser {
        id: "u1".into(),
        name: "quic-reject-check".into(),
        enabled: true,
        vless_uuid: "841cac4a-efe4-48ac-92b8-d11f4c98c45e".into(),
        hysteria2_password: SecretString::new("test-password"),
        subscription_token_hash_hex: "unused".into(),
        vision_off_experiment: false,
        created_at: 0,
        expires_at: None,
    }
}

fn endpoints() -> Vec<CompatEndpoint> {
    vec![
        CompatEndpoint {
            id: "reality-1".into(),
            transport: CompatTransport::VlessReality,
            host: "vpn.example.com".into(),
            port: 443,
            server_name: Some("www.microsoft.com".into()),
            label: "Reality".into(),
            public_parameters: PublicParameters::Reality {
                // A real X25519 public key. sing-box validates the
                // curve point at config-load time, so a placeholder here
                // would make `sing-box check` fail for a reason that has
                // nothing to do with what this suite tests. Produced by
                // `sing-box generate reality-keypair`; the matching
                // private key is deliberately not stored — no handshake
                // is performed here, only a config parse.
                public_key_hex: "0Xip_N5E_jam0u8fClW3rsLm7mm2vs9HZ4lhgyr1LDI".into(),
                short_id: "0a1b2c3d".into(),
                fingerprint: "chrome".into(),
            },
        },
        CompatEndpoint {
            id: "hysteria2-1".into(),
            transport: CompatTransport::Hysteria2,
            host: "vpn.example.com".into(),
            port: 8443,
            server_name: Some("vpn.example.com".into()),
            label: "Hysteria2".into(),
            public_parameters: PublicParameters::Hysteria2 {
                obfs_password: Some("obfs-secret".into()),
            },
        },
    ]
}

fn check_mode(sb: &SingBox, dir: &std::path::Path, mode: CompatibilityMode) {
    let doc = render_singbox_client_subscription_with_options(
        &test_user(),
        &endpoints(),
        SelectionProfile::default(),
        mode,
    )
    .expect("render client subscription");
    let path = dir.join(format!("client-{mode:?}.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    let output = sb.check(&path);
    assert!(
        output.status.success(),
        "real sing-box check REJECTED the client profile for {mode:?} (pinned version: see \
         deploy/lib/versions.env).\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Every compatibility mode must render a profile the real, pinned
/// sing-box accepts. A mode that generates a config the client cannot
/// even load is worse than no mode at all: the user gets an opaque import
/// failure rather than the behavior they asked for.
#[test]
fn every_compatibility_mode_renders_a_config_real_sing_box_accepts() {
    let Some(sb) = SingBox::find() else {
        skip_or_fail("sing-box binary not found (set SING_BOX_BIN)");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    for mode in [
        CompatibilityMode::Normal,
        CompatibilityMode::TcpOnly,
        CompatibilityMode::QuicReject,
        CompatibilityMode::VisionOff,
    ] {
        check_mode(&sb, dir.path(), mode);
    }
}

/// The specific regression this file exists for: the three fields that
/// make `QuicReject` a fast, visible rejection rather than a silent drop
/// must survive a real sing-box parse. Asserted here on the exact bytes
/// that were handed to the binary, so this cannot pass against a config
/// that differs from the one checked.
#[test]
fn quic_reject_rule_fields_survive_a_real_sing_box_parse() {
    let Some(sb) = SingBox::find() else {
        skip_or_fail("sing-box binary not found (set SING_BOX_BIN)");
        return;
    };
    let doc = render_singbox_client_subscription_with_options(
        &test_user(),
        &endpoints(),
        SelectionProfile::default(),
        CompatibilityMode::QuicReject,
    )
    .expect("render client subscription");

    // Setup guard: a passing `sing-box check` would prove nothing about
    // these fields unless the config actually being checked contains them.
    let rule = &doc["route"]["rules"][0];
    assert_eq!(
        rule["action"], "reject",
        "test setup bug: rule is not a reject"
    );
    assert_eq!(
        rule["method"], "default",
        "test setup bug: method must be `default` — `drop` is the silent black hole"
    );
    assert_eq!(
        rule["no_drop"], true,
        "test setup bug: no_drop must be true or reject escalates to drop under load"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("quic-reject.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    let output = sb.check(&path);
    assert!(
        output.status.success(),
        "real sing-box check REJECTED the udp/443 reject rule with method=default and \
         no_drop=true — the mode's core mechanism does not parse.\n--- stdout ---\n{}\n\
         --- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
