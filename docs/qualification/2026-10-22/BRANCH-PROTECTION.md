# Enforcing CI and Security on `main` (issue #170)

**Status: BLOCKED on repository administration. Nothing here has been applied.**
The assistant's token reads rulesets but cannot write them, and gets `403
Resource not accessible by integration` on classic branch protection. The
checker and update body below are prepared and tested; an administrator
applies them.

## Evidence as of 2026-10-22 (read-only, `gh api`)

Ruleset `22482226` "Protect main" (active, no bypass actors, applies to
`~DEFAULT_BRANCH`) has exactly three rules: `deletion`, `non_fast_forward`,
`pull_request` with `required_approving_review_count: 0`. It has **no**
`required_status_checks` rule. Ruleset `22482307` "Protect release tags"
(`refs/tags/v*`: deletion, non_fast_forward, update; no bypass) must stay as is.

So today a pull request can be merged with red or missing checks, and zero
approvals are required. The snapshot is saved as
`deploy/lib/tests/fixtures/branch-protection/live-2026-10-22-protect-main.json`.

## Why requiring the three checks was not enough on its own

`offline-full-verdict` depends (`needs`) on 15 jobs. By default a job whose
dependency fails is **skipped**, and GitHub counts a skipped required check as
passing. A failing `test` or `shell` job would therefore have left the
required verdict "passing". The workflow now runs the verdict job with
`if: always()` and fails it unless every needed job succeeded
(`deploy/lib/ci-needs-verdict.py`, tested by
`deploy/lib/tests/test-ci-verdict-semantics.sh`). **Apply the ruleset change
only after that workflow change is on `main`**, or the new requirement would
still be bypassable by a skipped verdict. (Skipped-counts-as-success is
GitHub's documented behaviour; it has not been re-demonstrated in this
repository, which is what step 4 below does.)

## Administrator steps

1. Read the current ruleset (needs only read access):
   `gh api repos/David610/singbox-vpn/rulesets/22482226 > ruleset.json`
2. Build the update body. It keeps every existing rule and condition and adds
   `required_status_checks` (`offline-full-verdict`, `codeql-rust`,
   `codeql-actions`, strict = checks must run on the current commit):
   `python3 deploy/lib/verify-main-protection.py ruleset.json --emit-update-payload > update.json`
   Review `update.json` (the diff against `ruleset.json` must be only the added rule).
3. Apply it (needs repository administration):
   `gh api -X PUT repos/David610/singbox-vpn/rulesets/22482226 --input update.json`
   or Settings -> Rules -> Rulesets -> "Protect main" -> add "Require status
   checks to pass" with the same three contexts and "Require branches to be up
   to date". Optionally set each check's source to GitHub Actions.
4. Verify, machine-checkably:
   `gh api repos/David610/singbox-vpn/rulesets/22482226 > after.json`
   `python3 deploy/lib/verify-main-protection.py after.json` must print
   `main protection: PASS` and exit 0. Also confirm ruleset `22482307` is
   unchanged and that Settings -> Branches shows no classic rule that
   contradicts it (classic protection cannot be read by the assistant).
5. Prove a failing check blocks a merge (live, once):
   open a throwaway PR that makes one CI job fail (for example, a deliberately
   failing assertion in a test the `test` job runs). Expected:
   `gh pr view N --json mergeStateStatus,statusCheckRollup` reports
   `mergeStateStatus: BLOCKED`, `offline-full-verdict` concluded `failure`
   (not `skipped`), and "Merge" is disabled. Close the PR without merging.
   A result of `skipped` for the verdict, or a mergeable PR, means the
   workflow change is not in effect: do not close #170.

## What stays out of scope

This change does not add required reviews, change merge methods, or touch the
tag ruleset. Raising `required_approving_review_count` is a separate owner
decision; the current value is 0.
