# Addendum 3 — two_hop_system dummy-interface fix, real CI iterations (2026-09-29)

Continuation of `ARCANA_BATCH6_STATUS_2026-09-29_ADDENDUM2.md`, at `44bf309`.
That pass hit a real wall: no passwordless sudo/CAP_NET_ADMIN in the local
sandbox, so the dummy-interface fix for `two_hop_system` could be designed
but not implemented or verified locally. This pass had the identical local
constraint and used the approach ADDENDUM2 itself prescribed: implement
from careful code reading, push, and iterate on real GitHub Actions CI logs
(`singbox-validate`, which runs uncontainerized on `ubuntu-latest` with real
sudo).

## What changed

`crates/compat-config/tests/two_hop_system.rs`:

1. Moved `RELAY_IP`/`EXIT_IP`/`TARGET_IP`/`UNDECLARED_IP`/`CLIENT_IP` off
   `127.0.0.0/8` onto `192.88.99.2`-`.6` — a real, ordinary /24 that is
   simply not listed in `C16_DENY_IPV4_CIDRS` (there is no block that is
   both listed-safe and not real Internet space, so this was never going
   to be "pick a nicer reserved range").
2. Added `DummyInterfaceGuard`: creates a Linux dummy interface
   (`sudo ip link add ... type dummy`, never given a route beyond its own
   host routes), assigns each address as a `/32` host route
   (`sudo ip addr add <addr>/32 dev <iface>`). Isolation comes from Linux
   delivering packets to any locally-assigned address through the kernel's
   local routing table before the main/default route is ever consulted —
   not from the address block being "reserved" — plus the dummy interface
   having no upstream to leak through regardless.
3. Wired setup into `prerequisites()` behind a process-wide `OnceLock`
   (needed by `harness_readiness_is_owned_by_the_started_process`, which
   binds `EXIT_IP` directly without going through `Lab`). Setup failure
   panics loudly (never silently skips) once sing-box/openssl are already
   confirmed present. `DummyInterfaceGuard` implements `Drop` for
   idempotent, best-effort `ip link delete`; documented plainly that
   `OnceLock`/`static` contents are not dropped at normal Rust process
   exit, so that particular process-wide instance never actually tears
   down — acceptable on an ephemeral GitHub Actions VM, called out as a
   real limitation rather than glossed over.
4. `ci.yml`'s `singbox-validate` job needed no changes: it runs directly
   on `ubuntu-latest` (not containerized), which has passwordless sudo and
   `iproute2` by default.

Verified locally before every push: `cargo fmt --all -- --check` and
`cargo clippy --locked --workspace --all-targets -- -D warnings`, both
clean. The test itself cannot run locally (no sudo here either) — every
result below is from real CI.

## Iteration 1 — commit `66ecbe3`

