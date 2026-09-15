//! Privacy-preserving operational telemetry.
//!
//! Design rules (see `docs/platform-v2/PRIVACY_MODEL.md`):
//!
//! * **Off by default.** Nothing leaves a device without explicit opt-in
//!   ([`TelemetryConsent::UploadAggregate`]).
//! * **Aggregate, coarse, route-level.** Reports describe how a *route*
//!   performed during an hour, in buckets. They never describe what a
//!   user did: no destinations, DNS queries, SNI, URLs, payloads, client
//!   IP addresses or local network identifiers.
//! * **Allowlisted schema.** [`validate_report_json`] rejects any key not
//!   in [`ROUTE_HEALTH_FIELDS`] and any string value shaped like a
//!   credential, key, token, address or URL — so a future field cannot
//!   leak something by accident.
//! * **Redacted free text.** Error text passes through [`redact_text`]
//!   before it may appear in a diagnostic bundle.

use crate::failure::FailureClass;
use crate::health::{HealthLayer, Observation};
use crate::route::{RouteCatalog, RouteKind};
use crate::transport::TransportKind;
use crate::TimestampMs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Schema identifier; bump on any field change.
pub const ROUTE_HEALTH_SCHEMA: &str = "route-health/1";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryConsent {
    /// Nothing is recorded beyond what the route engine needs in memory.
    #[default]
    Off,
    /// Reports are built and kept on the device for the user's own
    /// diagnostics only.
    LocalOnly,
    /// Aggregate reports may be uploaded.
    UploadAggregate,
}

impl TelemetryConsent {
    pub fn may_upload(self) -> bool {
        self == TelemetryConsent::UploadAggregate
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Low,
    Medium,
    High,
}

/// Documentation record for one exported field. The privacy model
/// document is checked against this table by a test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: &'static str,
    pub purpose: &'static str,
    pub retention_days: u32,
    pub sensitivity: Sensitivity,
    pub required: bool,
    pub locally_computed: bool,
}

pub const ROUTE_HEALTH_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        name: "schema",
        purpose: "schema version for parsing",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "policy_version",
        purpose: "route-engine rules the client used",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "client_version",
        purpose: "correlate regressions with releases",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "platform",
        purpose: "platform-specific failures (TUN, sleep, handover)",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "network_type",
        purpose: "separate Wi-Fi/cellular/wired behaviour",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "access_asn",
        purpose: "which access networks break which routes (opt-in, coarse)",
        retention_days: 30,
        sensitivity: Sensitivity::Medium,
        required: false,
        locally_computed: false,
    },
    FieldSpec {
        name: "country",
        purpose: "regional reachability (user- or operator-declared)",
        retention_days: 30,
        sensitivity: Sensitivity::Medium,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "route_id",
        purpose: "which operator route the aggregate describes",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "transport",
        purpose: "transport-level reliability",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "route_kind",
        purpose: "direct versus relayed reliability",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "hour_bucket",
        purpose: "time-of-day and incident windows (hour resolution)",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "attempts",
        purpose: "denominator for success rate",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "successes",
        purpose: "success rate (L4 usable)",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "failures",
        purpose: "failure classes (typed, no free text)",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: true,
        locally_computed: true,
    },
    FieldSpec {
        name: "highest_layer",
        purpose: "how far attempts got (L1-L6)",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "handshake_ms",
        purpose: "handshake latency bucket",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "rtt_ms",
        purpose: "RTT bucket",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "jitter_ms",
        purpose: "jitter bucket",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "loss_permille",
        purpose: "packet-loss bucket",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "throughput_kbps",
        purpose: "sustained-transfer bucket",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
    FieldSpec {
        name: "recovery_ms",
        purpose: "automatic recovery time bucket",
        retention_days: 90,
        sensitivity: Sensitivity::Low,
        required: false,
        locally_computed: true,
    },
];

