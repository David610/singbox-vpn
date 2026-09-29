# Arcana singbox-vpn — Batch 6 status (2026-09-29)

Branch: `claude/arcana-data-plane-production-final`. Does not redo batches
1-5; picks up exactly where `ARCANA_BATCH5_STATUS_2026-09-29.md` (and its
addendum, `3cfea4f`) left off.

`git fetch --all --prune` showed `origin/main` unchanged since batch 1
(merge-base(HEAD, origin/main) == origin/main HEAD) — no rebase/merge
needed.

## Task 1 — triage of the six pre-existing failure groups

All six groups batch 5 flagged were reproduced independently in this
batch (Windows-native and/or a **second, freshly re-verified** WSL Ubuntu
24.04 container — not the same one batch 5 used), root-caused for real,
and either fixed in code or documented with the exact mechanism. None
were left as an unexplained "environment" shrug.

### 1. `admin::cli` (2 failures) — real bug, fixed

`render_config_noop_reconcile_does_not_restart_singbox` and
`render_config_require_applied_succeeds_on_true_noop` asserted
`.stdout(contains("already current"))`. Batch 3's `93a027b` ("route
render/apply diagnostics to stderr, fix real --json stdout pollution")
intentionally moved this exact diagnostic
(`apps/admin/src/main.rs:2050`, `eprintln!("sing-box authorization
config is already current; no reload needed.")`) to stderr — but never
updated these two assertions, which nobody had run since. Not a
production bug: the stderr routing is the correct, intended behaviour.
Fixed by changing both assertions to `.stderr(...)`.
(`apps/admin/tests/cli.rs`)

### 2. `admin::relay_cli` (3 failures) — real bugs, fixed (one is a production bug)

All three traced to route-rule-count/shape assumptions from before C-16
(`3bb0eec`, batch 1) and the reserved-probe principal (`eca658b`, batch
1) landed, which added rules to relay/exit renders that didn't exist
when these tests were written — and this test file had apparently never
been run to completion since:

- `exit_render_is_unchanged_across_migration_to_explicit_identity`
  asserted an exit's render has **no** `route` section. C-16 gives every
  exit an unconditional `route` section now. The test's actual point —
  that a schema migration to explicit identity doesn't change the
  render — is proven by the byte-identical before/after comparison a few
  lines later, not by the absence of a route section. Fixed the stale
  assertion, kept the real one.
- `relay_render_config_applies_a_fail_closed_document` asserted
  `rules.len() == 3` for a paired relay with two `peer_endpoints`
  pointing at the same `de1.example.test:443`. `DeploymentConfig::
  relay_targets()`'s own doc comment says it is "sorted and
  de-duplicated" — it correctly emits **one** route rule for that one
  target, not two, so the real total is 2 (target + reject-all). Fixed
  the count and the rule index the test reads.
- `unpaired_relay_renders_reject_all_and_doctor_says_so` asserted
  `rules.len() == 2` for an unpaired relay with zero `peer_endpoints`
  and no reserved-probe user, i.e. zero relay targets. The correct
  count is 1 (just the fail-closed reject-all). Fixed the count.

  Fixing that count surfaced a **second, previously-hidden failure one
  assertion later in the same test**: the CLI's own `doctor` L2 check
  (`apps/admin/src/main.rs`, `NodeRole::Relay` branch) computes
  `fail_closed` from `rules.len() == targets.len() + 2 +
  hairpin_rule_count`. The `+ 2` hardcodes "the reserved-probe loopback
  exception rule is always present" — true before Phase 4 (`eca658b`)
  made that rule conditional on an active reserved-probe user existing
  (`if !probe_ids.is_empty() { ... }` in `server.rs`), false since. For
  a relay with **no** probe user (an entirely normal, safe state — e.g.
  before `vpn-admin user create-probe` has run), the doctor's arithmetic
  demanded a rule count one higher than the renderer ever produces, so
  `doctor` reported **"relay forwarding policy is NOT fail-closed"** for
  a relay that actually *was* correctly fail-closed. This is a real
  production diagnostics bug — an operator running `vpn-admin doctor` on
  a freshly-provisioned, correctly-configured unpaired or not-yet-probed
  relay would see a false negative telling them their relay is unsafe
  when it is not. Fixed by making the arithmetic detect the
  probe-loopback rule's actual presence (it uniquely carries both
  `auth_user` and `ip_cidr`, distinguishing it from the hairpin rules,
  which carry `auth_user` with `action == "sniff"` or `domain_suffix`
  instead) rather than assuming it. Verified: `cargo test -p admin
  --test relay_cli` — 15/15 passing on real Linux (WSL).
  (`apps/admin/src/main.rs`, `apps/admin/tests/relay_cli.rs`)

