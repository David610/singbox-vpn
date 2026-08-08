# ADR-0002: Initial transport portfolio

## Context
Spec calls for 2-3 genuinely independent transport families rather than
protocol-count maximalism, selected for failure-independence /
implementation-complexity ratio.

## Decision
Ship exactly two for this session: `direct-tls` (TCP+TLS1.3 via rustls)
and `noise-quic` (UDP+QUIC via quinn). Rationale: they fail under disjoint
censorship techniques (TCP RST/SNI-block vs. UDP filtering/QUIC block),
both use mature, audited libraries with stable APIs, and both run on Linux
without extra native dependencies (no libwebrtc, no libevent).

## Alternatives considered
- **Tor Snowflake**: requires a WebRTC/ICE stack; no mature pure-Rust
  WebRTC client library with a stable API existed to integrate safely in
  this session's time budget. Revisit via a process adapter (shell out to
  the upstream `snowflake-client` binary) rather than reimplementing.
- **obfs4**: mature Go implementation (`obfs4proxy`); a Rust-native
  reimplementation was rejected (custom crypto framing risk); a process
  adapter is the credible integration path, deferred.
- **Xray/REALITY, Hysteria2, sing-box transports**: all Go, all actively
  maintained, all plausible future process/protocol adapters. Not
  integrated this session — evaluating license (MIT/Apache-compatible for
  most) and API stability needs dedicated time not available here.
- **A third from-scratch transport** (e.g. domain fronting via CDN): more
  operational complexity (needs a real CDN account) than this local-only
  session can responsibly stand up.

## Consequences
Only two families exist today; a censor that can defeat both generic
TLS-fingerprinting and generic QUIC-fingerprinting simultaneously defeats
this slice. A third, more disguised family (browser-shaped or
volunteer-relay-shaped) is the highest-value follow-up (see final report).