/// Fields that must never be exported, whatever a future change adds.
/// Checked by [`validate_report_json`] at any depth.
pub const FORBIDDEN_KEYS: &[&str] = &[
    "network_id",
    "client_ip",
    "ip",
    "address",
    "host",
    "destination",
    "domain",
    "sni",
    "server_name",
    "url",
    "dns",
    "query",
    "uuid",
    "password",
    "private_key",
    "preshared_key",
    "token",
    "secret",
    "credential",
    "email",
    "account",
    "device_id",
    "mac",
    "ssid",
    "bssid",
    "imei",
    "payload",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Android,
    Magicos,
    Ios,
    Macos,
    Linux,
    ProbeAgent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkType {
    Wifi,
    Cellular,
    Ethernet,
    Datacenter,
    Unknown,
}

/// Coarse value bucket: lower bound inclusive, upper exclusive; `None`
/// upper means open-ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    pub from: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u32>,
}

pub fn bucket(value: u32, bounds: &[u32]) -> Bucket {
    let mut from = 0;
    for &b in bounds {
        if value < b {
            return Bucket { from, to: Some(b) };
        }
        from = b;
    }
    Bucket { from, to: None }
}

const MS_BOUNDS: &[u32] = &[50, 100, 200, 400, 800, 1600, 3200, 6400];
const JITTER_BOUNDS: &[u32] = &[5, 10, 20, 40, 80, 160];
const LOSS_BOUNDS: &[u32] = &[1, 5, 20, 50, 100, 250];
const KBPS_BOUNDS: &[u32] = &[256, 1_000, 5_000, 20_000, 50_000, 100_000];
const RECOVERY_BOUNDS: &[u32] = &[1_000, 3_000, 10_000, 30_000, 60_000, 180_000];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteHealthReport {
    pub schema: String,
    pub policy_version: String,
    pub client_version: String,
    pub platform: Platform,
    pub network_type: NetworkType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_asn: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    pub route_id: String,
    pub transport: TransportKind,
    pub route_kind: RouteKind,
    pub hour_bucket: u64,
    pub attempts: u32,
    pub successes: u32,
    pub failures: BTreeMap<FailureClass, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highest_layer: Option<HealthLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_ms: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_permille: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput_kbps: Option<Bucket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_ms: Option<Bucket>,
}

/// Context supplied by the host; everything optional here is only
/// included when the user opted in to it separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportContext {
    pub client_version: String,
    pub policy_version: String,
    pub platform: Platform,
    pub network_type: NetworkType,
    pub access_asn: Option<u32>,
    pub country: Option<String>,
}

pub const HOUR_MS: u64 = 60 * 60 * 1000;

