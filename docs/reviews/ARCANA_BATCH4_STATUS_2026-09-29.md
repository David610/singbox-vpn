# Arcana Batch 4 status — cli.rs root-cause, Phase 11 (sing-box pin), Phase 12 (bootstrap) — 2026-09-29

Branch: `claude/arcana-data-plane-production-final`. Deliberately narrow batch
(3 deliverables only), per instructions, after batch 3 tried too much.

Branch state re-verified at start: `git fetch --all --prune` showed
`origin/claude/arcana-data-plane-production-final` unchanged from local HEAD
(`4db7c94`, batch 3's own status commit); no rebase/merge was needed.

## Task 1 — root-cause the 4 apps/admin/tests/cli.rs failures — **FIXED**

Commit `93a027b`.

Batch 3 only categorized these as "Windows Git-Bash subprocess/path
artifacts" without reproducing the real error text. Re-ran
`cargo test -p admin --test cli` (OPENSSL_CONF unset, per batch 3's
unrelated psqlODBC finding) and captured actual output for all 4:

1. **`user_create_json_output_has_no_server_secrets`**
2. **`user_create_json_output_carries_the_experimental_vision_off_link_additively`**
3. **`user_create_json_output_is_unaffected_by_suppression`**

   All three: `serde_json::from_slice` on stdout failed with `expected
   value, line: 1, column: 1`. **This is a real cross-platform bug, not a
   Windows harness artifact.** Root cause: `render_and_apply_singbox_config()`
   in `apps/admin/src/main.rs` printed progress/warning diagnostics via
   `println!` (stdout) on every degraded/offline-mutation path (missing
   REALITY keyset under `SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1`, missing
   sing-box binary, systemctl unavailable, etc.) — exactly the fixture these
   3 tests use (`write_deployment_toml()` points `singbox_binary` at a
   nonexistent path). That text lands on stdout ahead of the `--json`
   document, breaking the "stdout of `--json` is exactly one JSON document"
   contract. Reproduced by hand outside the test harness:
   ```
   $ SINGBOX_VPN_ALLOW_OFFLINE_MUTATION=1 vpn-admin.exe --config ... user create --name bob --json
   warning: skipping sing-box config render/apply: reality private key missing ...
   {
     "enabled": true,
     ...
   ```
   Would reproduce identically on Linux with the same fixture — nothing
   Windows-specific about it. **Fix (production code):** moved the 7
   diagnostic `println!` calls in `render_and_apply_singbox_config` to
   `eprintln!` (stderr). Updated
   `render_config_require_applied_fails_when_reconciliation_could_not_be_applied`
   to assert the "not found; wrote nothing" warning on stderr instead of
   stdout, since that test's baseline case exercises the same warning line.

4. **`repair_runs_the_located_update_script_with_repair_flag_and_propagates_success`**

   Genuinely Windows-only test-harness artifact. `cmd_repair()` shells out
   via `Command::new("bash").arg(&update_sh)`. On this Windows dev machine,
   `bash` on `PATH` resolves to the WSL `bash.exe` stub (not Git-Bash, which
   is what runs the test harness itself). That stub does not translate a
   Windows path argument (`C:\Users\...\fake-update.sh`) into a WSL path —
   it hands the raw backslash string to the WSL-side shell, which mangles it
   into `C:UsersArina...fake-update.sh` and fails with "No such file or
   directory" (exit 127) before the fake script ever runs. Confirmed this is
   a toolchain artifact, not production logic: production always runs this
   on Linux (AlmaLinux), where `bash` is real bash and `update_sh` is
   already a POSIX path, so the exact same code is correct there.
   `repair_propagates_a_nonzero_exit_from_update_sh` (same bash-invocation
   code path) happens to still pass on this machine, because it only
   asserts `.failure()` + a generic "repair failed" stderr substring — both
   of which also hold for the mangled-path failure — so it does not
   actually exercise the specific-args contract that fails here. **Fix
   (test only):** gated the test `#[cfg(unix)]` with a doc comment spelling
   out the exact mechanism (verified by manual repro), so it is not
   re-litigated; it runs unmodified in Linux CI and matches the AlmaLinux
   production target.

**Evidence**: `cargo test -p admin --test cli` (OPENSSL_CONF unset) — 54
tests compiled and run on this Windows machine (the `#[cfg(unix)]`-gated
test correctly excluded), **0 failed**.

## Task 2 — Phase 11: immutable sing-box hash pin — **FIXED**

Commit `5d97561`.

Cherry-picked `remediation/supply-singbox-pin`'s single commit (`a3e91a6`,
"fix(supply): enforce Arcana sing-box hash pin first") — applied with **no
context conflicts** against the current tree (`install.sh`/`update.sh` have
moved since that branch's base per batch 1's survey, but the touched
sections were unaffected). The branch's other commit, `fb8d36c` (a chmod
for `deploy/lib/tests/test-singbox-pinned-checksum.sh`), was **not**
cherry-picked — that file does not exist anywhere on
`remediation/supply-singbox-pin`; the commit is a stray chmod for a file
that was never added, nothing to carry forward.

