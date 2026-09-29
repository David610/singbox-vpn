# Arcana Batch 5 status — tooling (cargo-audit/shellcheck), A3 (probe/C-16 regression test), A4 (idempotent op-id dedup) — 2026-09-29

Branch: `claude/arcana-data-plane-production-final`. Deliberately narrow
batch (3 deliverables only), per instructions, following batch 4's
pattern (batches 1-3 tried too much and stalled).

Branch state re-verified at start: `git fetch --all --prune` showed
`origin/claude/arcana-data-plane-production-final` unchanged from local
HEAD (`074854b`, batch 4's own status commit); no rebase/merge needed.

## Task 1 — tooling: cargo-audit and shellcheck — **FIXED**

Commit `36afbe6`.

Both tools had never actually run in this whole remediation (batch 3's
`cargo install cargo-audit` timed out at 90s; shellcheck was never
attempted). Installed both for real this time:

- **cargo-audit**: `choco install shellcheck` failed outright (non-admin
  NuGet lock contention — this machine has no admin rights available in
  this session) and `cargo install cargo-binstall --locked` failed to
  *compile* (`vergen@10.0.3 requires rustc 1.96.0`, this machine's
  default toolchain resolves to an older one in some contexts). Fell
  back to a prebuilt binary: downloaded
  `cargo-audit-x86_64-pc-windows-msvc-v0.21.2.zip` directly from
  RustSec's GitHub release assets. That version could load the advisory
  database once but failed to *parse* it (`unsupported CVSS version:
  4.0` — the advisory DB now contains CVSS 4.0 entries v0.21.2 predates).
  Ran `cargo install cargo-audit --locked --force` in the background
  with no artificial timeout this time (batch 3's actual mistake): took
  **11m27s**, succeeded, produced cargo-audit v0.22.2.

  **Real result**: `cargo audit` against this workspace — 265 crate
  dependencies scanned against 1277 loaded advisories, **zero
  vulnerabilities found**, exit 0. Nothing to triage.

- **shellcheck**: same `choco install` admin-lock failure. Downloaded
  `shellcheck-v0.10.0.zip` directly from koalaman/shellcheck's GitHub
  release assets, extracted `shellcheck.exe` to `~/.cargo/bin/`.

  **Real result**: ran at `-S warning` against every `*.sh` under
  `deploy/` plus root `install.sh`/`uninstall.sh` (87 scripts total). A
  raw run against the working tree reported dozens of SC1017 "literal
  carriage return" findings on every comment line — this Windows
  checkout has `core.autocrlf=true`, which converts the repo's LF blobs
  to CRLF on checkout. Confirmed via `git show HEAD:<file>` that the
  actual repository content is LF-only; this is a local-checkout
  artifact, not a repository defect (same category as batch 3/4's
  Windows-Git-Bash findings). Re-ran shellcheck against a CRLF-stripped
  copy of the real git blob content for an accurate result:

  - **One real finding**: SC2034 in
    `deploy/lib/tests/test-release-reproducibility.sh:499` — `rc=$?`
    captured by batch 4's own negative-pin test but never read. Fixed
    (removed the dead assignment). Re-ran shellcheck on the fixed file:
    clean.
  - Default-severity (style/info) scan of the four scripts this task
    named explicitly (root `install.sh`, `deploy/almalinux/install.sh`,
    `update.sh`, `nftables-egress-isolation.sh`) surfaced SC2015 (`A &&
    B || C`), SC2016 (single-quoted `bash -c '...'`), SC2086 (unquoted
    var in a `curl` URL port), SC2153 (possible var misspelling) — all
    info-level, reviewed individually:
    - SC2016: correct as written — the single quotes are intentional so
      `$1` expands inside the invoked subshell, not the caller.
    - SC2153 (`SINGBOX_VERSION`/`singbox_version`): false positive —
      `SINGBOX_VERSION` is a global assigned in a separately-sourced pin
      file (verified via a loop at line 53 that checks it's set), not
      unset; shellcheck's cross-file analysis doesn't see that source.
    - SC2015/SC2086: pre-existing low-risk style patterns in
      non-security-critical paths (SELinux port-label bookkeeping, a
      subscription-backend health-check retry loop's port number).
      Left as-is — fixing would touch working, reviewed control flow
      for no behavior change, out of the "fix real issues" bar this
      task set. None of these are in `nftables-egress-isolation.sh`,
      the batch-1 script this task named most specifically, which is
      clean at every severity level.

## Task 2 — A3: probe/C-16 regression test — **FIXED**

Commit `ac75514`.

New test in `apps/admin/tests/cli.rs`:
`apply_revision_preserves_reserved_probe_confinement_and_c16_policy_across_reapply`.

Batch 1 introduced `is_reserved_probe`
(`crates/compat-config/src/model.rs`) and the C-16 exit egress deny-list
(`apply_c16_egress_policy`/`apply_probe_user_confinement` in
`crates/compat-config/src/server.rs`, called from
`render_server_config_for_deployment`). Batches 2/3 both flagged that no
test proved a config apply/revision **composition** couldn't silently
drop either one on re-render.

The test drives the real production path — `cmd_apply_revision` ->
`apply_users_and_save` -> `render_and_apply_singbox_config` — via the
compiled `vpn-admin apply-revision` CLI (same fake-`sing-box`/
fake-`systemctl` harness this file's other apply-revision tests already
use), not a unit test calling the render functions directly. It:

1. `vpn-admin user create-probe` (the only path that ever sets
   `is_reserved_probe: true` — never reachable from customer/vpn-web
   input).
2. Applies revision 7: the current `users.json` (probe + nothing else)
   plus one customer, through the real wire-document shape
   (`store::parse_users_bytes`'s versioned envelope).
3. Applies revision 8: the same set plus a second customer — the
   **re-render** case this test exists for; a bug that only manifests
   on re-apply (not first render) would be invisible after step 2 alone.
4. After each apply, asserts against the real rendered/reloaded
   `state/sing-box/config.json`:
   - `probe_confinement_rule_count() == 3` and
     `c16_egress_policy_rule_count() == 4` (both survive);
   - the first 7 `route.rules` entries are exactly `[probe x3, C-16 x4]`
     in that order (checked by rule shape: probe rules all carry
     `auth_user`, the following 4 don't) — proving neither block is
     dropped *nor reordered* such that an unrestricted rule could ever
     be evaluated ahead of either;
   - `is_reserved_probe: true` survives in `users.json` for exactly one
     user, alongside the expected customer count.

`#[cfg(unix)]` (needs the file's existing fake-binary harness, same as
every other apply-revision test in this file).

**Evidence**: `cargo check -p admin --tests` (Windows) — clean, only
the expected unix-only APIs excluded, no syntax/type errors in the
reachable graph. Actually **run on real Linux** (WSL Ubuntu 24.04,
rustc 1.98.1): `cargo test -p admin --test cli
apply_revision_preserves_reserved_probe` — **1 passed, 0 failed**.

## Task 3 — A4: idempotent operation-id dedup — **FIXED**

Commit `872ddc6`.

Scoped per instructions: operation-ID-level dedup only, in the node
agent — not the full `claim_token`/reclaim contract redesign batch 2
found missing (backlog item E, still open).

**Dedup key**: `Job.id` (`apps/provisioning-agent/src/worker_client.rs`)
— the identifier vpn-web's Worker already assigns and sends per job,
already used by `ReportQueue` for `/complete`/`/fail` delivery. No new
wire field invented.

**New module**: `apps/provisioning-agent/src/op_dedup.rs` —
`OpDedupLog`, a disk-backed (0600) `job_id -> outcome` log using the
same atomic-write-then-rename persistence pattern as the existing
`report_queue.rs`. Retention bounded two ways: `MAX_ENTRIES = 4096`
(oldest evicted first) and a 24h TTL, both enforced on every `record()`
— chosen because the risk window this covers (agent crash/restart
between a vpn-admin mutation and the completion report being durably
enqueued) is seconds-to-minutes, and 24h comfortably covers a node down
for routine maintenance catching up on redeliveries afterward.

**Wiring** (`main.rs`): `poll_once`'s body after `client.claim()` was
factored into `apply_job()`, both so it's directly testable against a
hand-built `Job` without mocking `/api/agent/claim`, and so it's the
literal production code path under test, not a reimplementation.
`apply_job` looks up `op_dedup` before calling `dispatch::run_job`; a
recorded **success** short-circuits re-application and replays the
stored result to the report queue. A recorded **failure** is
deliberately *not* replayed — nothing mutated on a failure, so
redelivery should be free to retry rather than getting permanently
wedged on a possibly-transient cause.

**Config**: new `op_dedup_file` setting in `config.rs`
(`/var/lib/vpn-provisioning-agent/op-dedup.json` default),
`serde(default)` so existing `deployment.toml`/`provisioning-agent.toml`
files keep working unmodified.

**Tests** (`main.rs::op_dedup_integration_tests`, `#[cfg(unix)]` — needs
a real child process via `tokio::process::Command`, same precedent as
`dispatch.rs`'s existing Phase 9 test): CREATE_USER, ENABLE_USER,
DISABLE_USER, ROTATE_CREDENTIALS, APPLY_NODE_REVISION each applied
**twice** via `apply_job()` against a fake `vpn-admin` shell script,
with the `ReportQueue`/`OpDedupLog` handles **dropped and reconstructed
at the same path** between the two calls — simulating the agent process
restarting, not merely an in-process retry. Each test asserts the fake
`vpn-admin`'s real subcommand ran **exactly once** (a call-log line
count that can only grow when the real subprocess actually executes).
The `APPLY_NODE_REVISION` case additionally asserts (via wiremock's
`.expect(1)`) that the Worker's `GET /api/agent/revision/:revision` was
fetched exactly once too — proving the dedup hit skips the fetch, not
just the vpn-admin invocation.

There is **no `DELETE_USER` job type** in this agent's dispatch table
today (`dispatch.rs`'s `run_job_once` match covers CREATE_USER,
SET_EXPIRY, CLEAR_EXPIRY, ENABLE_USER, DISABLE_USER,
ROTATE_SUBSCRIPTION_TOKEN, ROTATE_CREDENTIALS, APPLY_NODE_REVISION) —
not invented here since it isn't part of the current wire contract;
the task's list of verbs is a superset of what actually exists.

**Evidence**:
- `cargo test -p provisioning-agent` (Windows) — **74 passed, 0
  failed**, including all 6 `op_dedup` unit tests (round-trip, replace
  not duplicate, restart survival via reopen, TTL pruning, bounded
  eviction).
- `cargo test -p provisioning-agent op_dedup_integration_tests` on real
  Linux (WSL Ubuntu 24.04, rustc 1.98.1) — **5 passed, 0 failed** (all 5
  verbs, restart simulation included).

## Validation

- `cargo fmt --all -- --check` — clean on Windows and on real Linux
  (WSL).
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
  (0 warnings) on Windows for every crate touched this batch
  (`provisioning-agent`, `admin`; full-workspace clippy from batch 4's
  own run was already clean and nothing outside these two crates
  changed).
- `cargo test --workspace --no-fail-fast` (Windows, OPENSSL_CONF unset
  per batches 3/4's finding) — **one pre-existing failure**,
  `concurrent_user_creates_do_not_lose_an_update` (apps/admin/tests/
  cli.rs), confirmed via `git stash` to fail identically on batch 4's
  own HEAD (`074854b`) before any batch 5 change — **not introduced by
  this batch**, not one of this batch's three deliverables, left
  untouched and flagged below for a human/later-batch decision. Every
  other target: pass. Full log at `/tmp/tools/fulltest.log` (ephemeral,
  not committed).
- `cargo audit` and `shellcheck` — Task 1's deliverable, see above; both
  ran for real, both clean save the one shellcheck finding already
  fixed.
- Real Linux (WSL Ubuntu 24.04) spot-checks: `cargo fmt --all --
  --check` clean; the new op_dedup integration tests (5/5) and the new
  A3 test (1/1) confirmed passing on the actual unix-gated code paths
  this Windows dev machine cannot execute.

## Carried-forward backlog (for a human to scope batch 6 from)

Unchanged items from batches 3/4, still open:

- **A)** host nftables not wired into install/update.
- **B)** own-public-IP egress blocking incomplete.
- **C)** no live VPS C-16 validation.
- **D)** Phase 9 kill-timed-out-child — **incidentally verified this
  batch** while setting up the WSL Linux environment for A3/A4:
  `cargo test -p provisioning-agent timeout_kills_the_child` on real
  Linux (WSL Ubuntu 24.04) — 1 passed, 0 failed. Not one of this
  batch's three deliverables, but worth closing out since it removes a
  previously-open item at no extra cost.
- **E)** `claim_token`/reclaim contract still doesn't exist in either
  repo (vpn-web or singbox-vpn).
- **F)** A3 and A4 — **closed this batch**.
- **G)** vpn-web's `node-bootstrap.js` bootstrap fetch still has no
  checksum verification (documented by batch 4, not fixed — cross-repo,
  needs a vpn-web-side change).