### 3. `compat-config::reality_decoy_budget` (1 failure) — environment (binary version), not fixed in code

Root cause confirmed directly: the WSL container's installed `sing-box`
was **1.13.19**; `deploy/lib/versions.env` pins production to
**1.14.1**. The test's own doc comment explains exactly this: 1.13.19
had a REALITY decoy-certificate-budget bug ("REALITY: processed invalid
connection" once the decoy's Certificate TLS record exceeds ~8192
bytes) that 1.14.1 fixed; this test is the forward regression guard for
that fix. Installing the *actually pinned* 1.14.1 binary
(`https://github.com/SagerNet/sing-box/releases/download/v1.14.1/
sing-box-1.14.1-linux-amd64.tar.gz`) and re-running made it pass
immediately — no code change, pure version mismatch in the dev
container versus the project's own pin. Not touched in code; flagging
for whoever maintains CI/dev-container provisioning to pin the sing-box
binary install to `deploy/lib/versions.env`'s `SINGBOX_VERSION` rather
than whatever the distro/container image happened to ship.

### 4. `compat-config::two_hop_system` (17 of 22 failures) — real, deterministic, root-caused; NOT a sing-box version or WSL-networking artifact (contradicts batch 5's guess); NOT resolved this batch

This is the most important finding of this batch, and it directly
answers the task's central question: **C-16 did break something here —
but it broke the test's topology, not two-hop routing itself, and not
production.**

**Evidence chain:**

1. Installing the correctly-pinned sing-box 1.14.1 (see item 3) and
   re-running `two_hop_system` reproduced the **identical** 17/22
   failures, byte-for-byte the same failing test names as with 1.13.19.
   Rules out "wrong sing-box version" as the cause for this suite.
2. The failures are `EOF` / `connection reset by peer` on the client's
   `vless[...]` outbound reaching `TARGET_IP` (`127.0.0.4`), including
   on **`s01_direct_route_client_exit_target`**, which doesn't even
   involve the relay hop — ruling out "only the two-hop/relay leg is
   broken."
3. `crates/compat-config/src/server.rs`'s `apply_c16_egress_policy`
   (C-16, `3bb0eec`, batch 1) unconditionally rejects the exit's
   `direct` outbound from reaching `127.0.0.0/8` (`C16_DENY_IPV4_CIDRS`
   in `crates/compat-config/src/model.rs`) — by design; this is the
   entire point of C-16 (block pivot to node loopback/private ranges).
   sing-box's default reject method is a TCP RST, which is exactly what
   `connection reset by peer` is.
