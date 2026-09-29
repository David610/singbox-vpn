# Addendum 2 — two_hop_system + CodeQL triage (2026-09-29)

Continuation of `ARCANA_BATCH6_STATUS_2026-09-29.md`, at `16f9ef8`.
Scope: exactly the two remaining PR-green blockers batch 6 left open.
Batch 7 (nftables/own-IP egress) explicitly NOT started here.

## Task 1 — two_hop_system topology redesign: still blocked, same wall as batch 6

Re-verified batch 6's root cause directly against `C16_DENY_IPV4_CIDRS`
(`crates/compat-config/src/model.rs:79-94`): the deny list covers
`0.0.0.0/8, 10/8, 100.64/10 (CGNAT), 127/8, 169.254/16, 172.16/12,
192.0.0.0/24, 192.0.2.0/24 (TEST-NET-1), 192.168/16, 198.18/15,
198.51.100.0/24 (TEST-NET-2), 203.0.113.0/24 (TEST-NET-3), 224/4, 240/4`.
This is every IANA-documented "safe for testing" special-use range plus
all private/link-local/CGNAT space. There is no address block left that
is both (a) not in this list and (b) not real, globally-routable
Internet space — confirming batch 6's own conclusion, not a new finding.

Attempted this batch's prescribed next step (dummy interface + `ip addr
add`, no default route, addresses outside the deny list): requires
`CAP_NET_ADMIN`. Checked directly in the available WSL Ubuntu-24.04
environment:

```
$ sudo -n true
sudo: a password is required
```

No passwordless sudo is available in this sandbox, and this is a
non-interactive session (no TTY to supply one) — identical blocker to
the one batch 6 already documented and exhausted (its `unshare --user
--net --map-root-user` attempt also failed, for the unrelated reason
that a fully isolated netns has no interface for sing-box to bind at
all). No new privilege-escalation path was found this pass. Per the
task's own exception list ("genuinely unavailable external
infrastructure"), this is left unresolved and undocumented as fixed.

**What a human needs to do**: get a runner/shell with real
`CAP_NET_ADMIN` (a GitHub Actions `ubuntu-latest` runner has this by
default via `sudo`, so this is very likely resolvable there even though
it isn't resolvable in this sandbox) and implement the dummy-interface
prototype batch 6 sketched: `sudo ip link add dummy-test type dummy &&
sudo ip addr add <addr>/32 dev dummy-test` for `TARGET_IP`/`UNDECLARED_IP`
replacements, wrapped in a Drop guard or trap-based teardown, wired into
`crates/compat-config/tests/two_hop_system.rs`'s `prerequisites()` the
same way the existing sing-box/openssl check is gated. No code was
changed this pass (nothing to safely commit without being able to prove
it passes).

Pass count: unchanged from batch 6 — still 17/22 failing under C-16 as
designed, 5/22 passing (the ones not touching `TARGET_IP`/`UNDECLARED_IP`
loopback addresses). Not re-run this pass since no topology change was
made; re-running would reproduce batch 6's numbers exactly.

## Task 2 — CodeQL aggregate check: classified (C), non-blocking, no fix needed

`gh pr checks 123` current state distinguishes three separate CodeQL-named
checks:

| name | status | conclusion |
|---|---|---|
| `codeql-rust` | IN_PROGRESS | — |
| `codeql-actions` | COMPLETED | SUCCESS |
| `CodeQL` | COMPLETED | **NEUTRAL** |

`CodeQL` (capital, no hyphen) is GitHub's own default auto-registered
code-scanning check — distinct from this repo's custom `security.yml`
jobs `codeql-rust`/`codeql-actions`. Its conclusion is `NEUTRAL`
("skipping" in `gh pr checks`' short form), not `FAILURE`: GitHub
registers this default check automatically but it self-skips because
the repo's actual CodeQL analysis runs through the custom-named
workflow instead of GitHub's default setup. A `NEUTRAL` conclusion does
not fail the PR's aggregate rollup.

Cross-referenced `gh api .../code-scanning/alerts --paginate`: every
alert's `most_recent_instance.ref` is `refs/heads/main` — none carry a
`refs/pull/123/*` ref. No alert's evidence ties it to this branch's own
commits; all are pre-existing on `main` (oldest `2026-08-23`, newest
`2026-09-24`, all before this branch's remediation commits). This rules
out classification (A) — nothing was newly introduced by this
remediation.

Checked branch protection for a possible stale/mismatched required
check name: `gh api repos/David610/singbox-vpn/branches/main/protection`
returned `404 Branch not protected`. There is no branch protection rule
on `main` at all, so no required-check-name mismatch is possible
(classification C's "required check" variant) — there's simply no
required check configured, meaning even a genuine `CodeQL` failure
could not block the merge button today.

**Conclusion: classification (C)** — the `CodeQL` check is GitHub's own
auto-registered default scan, distinct from and redundant with this
repo's real `codeql-rust`/`codeql-actions` jobs, concluding `NEUTRAL`
(not a failure) with no branch-protection rule making it required
either way. No code or workflow change made — there is nothing to fix;
a prior pass's read of this as "failing" was most likely catching it
mid-run (`IN_PROGRESS`/`PENDING`) before it settled to `NEUTRAL`. If the
duplicate/default `CodeQL` check is undesired noise long-term, a human
with repo admin access can disable GitHub's default code-scanning setup
under Settings → Code security (out of scope to change here — no
functional bug to fix, no branch protection depending on it).

## Validation and commit

No source or workflow files were changed this pass (both items landed
on documented, evidenced blockers rather than code fixes). Only this
addendum doc was added, so `cargo fmt`/`clippy`/`test` were not
re-run — nothing in their inputs changed since batch 6's own clean
run recorded in `ARCANA_BATCH6_STATUS_2026-09-29.md`.

## Carried-forward for whoever picks this up next

- two_hop_system: needs a real `CAP_NET_ADMIN`-capable shell (a GitHub
  Actions runner, or a WSL/host account with real passwordless sudo) to
  implement and validate the dummy-interface addressing fix sketched
  in batch 6 and re-confirmed here.
- CodeQL: no action required; documented as non-blocking noise from
  GitHub's default scan setup coexisting with this repo's custom-named
  jobs.
