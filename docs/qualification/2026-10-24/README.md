# Standalone production-readiness scorecard (2026-10-24)

Aggregate and address-free; host-specific detail stays private. Evidence classes follow `CLAUDE.md`.
Base: `main` `de89be39a3e0541328499e07fb5a0b204b32610d`.

**Verdict: NO_GO.** Mandatory gates are failed, blocked or unverified (below). Nothing here is device or staging evidence.

## State re-verified this session (GitHub, read-only)
- `main` = `de89be3`; CI (22/22 jobs, including `test`, `audit`, `clippy`, `fmt`, `shell`, `release-build`) and Security succeeded
  ([CI](https://github.com/David610/singbox-vpn/actions/runs/40015308680), [Security](https://github.com/David610/singbox-vpn/actions/runs/40015308732)).
- Tags: stable `v1.3.0`; latest published RC `v1.3.1-rc.2`; `v1.3.1-rc.3` not created. Open PRs: 0. Open issues: #165, #167, #168, #169.
- Not re-verified here: ruleset settings (last verified via issue #170 tooling), live node state.

## Registry reconciliation (this PR)
Three items were stale relative to `main`: I-87 (said "NOT merged to main"), I-99 and I-102 (said branch-only). PRs #192, #186, #190 are merged and their
unit tests run in the CI `test` job on `de89be3`. They are now `CI_VERIFIED` with unit-level scope only (generator batch `CI_PROMOTE_2`). I-100/I-98/I-101/I-103 are
vpn-web changes and stay out of this repository's standalone gate. I-86 stays UNVERIFIED: owner attribution of the historical logins does not establish host trust.

## Findings from reading current `main`
- **I-10 (restarts): confirmed as an upstream constraint, not a regression.** Every config-changing operation (`user create/disable/rotate-*`, `render-config` when the
  config changed, `restore`, REALITY/Hysteria2 secret rotation) goes through `reload_and_verify`, a full same-process restart (`apps/admin/src/main.rs`, comment at the
  restart site, citing SagerNet/sing-box#3731). Unchanged-config reconciles do not restart (`render_config_noop_reconcile_does_not_restart_singbox`,
  `render_config_repeated_timer_execution_is_idempotent`). Batching/coalescing is not implemented; for a 10-user scope this is a documented limitation, with a
  Hysteria2 client stall of up to ~30 s after a restart on the pinned 1.14.1 core. Not changed: no evidence yet that it justifies new code.
- **Provisioning-agent Retry-After minimum is process-local** and lost on restart. Impact is bounded (`MAX_RETRY_AFTER`; a still-limiting server answers 429 and re-arms
  the gate) and only applies while the agent runs, which it must not on standalone nodes. Recorded in I-87's remaining risk; no persistence code was added because it
  cannot be justified for the standalone scope and was not compilable/testable on this host.

## Scorecard
| Subsystem | Status | Basis | Blocks standalone release |
|---|---|---|---|
| Source CI / lint / unit / audit on `main` | PASS (CI-VERIFIED) | run 40015308680 on `de89be3` | no |
| Security workflow on `main` | PASS (CI-VERIFIED) | run 40015308732 | no |
| YouTube hairpin routing correctness | PASS (CODE-VERIFIED) | invariant tests + `hairpin_simulation.py` in CI | no |
| Shorts delivery via approved egress | BLOCKED | direct fin1 egress refused; relay's upstream path interferes with YouTube-named TLS/QUIC (2026-10-23 capture); no approved alternative | **yes** |
| Native signed-in/Incognito 8-cell matrix | BLOCKED | needs a physical iPhone + networks; runbook `docs/qualification/2026-10-22/RUNBOOK.md` | **yes** |
| Fresh install / upgrade / repair / rollback on exact artifact | UNVERIFIED | no RC.3 artifact, no disposable VPS authorised | **yes** |
| Certificate renewal, encrypted off-host backup/restore | UNVERIFIED | needs disposable host and owner-selected storage | **yes** |
| Host trust / SSH hardening (I-39, I-86) | FAIL / OPEN | root password login still on; owner key not yet installed | **yes** (for any acceptance on that host) |
| Soak (24 h), throughput, resource baselines | UNVERIFIED | no disposable infrastructure | yes (performance gate) |
| Monitoring alert delivery to an operator | UNVERIFIED | no operator destination configured | yes |
| Provisioning-agent isolation | PASS for standalone (CODE-VERIFIED) | agent stays disabled; Retry-After note above | no |
| Commercial integration (#165) | OUT OF SCOPE | tracked separately | no |

## Minimum owner actions to unblock
1. Install the owner's own SSH public key, prove a second-session key login with `deploy/lib/ssh-harden.sh verify`, then authorise disabling password login.
2. Authorise a disposable AlmaLinux 9 staging VPS (and an encrypted-backup destination) for the exact-RC lifecycle run; authorise creating `v1.3.1-rc.3` after merge.
3. Decide on a Google/YouTube egress path that YouTube accepts and that is not forbidden by INV-7/INV-8 (needs a network property: a residential-class or otherwise
   YouTube-accepted public IP with unfiltered YouTube TLS/QUIC, owner-approved provider/country). No traffic is moved without that approval.
4. Provide a physical iPhone session for the eight-cell matrix, or accept that those cells remain BLOCKED.
