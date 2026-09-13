# Two-hop (Privacy+) route — local system tests and acceptance scenarios

**Status (2026-09-13):** relay role, fail-closed relay forwarding and the
Core `detour` provisioning path are implemented and covered at three
automated levels. Evidence is **CODE-VERIFIED / CI-VERIFIED** only (see
[DEVICE_ACCEPTANCE_TESTS.md](DEVICE_ACCEPTANCE_TESTS.md) for the canonical
vocabulary). The loopback system tests below are CI-VERIFIED *local system*
evidence: real `sing-box` processes on one Linux host. They are **not**
SERVER-VERIFIED, DEVICE-VERIFIED, or evidence about any real network.

## What is being tested

```text
client ── "Germany · Direct" ─────────────────────────────► EXIT (role = exit) ─► Internet
client ── "Germany · via Russia" ─► RELAY (role = relay) ──► EXIT (role = exit) ─► Internet
```

* The **relay** is a normal singbox-vpn node installed with `--role relay`.
  Its own VLESS+REALITY listener (`reality-1`) is first-hop infrastructure,
  declared by an `[[access_paths]]` entry with `via_endpoint_id = "reality-1"`.
  Its rendered sing-box config forwards **only** to the exits declared in
  its `deployment.toml` and ends in an unconditional `reject`. An unpaired
  relay rejects everything. The single extra allowed destination is the
  node's own loopback subscription health port, which the protocol
  self-test (`vpn-admin doctor --protocol`, run by install/update
  acceptance) uses to prove a real first-hop handshake.
* The **exit** is an ordinary node (`role = exit`); its config is
  byte-for-byte the pre-relay single-server document.
* The **client document** is the first-party provisioning contract served by
  the relay: the via route is the exit outbound with `detour` set to a hidden
  first-hop outbound. The first hop is never a selectable exit, never a share
  link and never a capability. Formats that cannot express `detour`
  (`vless://`/`hysteria2://` share links) omit the via route instead of
  emitting a direct link. A user with nothing routable gets HTTP 503
  `no_selectable_route`, never the relay as an exit.

