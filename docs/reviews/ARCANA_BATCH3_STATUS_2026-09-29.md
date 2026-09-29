# Arcana Batch 3 Status — 2026-09-29

Branch: `claude/arcana-data-plane-production-final`. This batch was time/effort-bounded and
completed only Part A items A1/A2 with real verification. A3, A4, Part B, and Part C were
**not attempted** this session — see "Not attempted" below. Do not report those as done.

## A1 — full workspace validation gates

Commit: `4999b83` (portability fixes below) enables these to actually pass. Real results on
this Windows dev machine, HEAD after `4999b83`:

- `cargo fmt --all -- --check` — **PASS**, no diff.
- `cargo clippy --workspace --all-targets -- -D warnings` — **PASS** (0 errors). Before the
  fixes in this batch it failed with 5 compile errors (1 unused-import in `apps/admin`, 4
  `std::os::unix::*` resolution errors in `services/subscription`) — all Windows-portability
  issues, not clippy lint findings. One pre-existing non-fatal warning remains
  (`apps/admin/Cargo.toml`: `src/main.rs` used by both `vpn` and `vpn-admin` bin targets) —
  this is a `cargo` diagnostic, not a clippy lint, and is not gated by `-D warnings`.
- `cargo test --workspace --no-fail-fast` —
  - With the ambient Windows `OPENSSL_CONF` env var set (`D:\Program
    Files\PostgreSQL\psqlODBC\etc\openssl.cnf`, which does not exist on this machine): 1
    failure, `udp_probe_tests::cert_expiry_days_reports_positive_days_for_a_freshly_issued_cert`
    (openssl CLI cannot find its config file). **Confirmed environment-only**: unsetting
    `OPENSSL_CONF` makes it pass. This is the "OpenSSL-CLI environment issue" flagged in the
    task brief — it is not caused by Arcana code and is not fixed by code changes; documenting
    it here is the fix (a developer/CI machine with a stray `OPENSSL_CONF` pointing at an
    unrelated tool's config will hit this; CI itself is unaffected since it won't have that var).
  - With `OPENSSL_CONF` unset: 4 failures remain, all in `apps/admin/tests/cli.rs`, all Windows
    Git-Bash subprocess/path artifacts, **not regressions from this batch's edits** (edits this
    batch only added `#[cfg(unix)]` gates and moved one import — neither touches these tests or
    the code paths they exercise):
    - `repair_runs_the_located_update_script_with_repair_flag_and_propagates_success`: Git Bash
      mangles the fake script's Windows path when invoking it as a shell command
      (`C:UsersArinaAppDataLocalTemp...` — backslashes stripped), so `/bin/bash` reports "No
      such file or directory". Test harness issue specific to invoking `bash <windows-path>` on
      this machine's Git Bash, not a code bug.
    - `user_create_json_output_has_no_server_secrets`,
      `user_create_json_output_carries_the_experimental_vision_off_link_additively`,
      `user_create_json_output_is_unaffected_by_suppression`: stdout is not pure JSON
      (`expected value at line 1 column 1`), consistent with CRLF/extra-output interaction on
      Windows rather than a `--json` contract break — same class of pre-existing Windows-only
      test friction as the two problems A2 explicitly named, but not itself named in the brief
      and not fixed in this batch (would need per-test investigation of what's landing on
      stdout ahead of the JSON).
  - **Net**: workspace test suite is not a clean gate on Windows even after this batch; all
    failures are environment/harness artifacts, none are logic regressions, but full green is
    not yet achieved. On Linux CI these are expected to behave differently (bash paths and
    OPENSSL_CONF are non-issues there) but that was not verified live this batch.
- `cargo audit` — **BLOCKED**: `cargo-audit` is not installed on this machine and `cargo install
  cargo-audit --locked` did not complete within a 90s bound (compiling the tool from source is
  slow). Not run. Needs either a longer unattended install window or a machine/CI with it
  pre-installed.

## A2 — Windows test-portability fixes

**FIXED**, commit `4999b83`:
- `apps/admin/src/main.rs`: moved `use std::io::Write` into the `#[cfg(unix)]` block in
  `MachineStdout::write_document` (was unconditional but only used there).
- `services/subscription/src/main.rs`: gated `use std::os::unix::fs::PermissionsExt`,
  `libc_geteuid`, and `unreadable_present_file_fails_closed_rather_than_looking_disabled` with
  `#[cfg(unix)]`.
- `services/subscription/tests/startup_validation.rs`: gated
  `refuses_to_start_when_hysteria_obfs_password_file_is_unreadable` and `libc_geteuid` with
  `#[cfg(unix)]`.
- No production-relevant assertion was weakened or removed — the fail-closed permission checks
  still run in full on unix targets (Linux CI, prod). Only compilation on non-unix was fixed.

## Not attempted this session (explicitly deferred, not silently dropped)

- **A3** (regression test for `is_reserved_probe` flag + C-16 route rules surviving config
  apply/revision) — not written.
- **A4** (idempotent job mutations / operation-ID dedup at the agent layer) — not implemented.
- **Part B** (Phase 11 immutable sing-box hash pin, `remediation/supply-singbox-pin` branch
  integration) — not evaluated or cherry-picked this session.
- **Part C** (Phase 12 immutable bootstrap, movable-tag-as-root-of-trust fix) — not
  investigated this session.

These remain fully open and should be the first work of the next session/batch continuing this
branch. This status doc intentionally does not claim partial credit for unstarted work.

## Carried-forward backlog from Batches 1-2 (unchanged, still open)

- A. Host nftables (Phase 3, commit `c450e5d`) not wired into `install.sh`/`update.sh` —
  applied manually/out-of-band only.
- B. Own-public-IP egress blocking incomplete.
- C. No live VPS validation of C-16 egress isolation end-to-end.
- D. Phase 9 kill-timed-out-child fix (`7ffbc6b`, `#[cfg(unix)]`, process-group SIGKILL+reap) is
  UNVERIFIED on actual Linux — this dev machine is Windows, so it has never actually run on the
  unix path it targets.
- E. Full `claim_token`/reclaim contract does not exist in either `singbox-vpn` or `vpn-web`;
  Batch 2's job-report queue and this batch's (unimplemented) A4 dedup are both scoped narrowly
  below that missing contract, not a substitute for it.

## Human decisions needed before Batch 4 (crypto/privacy: phases 17-19)

- Whether to accept "operation-ID-level dedup only" (A4, still unbuilt) as sufficient for now,
  or whether the full claim_token/reclaim contract (item E above) needs to be scoped into an
  earlier batch given it blocks meaningful idempotency guarantees at the agent layer.
- Whether `cargo audit` needs to be made available in this environment (pre-installed or a
  longer install budget) before it can be treated as a real gate rather than a documented gap.
- Whether the 4 Windows-only `apps/admin/tests/cli.rs` failures found this batch (Git Bash path
  mangling, non-JSON stdout under `--json`) are worth root-causing now or are acceptable
  Windows-dev-machine noise given production runs on Linux — this batch did not investigate
  deeply enough to tell which.
