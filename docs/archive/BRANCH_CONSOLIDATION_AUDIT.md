> **HISTORICAL DOCUMENT — NOT CURRENT PRODUCT DOCUMENTATION.**
> This is a point-in-time audit/report snapshot, preserved for engineering
> history. It may describe code, findings, or product state that has since
> changed or been superseded. For the current product boundary and status,
> see `docs/SUPPORTED_PRODUCT.md`, `docs/IMPLEMENTATION_STATUS.md`, and
> `docs/DEVICE_ACCEPTANCE_TESTS.md`.

# Branch Consolidation Audit

## Baseline

- Date: 2026-08-25
- `main` SHA: `ba2490586dea727e35f79c0561ba48b0b20a6f5c` ("deps: bump ed25519-dalek from 2.2.0 to 3.0.0 (#31)")
- CI on that exact commit (workflow run 32771659820, `ci.yml`): **success**
- Repository default branch: `main`
- Total remote branches at audit time: 3 (`main`, `claude/current-main-release-gi5zaq`, `claude/review-open-prs-eu9byf`)
  - All Dependabot/codex/claude feature branches from historical PRs (#1–#40) had already been
    auto-deleted by GitHub on merge/close; nothing stale remained for those.
- Open pull requests: **0** (verified via `list_pull_requests` with `state: open`)
- Tags present: `v0.1.0-rc.1`, `v0.1.0-rc.2`, `v0.1.0-rc.3`, `v0.1.0`, `v0.1.1-rc.1`, `v0.1.1`, `v0.1.2`
  - Note (operational, not a branch-audit finding): PR #30 bumped every package to `0.1.3` and
    merged into `main`, but no `v0.1.3` tag has been pushed yet. Cutting that tag is a release
    action with real side effects (triggers `release.yml`), so it is out of scope for this audit
    and is left for the repository owner to do deliberately.

## Branch inventory

| Branch | Tip | Ahead of main | Behind main | PR | Semantic unique work | Classification |
|---|---|---|---|---|---|---|
| `main` | `ba24905` | — | — | — | — | n/a (baseline) |
| `claude/review-open-prs-eu9byf` | `ba24905` | 0 | 0 | none open | none (byte-identical to main) | I. Exact duplicate of main |
| `claude/current-main-release-gi5zaq` | `761dcaa` | 2 (`85327bc`, `761dcaa`) | 11 | #30 (**merged**) | none (fully incorporated into main by other commits) | A. Merged feature/fix branch, superseded by later history |

## Detailed branch analysis

### `claude/review-open-prs-eu9byf`

- **Purpose**: per its name, a working branch from a prior "review open PRs" session.
- **History**: tip SHA is `ba2490586dea727e35f79c0561ba48b0b20a6f5c`, which is byte-for-byte
  identical to `origin/main`'s current tip. `git rev-parse origin/main` and
  `git rev-parse origin/claude/review-open-prs-eu9byf` return the same SHA.
- **Unique commits**: none. `git log main..origin/claude/review-open-prs-eu9byf` is empty, and
  `git diff --stat main...origin/claude/review-open-prs-eu9byf` is empty.
- **Patch-equivalence result**: trivially exact — same commit object, not just same tree.
- **Unique files/changes**: none.
- **Value assessment**: no unique value; nothing to lose by deleting.
- **Security/reliability assessment**: n/a — no content differs from `main`.
- **Relationship to current architecture**: n/a.
- **Decision**: **SAFE TO DELETE — EXACT DUPLICATE OF MAIN.**

### `claude/current-main-release-gi5zaq`

- **Purpose**: head branch of PR #30, "Bump version to 0.1.3 to release current main" — a
  workspace-wide version bump (`0.1.2` → `0.1.3`) across all `apps/`, `crates/`, `services/`,
  `tests/` `Cargo.toml` files, `Cargo.lock`, and a fix to
  `deploy/lib/tests/test-release-version-contract.sh` so the release-version-contract test expects
  `v0.1.3` instead of `v0.1.2`.
- **History**: branched from `08fa9b0` (an ancestor of current `main`). PR #30 was merged into
  `main` on 2026-08-24 (`merged: true`, confirmed via `pull_request_read`). `main` has since moved
  11 commits ahead via further merges (PR #40) and 8 Dependabot dependency/CI-action bumps
  (PRs #31–#39).
- **Unique commits**: `git log main..origin/claude/current-main-release-gi5zaq` shows 2 commits
  (`85327bc` "Bump version to 0.1.3 to release current main", `761dcaa` "Update
  release-version-contract test for the 0.1.3 bump").
- **Patch-equivalence result**: `git cherry -v main origin/claude/current-main-release-gi5zaq`
  reports both as `+` (no exact patch-id match in `main`'s history) — expected, since PR #30 was
  merged via a merge commit (`0020172`) rather than fast-forwarded, and `main` has had 8 dependency
  bumps since, which touch overlapping files (`Cargo.lock`, package `Cargo.toml`s) and change the
  surrounding diff context. Patch-id alone is therefore not conclusive here, so the actual file
  content was compared directly.
- **Unique files/changes** (`git diff main...<branch>` on content, not patch-id):
  - All 21 workspace-member `Cargo.toml` `version = "..."` fields: **identical** between the
    branch and current `main` — both are `0.1.3` (`git diff main <branch> -- apps/cli/Cargo.toml`
    is empty; spot-checked and confirmed for the full set via `grep -m1 version` across
    `apps/*/Cargo.toml crates/*/Cargo.toml services/*/Cargo.toml`, all reading `0.1.3`).
  - `deploy/lib/tests/test-release-version-contract.sh`: **identical** — the `git diff` for this
    file between `main` and the branch is empty; the v0.1.3 contract-test fix is already present
    in `main`.
  - `Cargo.lock`: differs, but only because `main` has since applied 5 further Dependabot version
    bumps (`ed25519-dalek` 2→3, `thiserror` 1→2, `sha2` 0.10→0.11, `toml` 0.8→1.1, `criterion`
    0.5→0.8) that the branch predates. This is `main` being *ahead*, not the branch carrying
    anything `main` lacks.
- **Value assessment**: every semantic change originally introduced on this branch (the version
  bump and the matching contract-test fix) is present in current `main`, just reached there via a
  different commit path (merge commit, not the exact same diff hunks byte-for-byte, because later
  commits touched adjacent lines). There is no lost work.
- **Security/reliability assessment**: n/a — nothing to reintroduce; `main` already carries the
  intended state and is newer.
- **Relationship to current architecture**: pure version/release-process housekeeping, no
  connection to transport/protocol architecture.
- **Special check requirements** (Section 7 of the audit brief) — verified:
  1. Every semantic patch on the branch exists in current `main`. ✅ (shown above)
  2. Version 0.1.3 changes are present where expected. ✅ (all 21 package `Cargo.toml`s read
     `0.1.3` in `main`)
  3. The release-version-contract test change is present. ✅ (file is byte-identical to `main`)
  4. No later branch-only commit exists. ✅ (`git log main..branch` shows only the 2 known
     commits, both accounted for)
  5. No tag or workflow relies on the branch name. ✅ (`rg` over `.github/workflows/`, `docs/`,
     `deploy/`, `README.md` finds zero references to `claude/current-main-release-gi5zaq`)
  6. No open PR uses it. ✅ (PR #30 is closed/merged; `list_pull_requests(state=open)` returns
     zero results)
  7. Deleting the branch does not break any documented release process. ✅ (the release process
     is driven by tags per `deploy/lib/check-release-version.sh` and `.github/workflows/release.yml`,
     not by this branch name)
- **Decision**: **SAFE TO DELETE.**

## Valuable work recovered

None. No branch (other than `main` itself) contained any semantic change absent from `main`.

## Intentionally discarded work

None discarded — there was nothing on either branch that wasn't already incorporated into `main`.

## Safe deletion list

- `claude/review-open-prs-eu9byf` — SAFE TO DELETE — EXACT DUPLICATE OF MAIN
  - BRANCH: `claude/review-open-prs-eu9byf`
  - TIP SHA: `ba2490586dea727e35f79c0561ba48b0b20a6f5c`
  - PR: none
  - UNIQUE COMMITS: none
  - PATCH-EQUIVALENT: identical commit SHA to `main`
  - USEFUL WORK: none
  - WORK PRESERVED WHERE: n/a (nothing unique to preserve)
  - WORKFLOW REFERENCES: none found
  - TEST STATUS: n/a (identical to `main`, which is CI-green)
  - DELETION VERDICT: safe

- `claude/current-main-release-gi5zaq` — SAFE TO DELETE
  - BRANCH: `claude/current-main-release-gi5zaq`
  - TIP SHA: `761dcaa15cdb6f7e6cbddaeb375e2b6425f1d542`
  - PR: #30 (merged 2026-08-24)
  - UNIQUE COMMITS: `85327bc`, `761dcaa`
  - PATCH-EQUIVALENT: not by patch-id (merge commit + later overlapping Dependabot bumps changed
    context), but confirmed content-identical by direct file diff for every file the branch
    touched except `Cargo.lock`, whose only differences are `main` having *later* independent
    dependency bumps
  - USEFUL WORK: none beyond what's already in `main`
  - WORK PRESERVED WHERE: `main` (via PR #30 merge commit `0020172` and subsequent history)
  - WORKFLOW REFERENCES: none found
  - TEST STATUS: CI green on `main` at current tip
  - DELETION VERDICT: safe

## Branches that must remain

- `main` — the only branch left after this audit. It contains every useful production-worthy
  change found across the repository's branch history and is CI-green at its current tip.

## Test evidence

- GitHub Actions `CI` workflow run `32771659820` on `main` @ `ba24905`: **success** (fmt, build,
  test, clippy, and other configured jobs all passed as gated by `.github/workflows/ci.yml`).
- No code changes were made to `main` by this audit (both candidate branches were fully
  superseded/duplicate, so nothing needed integration), so no additional local build/test run was
  required beyond confirming the existing green CI evidence on the current tip.

## Final recommendation

Delete both `claude/review-open-prs-eu9byf` and `claude/current-main-release-gi5zaq`. Neither
carries any change that isn't already present in `main`, no open PR or workflow depends on either
branch name, and `main` alone is a complete, understandable, CI-green branch structure for this
repository's current scope (a single-VPS VLESS+REALITY/Hysteria2 deployment for ~10 users).