Credential scopes are independent: the subscription bearer token (relay),
credential **A** (the relay user's VLESS UUID, authenticates the first hop)
and credential **B** (issued by the exit, recorded on the relay as
`user peer set … de1-direct --uuid B`, reused by the via route through
`credential_ref`). The relay's server config never contains B, and neither
node ever holds the other's private key.

## Test pyramid

| Level | Where | What it proves |
|---|---|---|
| 1 Unit | `crates/compat-config/src/deployment.rs` tests, `crates/compat-config/tests/relay_role_policy.rs` | Role/node-id parsing, relay declaration validation (fail closed), relay target derivation, byte-identical exit rendering, relay rule shape, credential separation, share-link omission, migration keeping role, fresh-install templates loading |
| 2 Integration | `apps/admin/tests/relay_cli.rs`, `services/subscription/src/lib.rs` (`relay_subscription_tests`), `deploy/lib/tests/test-node-identity.sh` | `vpn-admin` against persisted relay/exit state (render, doctor, status, links, validate/migrate, backup/restore role guard, secret-free output), the real HTTP router on a relay, installer/update identity handling and rollback |
| 3 Local system | `crates/compat-config/tests/two_hop_system.rs`, CI step "Two-hop relay system tests S1-S15", CI step "doctor --protocol against a real live UNPAIRED RELAY" | Real sing-box client → relay → exit → HTTP target on loopback |
| 4 Real acceptance | **Out of scope here** | Real VPSs, providers, devices, networks — see the last section |

## Local system harness

Topology (distinct loopback addresses on one Linux host):

| Role | Address | Notes |
|---|---|---|
| client | `127.0.0.1` (SOCKS) | real sing-box running the provisioning document's embedded Core config; the test only adds a SOCKS inbound and selects a route tag (client-owned policy) |
| relay | `127.0.0.2` | `role = "relay"`, rendered by `render_server_config_for_deployment` |
| exit | `127.0.0.3` | `role = "exit"`, same renderer |
| declared target | `127.0.0.4` | HTTP server counting requests |
| undeclared target | `127.0.0.5` | HTTP server that must never be hit through the relay |
| REALITY decoy | `localhost` | local OpenSSL TLS 1.3 server (no third-party CDN) |

Every applied config goes through `apply_config_atomically` with the real
`sing-box check` as validator. Credentials are generated per run; no real
secret is used or logged.

Run locally (Linux, pinned sing-box on `PATH` or `SING_BOX_BIN`, `openssl`):

```bash
SINGBOX_VPN_REQUIRE_REAL_INTEROP=1 cargo test -p compat-config --test two_hop_system
```

Without the variable the suite skips when prerequisites are missing; CI sets
it so a skip is a failure, and asserts every scenario name ran and passed.

### Scenarios

| Id | Scenario | Expected | What is actually proven locally | What is NOT proven |
|---|---|---|---|---|
| S1 | client → exit → target (direct route) | PASS | provisioning document's direct route authenticates with B and carries traffic | real reachability of an exit |
| S2 | client → relay → exit → target (via route) | PASS; fails once the relay is stopped | the via route is a Core `detour` chain that depends on the relay | knowledge separation between hops on real hosts |
| S3 | first hop used as an exit | FAIL for every target; control through the same tunnel succeeds | relay policy, not auth or liveness, refuses; target request counters stay 0 | — |
| S4 | wrong first-hop credential | FAIL | relay rejects unknown A | — |
| S5 | wrong exit credential | FAIL; the relay still accepts A | exit rejects unknown B after the relay hop | — |
| S6 | exit unavailable | via and direct FAIL; no request reaches the target | no fallback path exists in the document or at the relay | — |
| S7 | relay unavailable | via FAIL; explicitly selected direct PASS | direct and via routes are independent | — |
| S8 | user disabled (at relay, then at exit) | FAIL after re-render + restart | revocation through the production render/apply path | reconnect behaviour of real clients |
| S9 | user expired | FAIL after reconciliation | expiry takes effect when `render-config` reconciles (the expiry timer), not instantly | timer scheduling on a real host |
| S10 | rotate A, then rotate B | old credential FAIL, new PASS; rotating one never changes the other | independent credential scopes end to end | — |
| S11 | dangling via / credential_ref, UDP capability, invalid role, missing node_id, future schema, truncated file | load/render FAIL; on-disk config unchanged | fail-closed configuration generation | — |
| S12 | unpaired relay | every target, the exit port, and other loopback ports FAIL; self-test PASS | reject-all policy on a live relay | — |
| S13 | declared exit by IP and by DNS name | declared PASS; undeclared host, same host other port, IP form of a name-declared host FAIL | exact-destination allowlist | real DNS behaviour |
| S14 | migrate + repeated render (repair/update) and v1→v2 relay migration | role stays relay, rules unchanged/idempotent, via PASS, undeclared FAIL | repair/update never loosens relay policy | the installer running on a real host |
| S15 | failed candidate apply, SIGKILL + restart from disk | on-disk config never unrestricted; restricted after restart | no temporary unrestricted state | systemd restart semantics |
| log privacy | relay/exit logs at the production level after S2/S3 traffic | no credential, key, token hash or tunnelled destination | default logging introduces no activity history | long-term log retention on a real host |

A mutation check was run while developing this suite: forcing the relay
renderer to return the exit document made S3, S12 and S13 fail, so the
negative assertions detect a fail-open relay rather than passing vacuously.

## Reusing the scenarios for real acceptance

The scenario list above is the checklist for the separate real-acceptance
phase. Each row keeps its expectation; only the environment changes:

* relay and exit on two real VPSs (ideally separate providers), installed with
  `--role relay` / `--role exit`;
* client on a real device through the real network under test;
* targets replaced by operator-controlled HTTP endpoints plus real services;
* additional evidence required that loopback cannot give: packet captures on
  each hop (what the relay and exit observe), provider/ASN separation, IPv4/IPv6
  and DNS leak behaviour, handover, latency/throughput, and Russian ISP/mobile
  reachability.

Results from that phase are recorded in
[DEVICE_ACCEPTANCE_TESTS.md](DEVICE_ACCEPTANCE_TESTS.md) as SERVER-VERIFIED /
DEVICE-VERIFIED rows. Nothing in this document upgrades those rows.
