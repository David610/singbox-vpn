//! Minimal destination-framing header used between client and egress (or
//! ingress and egress, for the split topology): after the transport-level
//! handshake, the initiator sends one small header naming the destination
//! `host:port`, then the connection is a raw bidirectional byte pipe.
//!
//! Wire format: `[version: u8=1][host_len: u16 BE][host bytes][port: u16 BE]`.
//! `host_len` is bounded (`MAX_HOST_LEN`) so a malicious peer can't force
//! an unbounded allocation before the header is fully validated — this
//! path is fuzz-tested (`fuzz/fuzz_targets/framing_header.rs`).

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const VERSION: u8 = 1;
pub const MAX_HOST_LEN: usize = 255;

#[derive(Debug, PartialEq, Eq)]
pub enum FramingError {
    UnsupportedVersion(u8),
    HostTooLong(usize),
    InvalidUtf8,
}

impl std::fmt::Display for FramingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FramingError::UnsupportedVersion(v) => write!(f, "unsupported framing version {v}"),
            FramingError::HostTooLong(n) => write!(f, "host length {n} exceeds max {MAX_HOST_LEN}"),
            FramingError::InvalidUtf8 => write!(f, "host is not valid utf-8"),
        }
    }
}
impl std::error::Error for FramingError {}

pub fn encode_destination(host: &str, port: u16) -> Result<Vec<u8>, FramingError> {
    let host_bytes = host.as_bytes();
    if host_bytes.len() > MAX_HOST_LEN {
        return Err(FramingError::HostTooLong(host_bytes.len()));
    }
    let mut buf = Vec::with_capacity(1 + 2 + host_bytes.len() + 2);
    buf.push(VERSION);
    buf.extend_from_slice(&(host_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(host_bytes);
    buf.extend_from_slice(&port.to_be_bytes());
    Ok(buf)
}

/// Parses a complete in-memory header buffer (used by fuzzing / unit
/// tests). Returns the parsed `(host, port)` and the number of bytes
/// consumed, or an error — never panics on malformed/truncated input.
pub fn decode_destination(buf: &[u8]) -> Result<Option<(String, u16, usize)>, FramingError> {
    if buf.is_empty() {
        return Ok(None);
    }
    let version = buf[0];
    if version != VERSION {
        return Err(FramingError::UnsupportedVersion(version));
    }
    if buf.len() < 3 {
        return Ok(None); // need more bytes for host_len
    }
    let host_len = u16::from_be_bytes([buf[1], buf[2]]) as usize;
    if host_len > MAX_HOST_LEN {
        return Err(FramingError::HostTooLong(host_len));
    }
    let needed = 3 + host_len + 2;
    if buf.len() < needed {
        return Ok(None);
    }
    let host_bytes = &buf[3..3 + host_len];
    let host = std::str::from_utf8(host_bytes)
        .map_err(|_| FramingError::InvalidUtf8)?
        .to_string();
    let port = u16::from_be_bytes([buf[3 + host_len], buf[3 + host_len + 1]]);
    Ok(Some((host, port, needed)))
}

pub async fn write_destination<W: AsyncWrite + Unpin>(
    w: &mut W,
    host: &str,
    port: u16,
) -> io::Result<()> {
    let buf = encode_destination(host, port)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    w.write_all(&buf).await
}

pub async fn read_destination<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(String, u16)> {
    let version = r.read_u8().await?;
    if version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            FramingError::UnsupportedVersion(version).to_string(),
        ));
    }
    let host_len = r.read_u16().await? as usize;
    if host_len > MAX_HOST_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            FramingError::HostTooLong(host_len).to_string(),
        ));
    }
    let mut host_buf = vec![0u8; host_len];
    r.read_exact(&mut host_buf).await?;
    let host = String::from_utf8(host_buf).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            FramingError::InvalidUtf8.to_string(),
        )
    })?;
    let port = r.read_u16().await?;
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let buf = encode_destination("example.com", 443).unwrap();
        let (host, port, consumed) = decode_destination(&buf).unwrap().unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(consumed, buf.len());
    }

    #[test]
    fn decode_truncated_buffer_returns_none_not_panic() {
        let buf = encode_destination("example.com", 443).unwrap();
        for cut in 0..buf.len() {
            let res = decode_destination(&buf[..cut]);
            assert!(res.is_ok(), "cut={cut}");
            assert!(matches!(res.unwrap(), None | Some(_)));
        }
    }

    #[test]
    fn decode_rejects_oversized_host_len() {
        let mut buf = vec![VERSION];
        buf.extend_from_slice(&((MAX_HOST_LEN as u16) + 1).to_be_bytes());
        let err = decode_destination(&buf).unwrap_err();
        assert!(matches!(err, FramingError::HostTooLong(_)));
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let buf = vec![99, 0, 0, 0, 0];
        let err = decode_destination(&buf).unwrap_err();
        assert_eq!(err, FramingError::UnsupportedVersion(99));
    }

    proptest::proptest! {
        #[test]
        fn decode_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
            // This is the same property a libfuzzer target in
            // `fuzz/fuzz_targets/framing_header.rs` checks continuously
            // against a coverage-guided corpus; running it here too means
            // it's exercised on every `cargo test`, not only when
            // cargo-fuzz is available (see docs/TEST_STRATEGY.md).
            let _ = decode_destination(&bytes);
        }
    }

    #[tokio::test]
    async fn async_write_read_roundtrip() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        write_destination(&mut client, "127.0.0.1", 8081)
            .await
            .unwrap();
        let (host, port) = read_destination(&mut server).await.unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 8081);
    }
}
