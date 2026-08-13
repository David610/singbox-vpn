//! Compatibility-domain types and rendering for third-party clients
//! (Hiddify, sing-box-compatible clients, v2rayNG) speaking VLESS+REALITY
//! or Hysteria2, backed by an external sing-box (or future) data plane.
//!
//! Deliberately separate from the native trust chain (`config`,
//! `transport-api`, `rendezvous-client`) — see
//! `docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §6. Nothing here is
//! signed into a `RelayBundle`; nothing native parses these types.

pub mod credentials;
pub mod deployment;
pub mod migrate;
pub mod model;
pub mod ownership;
pub mod render;
pub mod secret;
pub mod server;
pub mod store;

pub use model::{
    CompatEndpoint, CompatTransport, CompatUser, Hysteria2ServerParams, PublicParameters,
    RealityServerParams,
};
pub use secret::SecretString;

#[derive(thiserror::Error, Debug)]
pub enum CompatError {
    #[error("io error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("config validation failed: {0}")]
    ConfigValidationFailed(String),
    #[error("endpoint's public_parameters do not match the requested transport")]
    WrongTransportForEndpoint,
    #[error("user not found")]
    UserNotFound,
    #[error(
        "{what} schema version {found} is newer than this vpn-admin supports (max {max_supported}) — \
         refusing to load it: an older binary cannot safely assume it still understands every \
         field. Upgrade vpn-admin, or restore a compatible backup."
    )]
    UnsupportedSchema {
        what: &'static str,
        found: u32,
        max_supported: u32,
    },
}
