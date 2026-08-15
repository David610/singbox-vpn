# SECURITY_MODEL.md

## Cryptographic building blocks (all from audited crates, no custom crypto)

| Purpose | Primitive | Crate |
|---|---|---|
| Config/bundle signing | Ed25519 | `ed25519-dalek` |
| Transport session (TLS) | TLS 1.3 | `rustls` |
| Transport session (QUIC) | TLS 1.3 over QUIC | `quinn` + `rustls` |
| Random/nonces | OS CSPRNG | `rand` (`OsRng`) |

`crates/crypto` only wraps these — it contains no hand-rolled cipher, MAC,
or KDF construction. Every function has a doc comment stating the exact
underlying primitive.

## Separated trust domains

- **Authentication** (proving a bundle/relay comes from the operator) —
  ed25519 signatures, verified in `crates/config`.
- **Session encryption** (client↔relay bytes) — TLS 1.3 / QUIC-TLS, handled
  entirely by rustls/quinn; the application layer never sees key material.
- **Configuration signing** — a distinct key hierarchy from session
  encryption (ADR-0008); compromising a relay's TLS private key does not
  yield the ability to sign new config bundles, and vice versa.
- **Control-plane authentication** — deferred (no control plane service
  implemented this session beyond the signing-key hierarchy itself); noted
  as a follow-up in DEPLOYMENT.md.

## Secrets handling

- Private keys are read from files with `0600` permissions checked at
  startup (`crates/crypto::keys::load_signing_key` refuses to load a key
  file with group/other read bits set, on Unix).
- `tracing` field redaction: session tokens and private key bytes implement
  a `Debug`/`Display` that never prints the underlying bytes
  (`crypto::Secret<T>` wrapper); regression tests in `crates/crypto/src/`
  asserts no test accidentally derives `Debug` on raw key bytes.
- No secret is included in telemetry events by construction — the
  telemetry event enum has no `String`/`Vec<u8>` free-form field, only
  closed enums and bucketed numerics (see TELEMETRY_DICTIONARY.md).

## Hardening applied in this slice

- All network-facing parsers (`config` bundle parsing, rendezvous response
  parsing, relay framing header) enforce explicit max-length checks before
  allocating, and are fuzz-tested (`fuzz/fuzz_targets/`).
- `relay-agent` enforces a connection cap and per-connection idle timeout
  to bound resource usage (slowloris-class mitigation).
- `services/rendezvous` uses a token-bucket rate limiter per source IP.
- Services run as the invoking user; DEPLOYMENT.md documents running them
  as a dedicated non-root service user in any real deployment (not
  enforced by the code itself, which cannot drop privileges it never
  needed to begin with in the local dev slice).
