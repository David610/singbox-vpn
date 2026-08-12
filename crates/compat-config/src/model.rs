//! Compatibility-domain types. Deliberately separate from
//! `config::EndpointDescriptor` (the native, signed-bundle relay
//! descriptor) — see spec §5 and
//! `docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §6. A `CompatEndpoint`
//! describes a third-party-client-facing listener (VLESS+REALITY or
//! Hysteria2) on a sing-box (or future backend) data plane; it is never
//! signed into a native `RelayBundle` and native code never parses it.

use crate::secret::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatTransport {
    VlessReality,
    Hysteria2,
}

impl CompatTransport {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompatTransport::VlessReality => "vless-reality",
            CompatTransport::Hysteria2 => "hysteria2",
        }
    }
}

/// Public (client-safe) parameters for a compatibility endpoint. Never
/// includes server-private material (REALITY private key, TLS private
/// key) — those live in `RealityServerParams`/`Hysteria2ServerParams`
/// (server-side only, see `store.rs`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicParameters {
    Reality {
        public_key_hex: String,
        short_id: String,
        fingerprint: String,
    },
    Hysteria2 {
        /// Salamander obfuscation password, if enabled. Shared with
        /// clients by design (it is not a per-user secret, it's a
        /// protocol-obfuscation shared value) but still not logged.
        obfs_password: Option<String>,
    },
}

/// A single client-facing compatibility listener.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompatEndpoint {
    pub id: String,
    pub transport: CompatTransport,
    pub host: String,
    pub port: u16,
    pub server_name: Option<String>,
    pub label: String,
    pub public_parameters: PublicParameters,
}

/// A compatibility (third-party-client) user. Persisted in
/// `/etc/vpn/compat/users/users.json`; never mixed into the native
/// `config`/`rendezvous` trust chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompatUser {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub vless_uuid: String,
    pub hysteria2_password: SecretString,
    /// SHA-256 hex digest of the subscription token. The raw token is
    /// never persisted (spec §14) — only shown once at creation/rotation.
    pub subscription_token_hash_hex: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

impl CompatUser {
    pub fn is_active(&self, now_unix: i64) -> bool {
        self.enabled && self.expires_at.map(|exp| now_unix < exp).unwrap_or(true)
    }
}

/// Server-side REALITY parameters. `private_key_hex` must never be
/// logged, serialized into a subscription response, or rendered into a
/// client-facing config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealityServerParams {
    pub private_key_hex: SecretString,
    pub public_key_hex: String,
    pub short_ids: Vec<String>,
    /// Disguise/decoy target dialed for the real REALITY handshake
    /// (sing-box `tls.reality.handshake.server`/`server_port`).
    pub handshake_server: String,
    pub handshake_port: u16,
}

/// Server-side Hysteria2 TLS/obfuscation parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hysteria2ServerParams {
    pub tls_cert_path: String,
    pub tls_key_path: String,
    pub obfs_password: Option<SecretString>,
    /// Directory served back to unauthenticated/invalid Hysteria2
    /// connections (sing-box `masquerade` of type `file`), so failed
    /// probes see plausible HTTP content instead of a distinctive
    /// auth-reject signature. `None` disables masquerade (e.g. in tests
    /// that don't care about this behavior).
    pub masquerade_dir_path: Option<String>,
    /// Explicit fixed-rate (Brutal) bandwidth, Mbps, set only when the
    /// operator has measured the VPS's real sustained throughput (`vpn
    /// benchmark`) — see docs/PERFORMANCE_OPTIMIZATION_PLAN.md. `None`
    /// (the default) leaves sing-box's adaptive BBR-based congestion
    /// control in place, which is the only safe default when the real
    /// bandwidth is unknown: an inflated fixed value causes sustained
    /// self-induced congestion instead of the loss-adaptive backoff BBR
    /// provides (see `render_hysteria2_inbound`'s doc comment for the
    /// mechanism). Both fields are set together or not at all — see
    /// `Hysteria2Section::bandwidth` in `deployment.rs`.
    pub up_mbps: Option<u32>,
    pub down_mbps: Option<u32>,
}