Run: https://github.com/David610/singbox-vpn/actions/runs/36561053957
(job `singbox-validate`, id `109381748721`) — **FAILED**, but the dummy
interface itself worked: `test result: FAILED. 17 passed; 5 failed`, vs.
0 tests ever able to execute meaningfully under the old loopback
addressing (C-16 rejected the config before sing-box ever started).
Failures: `s03_relay_is_not_an_internet_exit`,
`s05_wrong_exit_credential_fails_after_reaching_the_relay` (both:
`first_hop_only_client`'s control check — "the authenticated first-hop
tunnel is alive" — failed with REALITY-layer `read: connection reset by
peer` / `EOF` on every retry across a 15s window),
`s12_unpaired_relay_cannot_reach_anything_as_an_exit` (route-rule-count
assertion, `left: 1, right: 2`), `s13_relay_reaches_only_the_declared_
exit_target` (`left: 3, right: 4`), `s14_reload_and_repair_preserve_
role_and_restrictions` (`left: "192", right: "127"`).

Diagnosis: s12/s13/s14 were pre-existing, address-independent test bugs
that could never be caught before because C-16 always blocked the suite
from executing far enough to hit them:
- s12/s13 asserted a "self-test" route rule
  (`render_server_config_for_deployment`'s loopback self-test exception)
  that only appears for a user with `is_reserved_probe` set. This suite's
  users never set that flag, so the rule never renders, and the asserted
  counts (2, 4) were always one too many — confirmed by reading
  `server.rs`'s rule-building code directly (`crates/compat-config/src/
  server.rs:88-106`, `relay_targets()` in `deployment.rs:1066-1086`).
- s14 hard-coded the pre-migration default `node_id` as the literal
  `"127"` — the first dot-segment of the *old* loopback `RELAY_IP`
  (`default_node_id_for_host`, `deployment.rs:128`). With `RELAY_IP` now
  `192.88.99.2`, the correct default is `"192"`.

## Iteration 2 — commit `4b91121`

Fixed s12 (`2` → `1`, reject-only), s13 (`4` → `3`, "2 exits + reject"),
s14 (hard-coded `"127"` → derived from `RELAY_IP`). Left s03/s05 as-is —
no code lead yet, not guessed at blindly.

Run: https://github.com/David610/singbox-vpn/actions/runs/36561848578
(job `singbox-validate`, id `109384335055`) — **FAILED**:
`test result: FAILED. 19 passed; 3 failed; ... finished in 84.42s`.
s12/s13/s14 now pass, confirming that diagnosis. Failures this run:
`s03_relay_is_not_an_internet_exit`,
`s05_wrong_exit_credential_fails_after_reaching_the_relay`, **and now
also** `s12_unpaired_relay_cannot_reach_anything_as_an_exit` — which
passed in iteration 1 — failing this time on its own `first_hop_only_
client` control check ("control: tunnel alive"), the identical symptom
as s03/s05, not the rule-count assertion (that part is fixed and stayed
fixed).

## Diagnosis of the remaining 3 failures (not yet fixed)

All three failures, across both runs, are the exact same signature: a
`reaches(socks, "127.0.0.1", control.port)` (or equivalent) call built
through `first_hop_only_client` — the one client construction path in
this suite that builds a minimal single-endpoint config via
`contract_endpoint()` + `render_singbox_config_from_contract()` directly,
bypassing the production `provisioning_document_with_mode_and_access_
paths()` path every other passing scenario (S1, S2, S4, S6-S11, S13-S17)
uses — fails at the REALITY/TLS layer with `read: connection reset by
peer` or `EOF`, retried for the full 15s window with no success.

Ruled out as the cause, by direct code comparison:
- **Flow mismatch**: both paths default to `VlessFlow::Vision`
  (`contract.rs:39-42`); `first_hop_only_client` passes it explicitly,
  the production path gets the same default. Identical.
- **uTLS fingerprint**: both resolve to `"chrome"`
  (`deployment.rs:492`, `render.rs:1078`). Identical.
- **Addressing itself**: S2's via-route crosses the identical client
  (`CLIENT_IP`) → relay (`RELAY_IP`) REALITY hop, over the same dummy
  interface, and passes reliably every run.
- Not deterministically tied to one scenario: s12 passed in iteration 1
  and failed in iteration 2 with the same code, meaning this is
  non-deterministic/flaky, not a fixed logic bug reachable by more
  reading — s03/s05 failed both times, which could mean either a
  consistently-reproduced race or simply bad luck twice.

Best working hypothesis (not yet confirmed against a relay-side log —
production log level suppresses per-connection REALITY events by
design, per this suite's own `relay_and_exit_logs_carry_no_credentials_
and_no_rejected_destinations` test, so the rejection reason isn't
visible without a temporary, deliberately-not-shipped log-level bump):
this is a real timing/protocol characteristic of XTLS Vision flow under
the dummy interface's standard 1500-byte MTU path (vs. loopback's
65535-byte MTU, which was implicitly true for every test before this
change), not a bug introduced by the addressing fix itself, and not
something guessable further without instrumented access to the relay's
REALITY handshake internals.

## Current state

- `gh pr checks 123`: only `singbox-validate` fails, every other check
  (fmt, clippy, docs, test, all 7 `os-matrix` targets, audit, license,
  secret-logging, shell, workspace-version-consistency,
  no-legacy-identity, codeql-rust, codeql-actions, release-container-
  smoke) passes.
- `two_hop_system`: 19/22 passing in real CI (run `36561848578`), up from
  0 meaningfully executable under the previous loopback addressing.
  Remaining: `s03_relay_is_not_an_internet_exit`,
  `s05_wrong_exit_credential_fails_after_reaching_the_relay`,
  `s12_unpaired_relay_cannot_reach_anything_as_an_exit` (this one flaky
  rather than deterministic).
- PR #123 is **not** fully green. Not merged, per instructions.

## Recommended next step for whoever picks this up

Add a temporary (never-shipped) diagnostic pass with the relay's log
level bumped from production ("warn"/silent-per-connection) to something
that surfaces sing-box's own REALITY rejection reason
(`ERROR ... process connection from ...` or the specific REALITY fallback
log line), run once against real CI, read the actual reason, then revert
the log-level bump before merging. That single piece of missing evidence
— why the relay resets `first_hop_only_client`'s connection specifically
— is what the two remaining honest iterations here could not obtain
without guessing, and guessing further would only spend CI minutes on
speculation the task's own instructions say to stop and document instead
of continuing.
