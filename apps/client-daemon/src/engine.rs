//! Wires `policy::ScoreBoard`, `failure_classifier::ShutdownGuard`, and the
//! concrete transports together into the thing `docs/DECISION_ENGINE.md`
//! describes: non-deterministic, capability-aware, scored selection with
//! endpoint/transport failure attribution.

use common::{EndpointId, TransportId};
use config::EndpointDescriptor;
use failure_classifier::ShutdownGuard;
use network_state::FailureCategory;
use policy::{Candidate, ScoreBoard};
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use transport_api::{Endpoint, Session, Transport, TransportError};

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("no external path detected: too many recent failures across transports/endpoints")]
    PossibleShutdown,
    #[error("no eligible transport/endpoint candidates available")]
    NoCandidates,
    #[error("all attempts failed, last error: {0}")]
    AllAttemptsFailed(#[from] TransportError),
}

pub struct ResolvedEndpoint {
    pub descriptor: EndpointDescriptor,
    pub endpoint: Endpoint,
}

fn parse_endpoint(d: &EndpointDescriptor) -> Option<Endpoint> {
    let address: SocketAddr = d.address.parse().ok()?;
    let hash_bytes = hex::decode(&d.cert_sha256_hex).ok()?;
    let mut pinned = [0u8; 32];
    if hash_bytes.len() != 32 {
        return None;
    }
    pinned.copy_from_slice(&hash_bytes);
    Some(Endpoint {
        id: EndpointId(d.id.clone()),
        address,
        provider_tag: d.provider_tag.clone(),
        pinned_cert_sha256: pinned,
    })
}

pub struct ConnectionEngine {
    transports: HashMap<TransportId, Arc<dyn Transport>>,
    endpoints: Vec<ResolvedEndpoint>,
    scores: Mutex<ScoreBoard>,
    shutdown_guard: Mutex<ShutdownGuard>,
    tick: AtomicU64,
    max_attempts: usize,
}

impl ConnectionEngine {
    pub fn new(transports: Vec<Arc<dyn Transport>>, endpoints: Vec<EndpointDescriptor>) -> Self {
        Self::with_max_attempts(transports, endpoints, 4)
    }

    /// Same as `new`, but with a configurable attempt budget — mainly for
    /// integration tests that want a very high confidence of exercising
    /// the quarantine-then-succeed path deterministically rather than
    /// relying on the default budget's (small, non-zero) chance of
    /// drawing the failing candidate every single time.
    pub fn with_max_attempts(
        transports: Vec<Arc<dyn Transport>>,
        endpoints: Vec<EndpointDescriptor>,
        max_attempts: usize,
    ) -> Self {
        let transports: HashMap<TransportId, Arc<dyn Transport>> =
            transports.into_iter().map(|t| (t.id(), t)).collect();
        let endpoints = endpoints
            .iter()
            .filter_map(|d| {
                let endpoint = parse_endpoint(d)?;
                Some(ResolvedEndpoint {
                    descriptor: d.clone(),
                    endpoint,
                })
            })
            .collect();
        Self {
            transports,
            endpoints,
            scores: Mutex::new(ScoreBoard::new()),
            // Window of 8, >=75% failure triggers a 90-tick (roughly
            // 90-attempt) cooldown before switching is allowed again.
            shutdown_guard: Mutex::new(ShutdownGuard::new(8, 0.75, 90)),
            tick: AtomicU64::new(0),
            max_attempts,
        }
    }

    fn candidates(&self) -> Vec<(Candidate, &ResolvedEndpoint, Arc<dyn Transport>)> {
        self.endpoints
            .iter()
            .filter_map(|re| {
                let transport = self
                    .transports
                    .get(&TransportId::new(known_transport_id_str(
                        &re.descriptor.transport,
                    )))?;
                Some((
                    Candidate {
                        transport: transport.id(),
                        endpoint: EndpointId(re.descriptor.id.clone()),
                    },
                    re,
                    transport.clone(),
                ))
            })
            .collect()
    }

    /// Attempts to establish a session to `(host, port)` through an
    /// adaptively-selected transport/endpoint, retrying with a different
    /// candidate on transport/endpoint-attributable failure.
    pub async fn connect(
        &self,
        host: &str,
        port: u16,
    ) -> Result<(TransportId, Box<dyn Session>), EngineError> {
        let all = self.candidates();
        if all.is_empty() {
            return Err(EngineError::NoCandidates);
        }

        let mut last_err: Option<TransportError> = None;
        // `rand::thread_rng()` (`ThreadRng`) is `!Send`, which is
        // incompatible with holding it across an `.await` inside a
        // `tokio::spawn`ed task (as `socks::run` does per connection).
        // `StdRng` is `Send` and still a real CSPRNG-seeded generator.
        let mut rng = rand::rngs::StdRng::from_entropy();

        for attempt in 0..self.max_attempts {
            let tick = self.tick.fetch_add(1, Ordering::Relaxed);
            {
                let mut guard = self.shutdown_guard.lock().await;
                if guard.should_suppress_switching(tick) {
                    return Err(EngineError::PossibleShutdown);
                }
            }

            let candidate_list: Vec<Candidate> = all.iter().map(|(c, _, _)| c.clone()).collect();
            let chosen = {
                let board = self.scores.lock().await;
                board.select_next(&candidate_list, &mut rng)
            };
            let Some(chosen) = chosen else {
                return Err(EngineError::NoCandidates);
            };
            let (_, resolved, transport) = all
                .iter()
                .find(|(c, _, _)| *c == chosen)
                .expect("select_next only returns candidates drawn from `all`");

            tracing::debug!(attempt, transport = %chosen.transport, endpoint = %chosen.endpoint, "engine: attempting connect");
            match transport.connect(&resolved.endpoint).await {
                Ok(session) => {
                    self.scores.lock().await.observe_success(&chosen);
                    self.shutdown_guard.lock().await.record(tick, true);
                    let _ = (host, port); // destination header is written by the caller after split()
                    return Ok((chosen.transport, session));
                }
                Err(e) => {
                    let category = e.category();
                    self.shutdown_guard.lock().await.record(tick, false);
                    if category.is_remote_signal() {
                        self.scores
                            .lock()
                            .await
                            .observe_failure(&chosen, category, &mut rng);
                    }
                    tracing::warn!(transport = %chosen.transport, endpoint = %chosen.endpoint, category = ?category, "engine: attempt failed");
                    last_err = Some(e);
                    // Non-deterministic backoff jitter before the next
                    // attempt (spec §8: avoid a fingerprintable fixed
                    // fallback cadence).
                    let jitter_ms = rng.gen_range(50..500);
                    tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;
                }
            }
        }

        Err(last_err
            .map(EngineError::AllAttemptsFailed)
            .unwrap_or(EngineError::NoCandidates))
    }
}

/// Matches a descriptor's transport string against a registered
/// `TransportId`. `TransportId` wraps a `&'static str` by design (cheap,
/// comparable, no per-connection allocation) but bundle-supplied strings
/// are owned `String`s; since only a small, fixed set of transport ids are
/// ever registered in `main.rs`, we match by string equality against the
/// registry instead of trying to intern arbitrary bundle content as
/// `'static`.
fn known_transport_id_str(s: &str) -> &'static str {
    match s {
        "direct-tls" => "direct-tls",
        "noise-quic" => "noise-quic",
        _ => "",
    }
}

pub fn failure_category_of(e: &TransportError) -> FailureCategory {
    e.category()
}