**What changed**: `install_singbox()` (`deploy/almalinux/install.sh`),
`update.sh`'s sing-box update path, and CI's `singbox-validate` job
previously tried upstream's `checksums.txt` FIRST and only fell back to
Arcana's own pinned `SINGBOX_SHA256_AMD64`/`ARM64` when no `checksums.txt`
was published for that release. This is exactly the anti-pattern the audit
warned about: a release-asset compromise (or a legitimate-looking but
wrong/rebuilt asset) that controls both the tarball and its own
`checksums.txt` could pass verification without ever being checked against
Arcana's immutable pin. Now the Arcana pin is checked unconditionally
first (`die` on mismatch or on no pin for this version/arch); upstream
`checksums.txt`, when published, is consulted only afterward as additional
corroboration, never a substitute.

**Negative test (real, not grep-based)**: added to
`deploy/lib/tests/test-release-reproducibility.sh`. Extracts
`install_singbox()`'s real Arcana-pin + upstream-checksums.txt verification
block via `sed` (unmodified — same technique already used by
`test-update-release-fixture-verification.sh`), and drives it against a
fixture where the downloaded tarball's actual SHA256 **matches** a
`checksums.txt` fixture standing in for upstream ("upstream's checksum
passes") but **disagrees** with the Arcana pin. Asserts:
- rejection (`die "... does not match the Arcana pin"`), AND
- the upstream-checksums stub function was **never invoked at all** (via a
  marker file) — proving the pin check runs and fails *before* upstream is
  ever consulted, not merely that the end result happens to be rejection.

A second case proves a matching pin still proceeds to, and passes, the
upstream corroboration step (the ordering fix doesn't silently disable that
check).

**Evidence**: `bash deploy/lib/tests/test-release-reproducibility.sh` — all
checks pass, including both new functional cases.

**Open**: `shellcheck` is not installed on this dev machine (same tooling
gap as `cargo-audit`, flagged in batch 3) — not run against
`install.sh`/`update.sh`/the test script this batch. Needs a machine/CI
with it available.

## Task 3 — Phase 12: immutable bootstrap — **FIXED** (singbox-vpn side); **DOCUMENTED, not fixed** (vpn-web side)

Commit `ee70d3d`.

**The gap**: root `install.sh`'s documented one-liner —
`curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh | sudo bash`
— fetches the bootstrap script itself from `main`, a movable branch ref,
with **zero verification**. This is the actual root of trust for a fresh
install: everything install.sh goes on to do (resolve a release tag,
download the source archive, checksum-verify it against `SHA256SUMS`,
verify GitHub attestation) only starts working *after* this first
unverified fetch has already run as root. Tag protection (if configured)
protects release tags but does nothing for a `main`-branch fetch, which
never touches a tag.

**Fix**: `.github/workflows/release.yml`'s `publish` job now checks out the
exact commit tagged for the release, copies root `install.sh` into the
release's `dist/` assets, computes its own `sha256sum`, folds it into that
release's `SHA256SUMS` (the same manifest already covering the source
archive/SBOM/license inventory), and publishes `dist/install.sh` as a
release asset alongside it. `install.sh`'s header comment and `--help`
output now document the recommended production fetch pattern:

```
ver=v1.2.3
curl -fsSLO "https://github.com/David610/singbox-vpn/releases/download/$ver/install.sh"
curl -fsSLO "https://github.com/David610/singbox-vpn/releases/download/$ver/SHA256SUMS"
grep ' install\.sh$' SHA256SUMS | sha256sum -c -
sudo SINGBOX_VPN_VERSION="$ver" bash install.sh
```

