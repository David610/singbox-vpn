//! Ingress and/or egress relay. `Role::Combined`/`Role::Egress` dial the
//! requested destination directly; `Role::Ingress` forwards the framed
//! stream to a second-hop egress over its own `direct-tls` connection —
//! see `docs/ARCHITECTURE.md`/ADR-0006 for why hop count is configurable
//! rather than hardcoded.
//!
//! Bidirectional relaying uses raw streams + `tokio::io::copy_bidirectional`
//! rather than the `transport_api::Session` trait's serialized send/recv:
//! `Session` requires `&mut self` for both directions, so driving it from
//! two concurrent tasks needs a lock that would have to be held across an
//! in-flight `.await` on one direction while the other direction is also
//! trying to make progress — a real deadlock risk for a relay that must
//! move bytes both ways at once. Raw streams give real split halves.

use anyhow::{Context as _, Result};
use common::framing;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{copy_bidirectional, AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use transport_api::Endpoint;
use transport_native::server::{tls_server_config, QuicBiStream, RelayIdentity};

#[derive(Clone)]
pub enum Role {
    Combined,
    Egress,
    Ingress { next_hop: Endpoint },
}

pub struct RelayLimits {
    pub max_connections: usize,
    pub idle_timeout: Duration,
    pub dial_timeout: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_connections: 256,
            idle_timeout: Duration::from_secs(120),
            dial_timeout: Duration::from_secs(8),
        }
    }
}

async fn handle_stream<S>(mut client: S, role: Role, limits: Arc<RelayLimits>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (host, port) =
        tokio::time::timeout(limits.idle_timeout, framing::read_destination(&mut client))
            .await
            .context("timed out reading destination header")??;
    tracing::debug!(host = %host, port, "relay: forwarding");

    match role {
        Role::Combined | Role::Egress => {
            let mut upstream = tokio::time::timeout(
                limits.dial_timeout,
                TcpStream::connect((host.as_str(), port)),
            )
            .await
            .context("timed out dialing destination")??;
            copy_bidirectional(&mut client, &mut upstream).await?;
        }
        Role::Ingress { next_hop } => {
            let mut next = tokio::time::timeout(
                limits.dial_timeout,
                transport_native::direct_tls::connect_stream(&next_hop),
            )
            .await
            .context("timed out dialing next hop")?
            .map_err(|e| anyhow::anyhow!("next hop connect failed: {e}"))?;
            framing::write_destination(&mut next, &host, port).await?;
            copy_bidirectional(&mut client, &mut next).await?;
        }
    }
    Ok(())
}

pub async fn serve_tls(
    bind: SocketAddr,
    identity: &RelayIdentity,
    role: Role,
    limits: RelayLimits,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    serve_tls_on(listener, identity, role, limits).await
}

/// Like `serve_tls`, but takes an already-bound listener so the caller can
/// learn the real (possibly OS-assigned) port before the accept loop
/// starts running — used by integration tests that bind to `:0`.
pub async fn serve_tls_on(
    listener: tokio::net::TcpListener,
    identity: &RelayIdentity,
    role: Role,
    limits: RelayLimits,
) -> Result<()> {
    let tls_config = tls_server_config(identity);
    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
    let semaphore = Arc::new(Semaphore::new(limits.max_connections));
    let limits = Arc::new(limits);
    tracing::info!(bind = ?listener.local_addr(), "relay-agent: direct-tls listener up");

    loop {
        let (tcp, peer) = listener.accept().await?;
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(%peer, "relay-agent: connection cap reached, dropping");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let role = role.clone();
        let limits = limits.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let tls = match acceptor.accept(tcp).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::debug!(%peer, error = %e, "tls accept failed");
                    return;
                }
            };
            if let Err(e) = handle_stream(tls, role, limits).await {
                tracing::debug!(%peer, error = %e, "relay session ended with error");
            }
        });
    }
}

pub fn quic_listen(bind: SocketAddr, identity: &RelayIdentity) -> Result<quinn::Endpoint> {
    let server_config = transport_native::server::quic_server_config(identity);
    Ok(quinn::Endpoint::server(server_config, bind)?)
}

pub async fn serve_quic(
    bind: SocketAddr,
    identity: &RelayIdentity,
    role: Role,
    limits: RelayLimits,
) -> Result<()> {
    let endpoint = quic_listen(bind, identity)?;
    serve_quic_on(endpoint, role, limits).await
}

/// Like `serve_quic`, but takes an already-bound `quinn::Endpoint` so the
/// caller can read `local_addr()` before the accept loop starts.
pub async fn serve_quic_on(
    endpoint: quinn::Endpoint,
    role: Role,
    limits: RelayLimits,
) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(limits.max_connections));
    let limits = Arc::new(limits);
    tracing::info!(bind = ?endpoint.local_addr(), "relay-agent: noise-quic listener up");

    while let Some(incoming) = endpoint.accept().await {
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let role = role.clone();
        let limits = limits.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let connection = match incoming.await {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(error = %e, "quic handshake failed");
                    return;
                }
            };
            while let Ok((send, recv)) = connection.accept_bi().await {
                let role = role.clone();
                let limits = limits.clone();
                tokio::spawn(async move {
                    let bi = QuicBiStream { send, recv };
                    if let Err(e) = handle_stream(bi, role, limits).await {
                        tracing::debug!(error = %e, "quic relay session ended with error");
                    }
                });
            }
        });
    }
    Ok(())
}