/// Aggregate `observations` (any mix of routes and hours) into one report
/// per (route, hour). The local `network_id` is consumed here and never
/// copied into a report.
pub fn build_reports(
    catalog: &RouteCatalog,
    observations: &[Observation],
    recoveries: &[(String, TimestampMs, u64)],
    ctx: &ReportContext,
) -> Vec<RouteHealthReport> {
    #[derive(Default)]
    struct Acc {
        attempts: u32,
        successes: u32,
        failures: BTreeMap<FailureClass, u32>,
        highest: Option<HealthLayer>,
        handshake: Vec<u32>,
        rtt: Vec<u32>,
        jitter: Vec<u32>,
        loss: Vec<u32>,
        kbps: Vec<u32>,
        recovery: Vec<u32>,
    }
    let mut groups: BTreeMap<(String, u64), Acc> = BTreeMap::new();
    for o in observations {
        let acc = groups
            .entry((o.route_id.clone(), o.at_ms / HOUR_MS * HOUR_MS))
            .or_default();
        acc.attempts += 1;
        if o.is_usable() {
            acc.successes += 1;
        } else if o.is_failure() {
            *acc.failures
                .entry(o.failure.unwrap_or(FailureClass::Unknown))
                .or_insert(0) += 1;
        }
        acc.highest = acc.highest.max(o.highest_passed_layer());
        acc.handshake.extend(o.handshake_ms);
        acc.rtt.extend(o.rtt_ms);
        acc.jitter.extend(o.jitter_ms);
        acc.loss.extend(o.loss_permille.map(u32::from));
        acc.kbps.extend(o.transfer_kbps);
    }
    for (route_id, at_ms, recovery_ms) in recoveries {
        let acc = groups
            .entry((route_id.clone(), at_ms / HOUR_MS * HOUR_MS))
            .or_default();
        acc.recovery
            .push((*recovery_ms).min(u32::MAX as u64) as u32);
    }

    groups
        .into_iter()
        .filter_map(|((route_id, hour), acc)| {
            let route = catalog.route(&route_id)?;
            let transport = catalog.entry_transport(route)?.clone();
            let median_bucket = |v: &Vec<u32>, bounds: &[u32]| median(v).map(|m| bucket(m, bounds));
            Some(RouteHealthReport {
                schema: ROUTE_HEALTH_SCHEMA.into(),
                policy_version: ctx.policy_version.clone(),
                client_version: ctx.client_version.clone(),
                platform: ctx.platform,
                network_type: ctx.network_type,
                access_asn: ctx.access_asn,
                country: ctx.country.clone(),
                route_id,
                transport,
                route_kind: route.kind,
                hour_bucket: hour,
                attempts: acc.attempts,
                successes: acc.successes,
                failures: acc.failures,
                highest_layer: acc.highest,
                handshake_ms: median_bucket(&acc.handshake, MS_BOUNDS),
                rtt_ms: median_bucket(&acc.rtt, MS_BOUNDS),
                jitter_ms: median_bucket(&acc.jitter, JITTER_BOUNDS),
                loss_permille: median_bucket(&acc.loss, LOSS_BOUNDS),
                throughput_kbps: median_bucket(&acc.kbps, KBPS_BOUNDS),
                recovery_ms: median_bucket(&acc.recovery, RECOVERY_BOUNDS),
            })
        })
        .collect()
}