4. `two_hop_system.rs`'s own topology doc comment (top of file) places
   `TARGET_IP = "127.0.0.4"` and `UNDECLARED_IP = "127.0.0.5"` — both
   inside 127.0.0.0/8 — to exploit that 127.0.0.0/8 is automatically
   routable to `lo` without root, so the test can simulate a
   multi-host topology (client/relay/exit/target) on one loopback
   interface. C-16 was landed in batch 1, *after* this topology was
   designed; nothing since has re-run this suite to completion and
   caught the collision (batch 5's own addendum ran it and misread the
   RSTs as a probable "sing-box binary version/WSL2-networking
   peculiarity").
5. **Definitive confirmation**: temporarily editing (locally, never
   committed) `C16_DENY_IPV4_CIDRS` to remove the `127.0.0.0/8` entry
   and re-running `s01_direct_route_client_exit_target` made it **pass**
   immediately, with no other change. Reverted immediately after
   (`git checkout -- crates/compat-config/src/model.rs`) — this
   confirms the mechanism with certainty but must never ship, since
   weakening C-16 is exactly the P0 regression this whole remediation
   exists to prevent.

**Why this is not fixed this batch:** the only security-preserving fix
is to stop using addresses inside any C-16-denied range for the test's
simulated "Internet target," which for a loopback-only harness (no
root, no real second network interface) means either (a) binding
`TARGET_IP`/`UNDECLARED_IP` as *additional* addresses on `lo` outside
every denied range (e.g. ordinary global-unicast space), which requires
`CAP_NET_ADMIN` to `ip addr add` — not available to this batch's
Windows dev environment nor, when probed, to the standard (non-root)
WSL test user (`sudo` here requires an interactive password); or (b) a
real network-namespace/veth harness. A prototype of (a) was written and
is architecturally correct (a `loopback_alias_available()` capability
probe in `prerequisites()`, gated the same way the existing
sing-box/openssl check is, with `TARGET_IP`/`UNDECLARED_IP` moved to
`93.184.216.34`/`.35`), but could not be fully, honestly validated
end-to-end: the only privilege-escalation path available in this
sandbox (`unshare --user --net --map-root-user`) creates a *fully
isolated* network namespace with no default route/interface at all,
which sing-box's own auto-interface-detection then fails on for a
reason unrelated to C-16 or to the address change (`network: missing
default interface`) — a side effect of the diagnostic technique itself,
not of the real fix. Shipping that prototype without being able to
prove it actually passes somewhere would be exactly the kind of
unverified claim this task explicitly warns against, so **it was
reverted** (`git checkout -- crates/compat-config/tests/
two_hop_system.rs`) rather than committed half-validated.

**Conclusion for the record:** two_hop_system's 17/22 failures are a
100%-reproducible consequence of C-16 (batch 1) correctly rejecting
127.0.0.0/8, which happens to be the same range this test's topology
was built on before C-16 existed. This is a real gap in test coverage
for the two-hop path post-C-16 (the suite has apparently not actually
passed since batch 1 landed C-16), not a production regression and not
an environment/binary-version artifact. **Left open for batch 7**, with
a concrete next step: either get a CI/dev runner with real
`CAP_NET_ADMIN` (not a namespace trick) to develop and validate the
`loopback_alias_available()` approach sketched above, or build a
proper network-namespace/veth two-host harness.

### 5. `provisioning-agent::report_queue` (1 failure) — flaky test, fixed

`queued_completion_survives_repeated_500s_and_delivers_once_the_endpoint_recovers`
mounts two 500 responses then a 200, and polls the on-disk queue for up
to `100 * 50ms = 5s` for it to drain. The retry loop's actual backoff
(`report_queue.rs`: `backoff = 1u64 << attempts.min(5)` seconds) sleeps
2s after the first failure and 4s after the second — 6s of guaranteed
sleep before the third (successful) attempt even starts, which is
already past the test's 5s bound before accounting for the request
round-trips themselves. Not a delivery bug: the queue does deliver,
just later than the test allowed for. Fixed by widening the bound to
`400 * 50ms = 20s`, with a comment deriving the 6s worst case from the
actual constants so this doesn't regress silently again. Verified 5/5
passing on real Linux (WSL).

### 6. `apps/admin`'s `concurrent_user_creates_do_not_lose_an_update` — Windows-only test-environment gap, not a production bug; `#[cfg(unix)]`-gated with an explanation

Reproduced failing on Windows-native (`alice` silently missing from
`user list` after two concurrent `vpn-admin user create` invocations)
and **passing on real Linux** (WSL Ubuntu 24.04, identical test body,
no code change). Root cause: `apps/admin/src/lock.rs::
acquire_state_lock`'s `#[cfg(not(unix))]` branch is a deliberate no-op
— it opens a throwaway `tempfile()` and returns immediately, providing
zero mutual exclusion — documented in its own comment as intentional,
since `singbox-vpn` only ships for systemd Linux hosts. On Windows the
two spawned processes therefore race for real: exactly the load-mutate-
persist race the lock exists to prevent, with nothing preventing it.
This was investigated as seriously as the task demanded ("could mean a
user mutation silently vanishes") precisely because "lost update under
concurrency" is a correctness-class bug — but the evidence is
unambiguous: the *only* difference between the failing and passing runs
is whether `flock(2)` is real (Linux) or a no-op (Windows-only, by
design, for a platform this project never deploys to). This is not a
production concurrency bug; production is Linux-only and the flock path
works. Per the task's explicit instruction ("fix the test to be
reliable... or document precisely why it's excluded on this platform
with a `#[cfg(...)]` and a comment explaining the exact mechanism"),
added `#[cfg(unix)]` to this one test with a comment naming the exact
mechanism and citing this doc. (`apps/admin/tests/cli.rs`)

## Task 2 — PR

Opened after all of the above. See the PR link recorded below.

## Final validation (this batch, on top of batches 1-5)

- `cargo fmt --all -- --check` — clean (Windows).
- `cargo clippy --workspace --all-targets -- -D warnings` — clean, 0
  warnings (Windows).
- `cargo test --workspace --no-fail-fast`:
  - **Windows-native**: 0 failures workspace-wide (full log tail
    confirmed no `FAILED`/`test result: FAILED` lines anywhere in the
    run). `concurrent_user_creates_do_not_lose_an_update` correctly
    excluded via `#[cfg(unix)]`.
  - **Real Linux (WSL Ubuntu 24.04, sing-box 1.14.1 installed to match
    the project's actual pin)**: full parallel `cargo test --workspace
    --no-fail-fast` reported 2 failing targets:
    `compat-config::two_hop_system` (17/22, root-caused above,
    explicitly left open for batch 7) and, newly observed this run,
    a single flake in `compat-config::hysteria2_interop`
    (`hysteria2_handshake_succeeds_with_matched_password`, a real UDP/
    QUIC handshake against the real sing-box binary). Re-run in
    isolation (`cargo test -p compat-config --test hysteria2_interop
    hysteria2_handshake_succeeds_with_matched_password
    --test-threads=1`) passed immediately (0.70s) — this is timing
    sensitivity under the full workspace's parallel test load (many
    concurrent real sing-box/UDP processes contending for CPU/ports),
    the same class of issue `two_hop_system` and `reality_interop`
    already guard against with their own `SERIAL` mutex; this one test
    file doesn't have that guard. Not one of this batch's six triaged
    groups (batch 5 never flagged it), not investigated further beyond
    confirming it passes in isolation, and not blocking — recorded here
    for honesty rather than left silent. A `cargo test --workspace
    --no-fail-fast -- --test-threads=1` run (serialized, matching how
    CI should probably run the real-sing-box suites) is the number
    quoted for the PR.
- `cargo audit` — 0 vulnerabilities (265 crates scanned).
- `shellcheck` on `deploy/`: the Windows checkout (`core.autocrlf=true`)
  makes several scripts appear to have CRLF-related parse errors
  (SC1017 etc.) — confirmed via `git show HEAD:<path> | file -` that
  the **committed** content is pure LF; re-ran shellcheck against a
  fresh WSL git worktree (`git worktree add`, genuine LF checkout) and
  got **zero error-severity findings** (`shellcheck -S error`, exit 0)
  across every `deploy/**/*.sh` — only info-level style notes
  (SC2317/SC2015/SC2086/SC2153), matching batch 5's "clean" baseline.
  Worktree removed after (`git worktree remove --force`).

## Commits this batch

- `docs(reviews)`, `fix/test` commits per area — see `git log` on
  `claude/arcana-data-plane-production-final` for the exact list at
  PR-open time.

## Carried-forward backlog for batch 7 (unchanged unless noted)

- **two_hop_system's C-16/loopback-topology conflict** (new this
  batch, see item 4 above) — concrete next step given.
- Host nftables not wired into install/update.
- Own-public-IP egress blocking incomplete.
- No live VPS C-16 validation.
- `claim_token`/reclaim contract still doesn't exist in either repo.
- vpn-web's `node-bootstrap.js` bootstrap fetch has no checksum
  verification (cross-repo, not fixed).
- Dev/CI container sing-box binary provisioning should pin to
  `deploy/lib/versions.env`'s `SINGBOX_VERSION` (new this batch, see
  item 3) — currently whatever the container image ships, which can
  silently diverge from what production actually runs and produce
  exactly the false-failure/false-pass risk item 3 above hit.
- Phases 13, 15, 16, 17, 18, 19, 20, 22, 23 — untouched, per this
  batch's explicit scope instruction.
