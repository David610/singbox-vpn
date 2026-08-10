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
    /// Bound on the TLS handshake itself, applied BEFORE a connection is
    /// allowed to occupy one of the `max_connections` slots for any
    /// meaningful length of time. Without this, a peer that completes the
    /// TCP handshake and then sends nothing parks its permit forever, so
    /// `max_connections` silent sockets take the relay down permanently.
    /// Deliberately much shorter than `idle_timeout`: a real TLS handshake
    /// is a couple of round trips, not two minutes.
    pub handshake_timeout: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_connections: 256,
            idle_timeout: Duration::from_secs(120),
            dial_timeout: Duration::from_secs(8),
            handshake_timeout: Duration::from_secs(10),
        }
    }
}

/// Wraps a stream so every byte actually read or written bumps a shared
/// counter. This is what lets `copy_with_idle_timeout` tell "slow but
/// alive" apart from "connected and silent" without capping total session
/// duration — a long-lived SSH or websocket tunnel must not be killed just
/// for being long-lived.
struct ActivityTracked<S> {
    inner: S,
    activity: Arc<std::sync::atomic::AtomicU64>,
}

impl<S> ActivityTracked<S> {
    fn bump(&self) {
        self.activity
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ActivityTracked<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &poll {
            if buf.filled().len() > before {
                self.bump();
            }
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ActivityTracked<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let poll = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = &poll {
            if *n > 0 {
                self.bump();
            }
        }
        poll
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Accept-loop errors that a long-running listener must survive rather than
/// die on. `EMFILE`/`ENFILE` are especially reachable on a relay holding
/// hundreds of connections (two fds each); `ECONNABORTED` just means the
/// peer went away between the SYN and our `accept`.
fn is_transient_accept_error(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        ConnectionAborted | ConnectionReset | Interrupted | WouldBlock
    ) || matches!(e.raw_os_error(), Some(libc_emfile) if libc_emfile == 24 || libc_emfile == 23)
}

/// `copy_bidirectional` with an INACTIVITY bound. `RelayLimits::idle_timeout`
/// was previously applied only to the destination-header read, leaving the
/// data phase completely unbounded: a peer that completed TLS, sent a valid
/// header and then went silent held its permit, its two file descriptors and
/// its task forever — the same permanent denial as an unbounded handshake,
/// reached through an entirely "legitimate" path.
async fn copy_with_idle_timeout<A, B>(a: &mut A, b: &mut B, idle: Duration) -> std::io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let activity = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut ta = ActivityTracked {
        inner: a,
        activity: activity.clone(),
    };
    let mut tb = ActivityTracked {
        inner: b,
        activity: activity.clone(),
    };
    let copy = copy_bidirectional(&mut ta, &mut tb);
    tokio::pin!(copy);
    loop {
        let before = activity.load(std::sync::atomic::Ordering::Relaxed);
        match tokio::time::timeout(idle, &mut copy).await {
            Ok(result) => return result.map(|_| ()),
            Err(_) => {
                // The copy is still running. Only abort if NOTHING moved in
                // either direction for a whole idle window.
                if activity.load(std::sync::atomic::Ordering::Relaxed) == before {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "relay session idle timeout",
                    ));
                }
            }
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
            copy_with_idle_timeout(&mut client, &mut upstream, limits.idle_timeout).await?;
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
            copy_with_idle_timeout(&mut client, &mut next, limits.idle_timeout).await?;
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
        let (tcp, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) if is_transient_accept_error(&e) => {
                // Resource exhaustion (EMFILE/ENFILE) and aborted handshakes
                // (ECONNABORTED) are transient conditions a long-running relay
                // must survive, not reasons to terminate the listener — and
                // `?` here propagates all the way out of `main`, killing the
                // process. Back off briefly so an fd-exhaustion storm doesn't
                // become a busy loop, then keep serving.
                tracing::warn!(error = %e, "relay-agent: transient accept error, continuing");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
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
            // Bound the handshake. The permit is already held at this point,
            // so an unbounded `accept` lets a peer that completes the TCP
            // handshake and then says nothing hold a connection slot forever
            // — `max_connections` such sockets from a single host take the
            // relay down permanently, and every subsequent client is dropped
            // with no response.
            let tls = match tokio::time::timeout(limits.handshake_timeout, acceptor.accept(tcp))
                .await
            {
                Ok(Ok(t)) => t,
                Ok(Err(e)) => {
                    tracing::debug!(%peer, error = %e, "tls accept failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(%peer, "tls handshake timed out; releasing connection slot");
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
