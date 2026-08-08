# TEST_STRATEGY.md

| Layer | Technique | Location |
|---|---|---|
| config/rendezvous parsing | fuzzing (`cargo-fuzz`) | `fuzz/fuzz_targets/{config_bundle,rendezvous_response}.rs` |
| state machine invariants | property tests (`proptest`) | `crates/failure-classifier/tests` |
| scoring bounds & selection distribution | property tests | `crates/policy/tests` |
| serialization round trips | property tests | `crates/config/tests` |
| signed bundle verification | unit tests (valid/expired/bad-sig/revoked/wrong-schema) | `crates/config/tests` |
| transport connect/health/close | unit + local loopback integration | `crates/transport-native/tests` |
| end-to-end vertical slice | integration test spinning up rendezvous+relay+test-service+client in-process | `tests/e2e.rs` |
| failure independence | integration tests that kill one transport/endpoint and assert the other still works | `tests/failure_independence.rs` |
| hostile network | `tc netem` + netns, `--ignored`, requires root | `tests/hostile_network/` |
| performance | `criterion` benches | `crates/*/benches` |

## What "done" means for this session (spec §51 checklist, honestly scored)

1. client establishes a real encrypted path through local relay infra — **done** (`tests/e2e.rs`)
2. traffic flows end-to-end — **done**
3. ≥2 independent transport implementations — **done** (direct-tls, noise-quic)
4. dynamic transport selection — **done** (`policy::select_next`)
5. dynamic endpoint selection — **done**
6. failures classified — **done** (`network-state::FailureCategory`)
7. endpoint failure doesn't penalize whole transport — **done + tested** (invariant test)
8. fallback occurs correctly — **done** (`tests/failure_independence.rs`)
9. config bundles cryptographically verified — **done**
10. expired/invalid bundles rejected — **done + tested**
11. kill-switch logic tested — **partially**: policy tested against a mock backend only; no real OS backend this session (documented gap)
12. DNS leak behavior tested — **partially**: no local-resolver leak path exists in the current proxy-based slice (no DNS resolution happens client-side at all yet); real TUN/DNS routing is deferred, so this is "not applicable yet" rather than "tested and passing" for the deferred scope
13. telemetry obeys privacy policy — **done** (schema can't carry disallowed fields; see PRIVACY_MODEL.md)
14. hostile-network simulations exist — **partially**: script + test written and reviewed (`tests/hostile_network/`), not executed in this session (sandbox lacks `iproute2`/`tc`); deterministic failure-independence *is* proven without netem (`tests/failure_independence.rs`)
15. CI passes — **done** for the checks CI actually runs (fmt/clippy/test/build); see `.github/workflows/ci.yml`
16. docs explain how to reproduce — **done** (DEPLOYMENT.md)

Item 11 and 12 are the two honestly-partial items; they are not claimed as
fully done anywhere else in the docs.
