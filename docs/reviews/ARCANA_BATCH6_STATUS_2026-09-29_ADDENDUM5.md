# Arcana Batch 6 status — ADDENDUM5 (2026-09-29)

## Context

Commit `9e728ce` correctly diagnosed and fixed the L5-6 INCONCLUSIVE
failure by adding `apply_exit_loopback_selftest_exception` as a new
leading `route.rules` entry (scoped to the reserved-probe identity) on
relays that have a reserved-probe user, inserted ahead of
`apply_probe_user_confinement`'s 3-rule block. That fix was confirmed
working in real CI (L5-6 passed). But it shifted the relay's leading
rule shape from `[probe x3, ...]` to `[loopback x1, probe x3, ...]`,
and two places still assumed the old shape, both failing in real CI.

## Correct rule composition

On a relay with an active reserved-probe user, `route.rules` is:
`[loopback-exception x1] + [probe-confinement x3] + [hairpin x2 per
hairpin user] + [relay-target x1 per declared exit] + [final reject x1]`.
The loopback exception and the 3 confinement rules are gated on the
exact same condition (`is_reserved_probe` user exists) and always
travel together as a 4-rule leading block —
`compat_config::server::probe_confinement_rule_count` already computes
this correctly (0, 3, or 4) via `probe_confinement_rule_count`'s own
leading-loopback-rule detection.

## What was fixed

1. **`apps/admin/src/main.rs`, `report_relay_policy`'s `Relay` branch**:
   the fail-closed arithmetic previously counted at most `+1` for the
   loopback rule (a local `auth_user`+`ip_cidr` scan) and never
   accounted for the 3 confinement rules at all — so a correctly
   fail-closed, probe-provisioned relay failed doctor's own L2 check
   (`relay forwarding policy is NOT fail-closed`). Fixed to reuse
   `probe_rules` (`probe_confinement_rule_count(doc)`, already computed
   once above the role match and shared with the `Exit` branch), which
   reports the correct 0/3/4 count.

2. **`apps/admin/tests/cli.rs`**:
   `apply_revision_preserves_reserved_probe_confinement_and_c16_policy_across_reapply`
   hardcoded the pre-fix 3-rule expectation for an exit's leading probe
   block. Traced `probe_confinement_rule_count` and confirmed the new
   loopback-exception rule is genuinely part of what "confinement"
   means for this exit+probe-user scenario (both rules exist only
   because a reserved-probe user exists, and are inserted as one
   contiguous leading block) — not a bug. Updated the expected count to
   4 and shifted the ordering assertions (`rules[0..4]` = loopback +
   probe, `rules[4..8]` = C-16) accordingly.

3. `.github/workflows/ci.yml`'s `len(r)==5` assertion for the unpaired
   relay self-test scenario (0 relay targets + 1 final reject + 4
   leading probe/loopback rules = 5) was already correct and needed no
   change — verified independently against the traced composition
   rather than assumed.

## Verification

- `cargo fmt --all -- --check`, `cargo clippy --locked --workspace
  --all-targets -- -D warnings`, `cargo test --locked -p compat-config`
  (62 passed), and `cargo test --locked -p admin --test cli` (53
  passed; the fixed test itself is `#[cfg(unix)]` and only compiles on
  Linux, so it ran in real CI, not locally on Windows) — all green.
- Commit: `f7b50eb`, pushed to `claude/arcana-data-plane-production-final`.
- Real CI, PR #123, both runs (main run 36571983064 and a duplicate
  36571991970):
  - `test`: **pass** (2m53s / 2m23s)
  - `singbox-validate`: **pass** (2m4s / 2m3s)
  - All other jobs (fmt, clippy, audit, docs, os-matrix x6 x2,
    release-container-smoke, shell, license-check,
    no-legacy-identity-check, secret-logging-check,
    workspace-version-consistency-check, codeql-actions, codeql-rust):
    **pass**.
  - One check, a separate legacy `CodeQL` check-run (distinct from
    `codeql-actions`/`codeql-rust`, which both pass), shows `fail`.
    Confirmed via `gh api .../commits/<sha>/check-runs` that this same
    check already failed identically on `9e728ce` (the commit before
    this fix) — pre-existing and unrelated to this change, not
    something this fix introduced or could have caused.

`gh pr checks 123` after the fix: every check passes except that one
pre-existing, unrelated `CodeQL` check-run. Both `test` and
`singbox-validate` — the two jobs this task was specifically scoped to
— are genuinely green on both runs. PR #123 was not merged.