fn median(v: &[u32]) -> Option<u32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    Some(s[(s.len() - 1) / 2])
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TelemetryViolation {
    #[error("unknown field {0:?}")]
    UnknownField(String),
    #[error("forbidden field {0:?}")]
    ForbiddenField(String),
    #[error("missing required field {0:?}")]
    MissingField(&'static str),
    #[error("field {field:?} carries a value shaped like {shape}")]
    SensitiveValue { field: String, shape: &'static str },
    #[error("report is not a JSON object")]
    NotAnObject,
}

/// Validate a serialized report before upload.
pub fn validate_report_json(value: &serde_json::Value) -> Result<(), TelemetryViolation> {
    let obj = value.as_object().ok_or(TelemetryViolation::NotAnObject)?;
    for key in obj.keys() {
        if FORBIDDEN_KEYS.contains(&key.as_str()) {
            return Err(TelemetryViolation::ForbiddenField(key.clone()));
        }
        if !ROUTE_HEALTH_FIELDS.iter().any(|f| f.name == key) {
            return Err(TelemetryViolation::UnknownField(key.clone()));
        }
    }
    for f in ROUTE_HEALTH_FIELDS.iter().filter(|f| f.required) {
        if !obj.contains_key(f.name) {
            return Err(TelemetryViolation::MissingField(f.name));
        }
    }
    for (key, v) in obj {
        check_value(key, v)?;
    }
    Ok(())
}

fn check_value(field: &str, v: &serde_json::Value) -> Result<(), TelemetryViolation> {
    match v {
        serde_json::Value::String(s) => {
            if let Some(shape) = sensitive_shape(s) {
                return Err(TelemetryViolation::SensitiveValue {
                    field: field.into(),
                    shape,
                });
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items.iter().try_for_each(|i| check_value(field, i)),
        serde_json::Value::Object(map) => {
            for (k, inner) in map {
                if FORBIDDEN_KEYS.contains(&k.as_str()) {
                    return Err(TelemetryViolation::ForbiddenField(format!("{field}.{k}")));
                }
                if let Some(shape) = sensitive_shape(k) {
                    return Err(TelemetryViolation::SensitiveValue {
                        field: format!("{field}.{k}"),
                        shape,
                    });
                }
                check_value(field, inner)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Classify a string that must not appear in telemetry. Returns the
/// shape name, or `None` if the string looks harmless.
pub fn sensitive_shape(s: &str) -> Option<&'static str> {
    let t = s.trim();
    if t.contains("://") {
        return Some("url");
    }
    if t.contains('@') && t.rsplit('@').next().is_some_and(|d| d.contains('.')) {
        return Some("email");
    }
    if strip_port(t).parse::<std::net::IpAddr>().is_ok() {
        return Some("ip-address");
    }
    if is_uuid(t) {
        return Some("uuid");
    }
    if t.len() >= 32 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some("hex-secret");
    }
    if looks_like_base64_secret(t) {
        return Some("base64-secret");
    }
    if t.starts_with("-----BEGIN") {
        return Some("pem");
    }
    None
}

fn strip_port(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    // a.b.c.d:port
    if s.matches(':').count() == 1 {
        if let Some((host, port)) = s.split_once(':') {
            if port.chars().all(|c| c.is_ascii_digit()) {
                return host;
            }
        }
    }
    s
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn looks_like_base64_secret(s: &str) -> bool {
    s.len() >= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| c.is_ascii_uppercase())
        && s.chars().any(|c| c.is_ascii_lowercase())
}

/// Redact sensitive tokens from free text (engine errors, log tails).
pub fn redact_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if !token.is_empty() {
            let trimmed = token.trim_end_matches(['.', ',', ';', ')', '"', '\'']);
            let tail = &token[trimmed.len()..];
            match sensitive_shape(trimmed) {
                Some(shape) => {
                    out.push_str("<redacted:");
                    out.push_str(shape);
                    out.push('>');
                    out.push_str(tail);
                }
                None => out.push_str(token),
            }
            token.clear();
        }
    };
    for c in input.chars() {
        if c.is_whitespace()
            || matches!(c, '(' | '"' | '\'' | '<' | '>' | ',' | '=') && !is_b64_padding(&token, c)
        {
            flush(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// `=` terminates a `key=value` token unless it is base64 padding at the
/// end of an already long token.
fn is_b64_padding(token: &str, c: char) -> bool {
    c == '=' && token.len() >= 20
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::test_support::{failure, success};
    use crate::route::test_support::two_node_catalog;

    fn ctx() -> ReportContext {
        ReportContext {
            client_version: "0.3.0".into(),
            policy_version: "route-engine/1".into(),
            platform: Platform::Android,
            network_type: NetworkType::Cellular,
            access_asn: None,
            country: None,
        }
    }

    #[test]
    fn consent_defaults_to_off() {
        assert_eq!(TelemetryConsent::default(), TelemetryConsent::Off);
        assert!(!TelemetryConsent::LocalOnly.may_upload());
    }

    #[test]
    fn reports_aggregate_per_route_and_hour_without_network_id() {
        let c = two_node_catalog();
        let obs = vec![
            success("fi1/awg", 1_000, 40),
            success("fi1/awg", 2_000, 60),
            failure(
                "fi1/awg",
                3_000,
                HealthLayer::Handshake,
                FailureClass::UdpUnreachable,
            ),
            success("fi1/awg", HOUR_MS + 1, 40),
        ];
        let reports = build_reports(&c, &obs, &[("fi1/awg".into(), 2_500, 4_200)], &ctx());
        assert_eq!(reports.len(), 2);
        let r = &reports[0];
        assert_eq!((r.attempts, r.successes), (3, 2));
        assert_eq!(r.failures.get(&FailureClass::UdpUnreachable), Some(&1));
        assert_eq!(
            r.rtt_ms,
            Some(Bucket {
                from: 0,
                to: Some(50)
            })
        );
        assert_eq!(
            r.recovery_ms,
            Some(Bucket {
                from: 3_000,
                to: Some(10_000)
            })
        );
        for report in &reports {
            let json = serde_json::to_value(report).unwrap();
            validate_report_json(&json).unwrap();
            assert!(
                !json.to_string().contains("net-a"),
                "local network id leaked"
            );
        }
    }

    #[test]
    fn unknown_and_forbidden_fields_are_rejected() {
        let c = two_node_catalog();
        let r = &build_reports(&c, &[success("nl1/reality", 0, 30)], &[], &ctx())[0];
        let mut json = serde_json::to_value(r).unwrap();
        json["destination"] = "example.org".into();
        assert!(matches!(
            validate_report_json(&json),
            Err(TelemetryViolation::ForbiddenField(_))
        ));

        let mut json = serde_json::to_value(r).unwrap();
        json["new_field"] = 1.into();
        assert!(matches!(
            validate_report_json(&json),
            Err(TelemetryViolation::UnknownField(_))
        ));

        let mut json = serde_json::to_value(r).unwrap();
        json.as_object_mut().unwrap().remove("route_id");
        assert!(matches!(
            validate_report_json(&json),
            Err(TelemetryViolation::MissingField("route_id"))
        ));
    }

    #[test]
    fn credential_shaped_values_never_pass_validation() {
        let c = two_node_catalog();
        let r = &build_reports(&c, &[success("nl1/reality", 0, 30)], &[], &ctx())[0];
        let secrets = [
            "00000000-0000-4000-8000-000000000001",
            "yH6jKnw0cqg8x7dS3wA3m1F5y3wXk5a1v9QW2m8Zp1E=",
            "2a0b6f5e3c1d4e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d",
            "203.0.113.10:443",
            "[2001:db8::1]:51820",
            "https://vpn.example.com/v1/provision/tok",
            "user@example.com",
            "-----BEGIN PRIVATE KEY-----",
        ];
        for secret in secrets {
            let mut json = serde_json::to_value(r).unwrap();
            json["client_version"] = secret.into();
            assert!(
                matches!(
                    validate_report_json(&json),
                    Err(TelemetryViolation::SensitiveValue { .. })
                ),
                "{secret} passed"
            );
        }
    }

    #[test]
    fn ordinary_values_are_not_flagged() {
        for ok in [
            "0.3.0",
            "fi1/awg",
            "route-engine/1",
            "RU",
            "vless-reality",
            "nl1/reality-via-ru1",
        ] {
            assert_eq!(sensitive_shape(ok), None, "{ok}");
        }
    }

    #[test]
    fn redaction_removes_credentials_addresses_and_urls() {
        let line = "dial 203.0.113.10:443 failed for uuid=00000000-0000-4000-8000-000000000001 \
                    key=yH6jKnw0cqg8x7dS3wA3m1F5y3wXk5a1v9QW2m8Zp1E= via https://vpn.example.com/sub/abc, \
                    peer [2001:db8::1]:51820 (timeout).";
        let red = redact_text(line);
        for leaked in [
            "203.0.113.10",
            "00000000-0000-4000",
            "yH6jKnw0",
            "vpn.example.com",
            "2001:db8",
        ] {
            assert!(!red.contains(leaked), "{leaked} leaked in {red}");
        }
        assert!(red.contains("failed for uuid=<redacted:uuid>"), "{red}");
        assert!(red.contains("(timeout)."), "{red}");
    }

    #[test]
    fn privacy_model_documents_every_field() {
        let doc = include_str!("../../../docs/platform-v2/PRIVACY_MODEL.md");
        for f in ROUTE_HEALTH_FIELDS {
            assert!(
                doc.contains(&format!("`{}`", f.name)),
                "PRIVACY_MODEL.md lacks `{}`",
                f.name
            );
        }
        for k in FORBIDDEN_KEYS {
            assert!(
                doc.contains(&format!("`{k}`")),
                "PRIVACY_MODEL.md lacks forbidden `{k}`"
            );
        }
    }

    #[test]
    fn buckets() {
        assert_eq!(
            bucket(0, MS_BOUNDS),
            Bucket {
                from: 0,
                to: Some(50)
            }
        );
        assert_eq!(
            bucket(50, MS_BOUNDS),
            Bucket {
                from: 50,
                to: Some(100)
            }
        );
        assert_eq!(
            bucket(99_999, MS_BOUNDS),
            Bucket {
                from: 6400,
                to: None
            }
        );
    }
}
