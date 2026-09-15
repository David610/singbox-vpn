//! Platform v2 domain core.
//!
//! Everything in this crate is deterministic and free of I/O: time is
//! passed in as milliseconds, randomness is never used, and no module
//! touches the network, the filesystem or a process. That is what lets
//! the route engine, the failover controller and the replacement
//! orchestrator be tested exhaustively here and replayed as golden
//! vectors by other implementations (Tamara, the probe agent).
//!
//! Architecture: `docs/platform-v2/TARGET_ARCHITECTURE.md`.

pub mod bench;
pub mod evidence;
pub mod failover;
pub mod failure;
pub mod health;
pub mod lifecycle;
pub mod probe;
pub mod provider;
pub mod replacement;
pub mod route;
pub mod scoring;
pub mod telemetry;
pub mod transport;

/// Milliseconds since the Unix epoch, supplied by the caller.
pub type TimestampMs = u64;