- Tooling (cargo-audit/shellcheck) — **closed this batch**.

Not carried forward from batch 4 but newly observed this batch, **not
one of the three deliverables and deliberately not touched**:
`concurrent_user_creates_do_not_lose_an_update` fails deterministically
on this Windows dev machine (confirmed pre-existing on batch 4's HEAD,
not a batch 5 regression) — worth a human decision on whether this is a
real lost-update bug in `vpn-admin`'s concurrent-create path or another
Windows-filesystem-locking artifact (same category as several batch 3/4
findings), since it was not previously called out in any prior batch's
status doc.

Remaining phases not in this batch's scope (per the task's explicit
instruction not to expand into these): 13 (real protocol health), 14
(revision-apply state preservation — now *partially* covered by A3,
which proves the render-layer composition survives re-apply, but not
every other kind of state a revision-apply could still lose), 15
(uninstall ownership), 16 (probe-target SSRF), 17 (2-hop credential
linkability), 18 (exit DNS privacy), 19 (REALITY decoy resilience), 20
(systemd hardening), 22 (update/rollback), 23 (observability).

**On opening the PR**: not opened this batch, per instructions. Given
how much has accumulated across 5 batches on this one branch
(structural probe/C-16 policy, host nftables, lease pool, report queue,
timeout-kill, sing-box hash pin ordering, bootstrap checksum, cli.rs
stdout-pollution fix, this batch's tooling/A3/A4), a human should decide
whether to open one large PR now or split by theme (e.g. security
policy changes vs. reliability/agent changes vs. tooling/test-only
changes) before merge review gets unmanageable — this is a judgment
call outside a single batch's scope.

## Commits this batch

- `36afbe6` — `chore(tooling): install cargo-audit/shellcheck, fix real finding (Batch 5 Task 1)`
- `ac75514` — `test(admin): A3 regression test — probe confinement + C-16 survive revision reapply`
- `872ddc6` — `feat(agent): A4 — operation-id dedup for idempotent job application`

All pushed to `origin/claude/arcana-data-plane-production-final`. PR not
opened, per instructions.