This pins the bootstrap fetch to an immutable release tag **and** verifies
it against a checksum published in that same release — the same trust
model Phase 11 (this batch) applies to sing-box and the pre-existing model
for the source archive. The `main`-branch one-liner still works (no UX
regression for quick-start/dev use) but is now clearly labeled as the
weaker, unverified path, mirroring the existing `SINGBOX_VPN_CHANNEL=dev`
precedent in the same file.

**vpn-web** (`D:\David610\vpn-web`, checked read-only, not edited):
`functions/lib/node-bootstrap.js` (generated per-node bootstrap script,
`stage_install()`, lines ~253-259) has the same class of gap for
production fleet provisioning:
```js
curl -fsSL ... -o "$installer" \
  "https://raw.githubusercontent.com/$SINGBOX_VPN_REPO/$SINGBOX_VPN_VERSION/install.sh"
bash "$installer" --non-interactive ...
```
It already uses `$SINGBOX_VPN_VERSION` (a version tag) rather than `main`,
which is better than the default one-liner, but performs **no checksum
verification whatsoever** of the fetched `install.sh` before executing it
as root — and a tag alone is not sufficient per this task's own framing
(deletable/recreatable absent tag protection). Not fixed this batch: doing
it correctly needs one of two design decisions that shouldn't be made
unilaterally inside this batch's scope —
  (a) vpn-web's bootstrap fetches `SHA256SUMS` alongside `install.sh` from
      the same release (`.../releases/download/$SINGBOX_VPN_VERSION/...`,
      now possible given this batch's release.yml change) and greps/verifies
      before executing, mirroring the pattern just documented in root
      `install.sh`; or
  (b) the fleet enrollment/provisioning API response (whatever endpoint
      currently hands `singboxVpnVersion` to `node-bootstrap.js`, per
      `assertMatch("singboxVpnVersion", ...)` in that same file) also
      carries the expected `install.sh` checksum for that version, sourced
      server-side from the corresponding singbox-vpn release, removing the
      need for the node to trust GitHub's raw-content HTTPS endpoint at all
      for that one field.
  (a) is the smaller, more self-contained change and should be the default
  choice for whichever batch picks this up, unless vpn-web's own
  maintainers have a reason to prefer (b).

**Evidence**: new static checks in
`deploy/lib/tests/test-release-reproducibility.sh` (release.yml publishes
root `install.sh` folded into `SHA256SUMS`; `install.sh` documents the
pinned+verified fetch in both its header comment and `--help`) — pass.

## Validation

- `cargo fmt --all -- --check` — **clean**, no output.
- `cargo clippy --workspace --all-targets -- -D warnings` (OPENSSL_CONF
  unset) — **clean**, 0 warnings.
- `cargo test --workspace --no-fail-fast` (OPENSSL_CONF unset, same
  psqlODBC-noise workaround batch 3 used and documented — this env var is a
  stray pointer to an unrelated tool's OpenSSL config on this specific dev
  machine, not present in CI) — **647 tests passed, 0 failed** across 32
  test binaries (`exit=0`). Full log captured at
  `/tmp/fulltest.log` during this session (not committed — ephemeral).
- `shellcheck` — **not run**: not installed on this machine. Same gap as
  `cargo-audit` (batch 3). Flagged as open below.

## Carried-forward backlog (unchanged from batch 3, not attempted this batch — in scope for next batch)

- **A)** host nftables not wired into install/update.
- **B)** own-public-IP egress blocking incomplete.
- **C)** no live VPS C-16 validation.
- **D)** Phase 9 kill-timed-out-child unverified on Linux.
- **E)** `claim_token`/reclaim contract still doesn't exist.
- **F)** A3 (probe/C-16 regression test) and A4 (idempotent dedup) from
  batch 3 still not done — next batch's first work, along with
  `cargo-audit`/`shellcheck` tooling setup on whatever machine/CI runs
  next.
- **G) (new, this batch)** vpn-web's `node-bootstrap.js` bootstrap fetch
  still has no checksum verification — see Task 3 above for the two
  candidate fixes; needs a human/later-batch decision on which.

## Commits this batch

- `93a027b` — `fix(admin): route render/apply diagnostics to stderr, fix real --json stdout pollution`
- `5d97561` — `fix(supply): enforce Arcana sing-box hash pin before upstream checksums.txt (Phase 11)`
- `ee70d3d` — `fix(bootstrap): publish root install.sh as a checksummed release asset (Phase 12)`

All pushed to `origin/claude/arcana-data-plane-production-final`. PR not
opened, per instructions.
