# ADR-0003: WASM transport sandbox — deferred, not stubbed

## Context
Spec §10-§11 asks for pluggable transports executing in a sandboxed
runtime (candidate: Wasmtime/WASI) so transport code can be updated
independently and a compromised transport module can't gain arbitrary
native code execution.

## Decision
Do not implement the sandbox this session. Both shipped transports
(`direct-tls`, `noise-quic`) are compiled directly into `transport-native`
and loaded via the static `Transport` trait — there is no dynamic loading
of third-party code at all, so there is currently no code-execution attack
surface from "pluggable transports" to sandbox in the first place.

## Required shape for the follow-up (recorded so it isn't lost)
- Wasmtime host, WASI-preview2-restricted (no filesystem, no arbitrary
  sockets — only a host-provided `send`/`recv` capability matching
  `transport-api::Transport`).
- Memory/fuel limits per module instance.
- Module manifest: publisher identity, version, hash, ed25519 signature
  (reusing `crates/crypto`/`crates/config` machinery already built),
  capability declaration, min/max compatible client version, revocation
  check against the same `revoked_key_ids` list used for rendezvous
  bundle-signing-key revocation (ADR-0008) — one revocation mechanism, not
  two.
- Signature verification happens before the module bytes are ever handed
  to Wasmtime, using the existing `config` crate's verification path.

## Alternatives considered
- Native dynamic loading (`dlopen`) of transport `.so` files: rejected —
  no sandbox, defeats the entire point (spec §10 explicitly forbids this).
- Subprocess-per-transport with a narrow IPC protocol: viable alternative
  to WASM, weaker sandboxing (still a full OS process, just without shared
  memory) but simpler to implement and audit. Worth reconsidering as the
  first real implementation if Wasmtime integration proves to be the
  long pole — noted here rather than silently discarded.

## Consequences
No third-party/updatable transport exists yet; both transports ship with
the client binary and are updated only via full client releases. This is
a real limitation of the current system, stated plainly rather than papered
over with a fake "transport-runtime" crate that does nothing.
