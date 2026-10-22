# Production qualification — 2026-10-22

Decision: **NO-GO**. This is a dated qualification record, not a release or
authorization to change a host. The current issue source remains
[`issue-registry.json`](../../handoffs/2026-10-20/issue-registry.json), generated
by `deploy/lib/registry/build_issue_registry.py`; historical server states in
that registry do not qualify the rebuilt host.

Baseline: main `6825fcf491fd44e80543edc0b076c5b114cade16`, tree
`3ddd509b39f57579b5832d58ec25d17690c1d637`. The local repository was restored
through GitHub's connector: all 793 blob hashes, the complete tree and the
signed commit object matched GitHub. History is shallow at that commit.
GitHub comparisons supplied PR ancestry. No production connection was made.

| Status | Result | Evidence |
|---|---|---|
| Baseline code qualified | PASS | [CI: 22/22](https://github.com/David610/singbox-vpn/actions/runs/39818433277), [Security: 2/2](https://github.com/David610/singbox-vpn/actions/runs/39818433134), both exact baseline SHA |
| Integration code qualified | PASS for `ca4e34fa5bbd340490b678d7146c20e3f0516fab` | [CI: 22/22](https://github.com/David610/singbox-vpn/actions/runs/39828809922), [Security: 2/2](https://github.com/David610/singbox-vpn/actions/runs/39828809935); later commits and main merge require independent exact-commit checks |
| Server qualified | BLOCKED | No host access; unexplained successful SSH logins and installation prerequisites unresolved |
| Device verified | BLOCKED | No physical device or exact release artifact available |
| Standalone VPN production ready | BLOCKED | Server, release artifact and device gates absent; SSH trust unresolved |
| Full Arcana commercial platform production ready | BLOCKED | Paired control-plane changes, deployment/migration, accounts/billing/Telegram end-to-end evidence absent |

Deliverables: [PR integration](PR-INTEGRATION.md), [Shorts diagnosis](SHORTS.md),
[release qualification](RELEASE.md), [security and operations](SECURITY-OPERATIONS.md),
[release/rollback and device procedure](RUNBOOK.md). Machine-readable status:
[`status.json`](status.json). These record limitations explicitly; no phase is
considered complete because its report exists.

Local results and refreshed GitHub metadata are retained separately under
`/workspace/audit-evidence`; private host evidence must stay outside Git.
I-90 was found after the first integration passed CI/Security. The corrected
source `ca4e34fa5bbd340490b678d7146c20e3f0516fab` subsequently passed both complete workflows.
Those results include the nonzero-health cleanup regression. They do not
qualify later documentation commits or the actual main merge; each requires
its own complete checks.

Next owner action: confirm whether the successful password logins in A3 of the
[pre-install checklist](../../handoffs/2026-10-22/PRE-INSTALL-CHECKLIST.md)
were yours. If any is unknown, investigate via the provider console and
establish a trusted rebuild before authorizing installation. Separately,
provide a disposable staging host/device and the owner-key, recovery-console,
snapshot and independent backup prerequisites. No private key or password is
needed in chat. The repository-consolidation session authorizes qualified PR merges after
verification. Release publication, credentials and installation remain outside
that authorization.
