//! Minimal SOCKS5 server (no-auth, CONNECT only) as the client's local
//! listener — spec §5 "Network Capture" layer. Just enough of RFC 1928 to
//! let `curl --socks5-hostname` and similar tools drive the tunnel; no
//! BIND/UDP ASSOCIATE support.

use crate::engine::ConnectionEngine;
use anyhow::{bail, Context, Result};
use common::framing;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub async fn run(bind: &str, engine: Arc<ConnectionEngine>) -> Result<()> {
    let listener = TcpListener::bind(bind).await?;
    tracing::info!(%bind, "client-daemon: SOCKS5 listener up");
    loop {
        let (socket, peer) = listener.accept().await?;
        let engine = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, engine).await {
                tracing::debug!(%peer, error = %e, "socks session ended with error");
            }
        });
    }
}

async fn handle_client(mut socket: TcpStream, engine: Arc<ConnectionEngine>) -> Result<()> {
    // Greeting.
    let ver = socket.read_u8().await?;
    if ver != 0x05 {
        bail!("unsupported SOCKS version {ver}");
    }
    let nmethods = socket.read_u8().await?;
    let mut methods = vec![0u8; nmethods as usize];
    socket.read_exact(&mut methods).await?;
    socket.write_all(&[0x05, 0x00]).await?; // no-auth

    // Request.
    let ver = socket.read_u8().await?;
    let cmd = socket.read_u8().await?;
    let _rsv = socket.read_u8().await?;
    let atyp = socket.read_u8().await?;
    if ver != 0x05 || cmd != 0x01 {
        socket
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?; // command not supported
        bail!("only CONNECT is supported (cmd={cmd})");
    }

    let host = match atyp {
        0x01 => {
            let mut buf = [0u8; 4];
            socket.read_exact(&mut buf).await?;
            std::net::Ipv4Addr::from(buf).to_string()
        }
        0x03 => {
            let len = socket.read_u8().await? as usize;
            let mut buf = vec![0u8; len];
            socket.read_exact(&mut buf).await?;
            String::from_utf8(buf).context("invalid domain name")?
        }
        0x04 => {
            let mut buf = [0u8; 16];
            socket.read_exact(&mut buf).await?;
            std::net::Ipv6Addr::from(buf).to_string()
        }
        other => {
            socket
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?; // addr type not supported
            bail!("unsupported ATYP {other}");
        }
    };
    let port = socket.read_u16().await?;

    tracing::info!(host = %host, port, "client-daemon: SOCKS CONNECT request");

    let (transport_id, session) = match engine.connect(&host, port).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "client-daemon: engine failed to connect");
            socket
                .write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?; // general failure
            return Err(e.into());
        }
    };
    tracing::info!(host = %host, port, transport = %transport_id, "client-daemon: connected via transport");

    socket
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?; // success

    let (tunnel_reader, mut tunnel_writer) = session.split();
    framing::write_destination(&mut tunnel_writer, &host, port).await?;

    let (mut client_reader, mut client_writer) = socket.into_split();
    let mut tunnel_reader = tunnel_reader;
    let client_to_tunnel = tokio::io::copy(&mut client_reader, &mut tunnel_writer);
    let tunnel_to_client = tokio::io::copy(&mut tunnel_reader, &mut client_writer);
    let _ = tokio::try_join!(client_to_tunnel, tunnel_to_client);
    Ok(())
}
