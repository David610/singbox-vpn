# Branch Survey — 2026-09-29

Prepared as Step 1 of the Arcana data-plane production remediation
(batch 1 of ~5, working branch `claude/arcana-data-plane-production-final`).
Scope: survey every remote branch that could plausibly bear on this
batch's phases (egress isolation, own-node pivot protection, reserved
probe principal) or on the ~19 remaining phases from later batches. This
document records what each branch changes, how stale it is, and whether
it is safely cherry-pickable — **it does not implement anything itself.**

Base: `main` @ `5cee2fa` (`927d9e6` on `origin/main` after fetch — 2
docs-only commits ahead, not pulled into this working branch to keep the
diff reviewable; see "Not pulled" note below).

## Method

For every branch: `git rev-list --count main..<branch>` (divergence) and
`git diff main..<branch> --stat` (shape of the change). Branches with 0
commits ahead of `main` are already fully merged or were rebased away —
excluded from the tables below except where explicitly named in the
task. The three branches named in the task instructions
(`remediation/node-agent-job-safety`, `remediation/node-dp-egress-isolation`,
`remediation/supply-singbox-pin`) got a full `git diff` read, not just a
stat.

## This batch's scope (phases 1–4: reconciliation, egress isolation,
own-node pivot, probe principal)

| Branch | Ahead | What it changes | Staleness | Verdict |
|---|---|---|---|---|
| `remediation/node-dp-egress-isolation` | **0** | Nothing — `git merge-base main <branch>` equals `main`'s current HEAD (`5cee2fa`). Despite the name, this branch contains **no egress-isolation work at all**; either the branch was created and never committed to, or an earlier attempt reset it. | N/A | Nothing to reconcile or cherry-pick. This batch's C-16 egress-isolation work (Phase 2 below) is implemented from scratch on the working branch. |
| `remediation/node-agent-job-safety` | 1 | One commit touching `apps/provisioning-agent/src/config.rs`: changes `default_lease_pool_size()` from `32` to `0` (fail-safe — an unmanaged/self-host node must not synthesize lease users and restart `sing-box` periodically just because the setting was omitted), plus a matching unit test. | Diverges cleanly from current `main` (clean diff, no conflicts expected). | **Cherry-pickable as-is**, but it is a job-safety/restart-storm fix, not egress/pivot-relevant — out of this batch's scope per the task's own scoping note. Recommended for the job-safety batch: `git cherry-pick` the single commit, re-run `cargo test -p provisioning-agent`. |
| `remediation/supply-singbox-pin` | 1 | One commit touching `.github/workflows/ci.yml`, `deploy/almalinux/install.sh`, `deploy/almalinux/update.sh`, `deploy/lib/tests/test-release-reproducibility.sh` — sing-box binary pin/checksum verification hardening. | Diverges cleanly. | **Cherry-pickable as-is**, but supply-chain/pin work, not egress/pivot — out of this batch's scope. Recommended for the supply-chain batch; the diff is small (4 files, ~45/38 lines) and should be reviewed line-by-line against current `install.sh`/`update.sh` (both files have moved on since this branch's base and a mechanical cherry-pick may need minor context fixes) rather than merged blindly. |

**Net for this batch:** neither of the two named remediation branches
that this batch was told to reconcile turned out to have anything
egress/pivot-relevant to port. Phase 2/3/4 below are original
implementation on top of current `main`, not ports.

## Other branches, surveyed for later batches

Every branch below has **0 commits ahead of `main`** (`git rev-list
--count main..<branch>` = 0) unless noted — meaning either already
merged into `main`'s history, or squash-merged/rebased away and now a
strict subset. These are effectively inert; no action needed, listed
for completeness since the task asked for a survey of anything
touching the listed areas:

`arcana/provisioning-agent`, `archive/native-adaptive-stack-2026`,
`chatgpt/product-ops-agent-v2`, `chore/sing-box-1.14.1`,
`claude/singbox-rc-evaluation-i0kpp0`, `claude/traffic-accounting`,
`claude/vpn1-singbox-migration-bbcv6y`,
`claude/youtube-direct-device-verify`, `feat/b2-ephemeral-auth`,
`feat/protocol-health`, `fix/acceptance-certbot-random-sleep`,
`fix/lifecycle-realvps-findings`,
`fix/public-bootstrap-verification-token`,
`fix/stage5-reboot-health-diagnostics`,
`fix/update-normalize-source-perms`, `fix/vps-acceptance-ssh-agent`,
`perf/burst-provisioning`, `perf/subscription-hundreds`,
`release/v1.1.0-acceptance`, `release/v1.1.0-rc.1-version-bump`.

### Branches with real divergence (candidates for later-batch review)

| Branch | Ahead | Shape | Staleness | Verdict |
|---|---|---|---|---|
| `chatgpt/full-plan-completion` | 7 | `--stat` shows **7167 deletions vs 444 insertions** across 68 files, including wholesale removal of `services/subscription/src/lib.rs` content and a design doc. | Diverged from a base that predates significant current-`main` structure; the large deletion count is the signature of a stale branch whose base lacks files/refactors `main` now has, not of an intentional revert. | **Do NOT merge wholesale — task instruction confirmed correct.** If anything here is useful it must be identified commit-by-commit (`git log chatgpt/full-plan-completion --oneline` gives 7 commits to individually inspect) in a later batch; this survey did not find a specific extractable commit worth flagging given the wholesale-deletion shape of the diff. |
| `chatgpt/phase2-access-paths` | 17 | 23619 deletions / 521 insertions across 126 files — same stale-base signature, largest of the phase2-* branches. | Very stale. | Reimplementation-only if anything is wanted; do not merge or cherry-pick without individual commit review. |
| `chatgpt/phase2-reachable-first-hop-design` | 13 | 24267 deletions / 545 insertions across 128 files — same pattern. | Very stale. | Same as above. |
| `chatgpt/phase2-local-complete-docs` | 2 | Not stat'd in detail (docs-scoped per name); low commit count, likely low risk to review in a later batch if the phase2 design docs are wanted. | Unknown, low priority. | Low priority for a docs-only later pass. |
| `chatgpt/product-ops-agent` | 6 | Product-ops agent scope, unrelated to node data plane. | Unreviewed in depth (out of this batch's phase range). | Defer to whichever later batch owns product-ops/control-plane agent work. |
| `feat/relay-hardening-live-vps` | 16 | 23104 deletions / 1515 insertions across 128 files — same stale-base deletion signature as the chatgpt/phase2-* branches, but the name is directly relevant to relay hardening (this batch's area). | Very stale — despite the name, this predates the current relay-role/route-rule architecture in `compat-config::server`, so a diff-based cherry-pick is not viable. | **Worth a manual read in a later batch** (or now, budget permitting) via `git log feat/relay-hardening-live-vps --oneline` to see if any individual commit message describes a relay hardening idea not yet implemented — but expect reimplementation, not merge, given the diff shape. |
| `experiment/server-side-quic-reject` | 3 | 13744 deletions / 687 insertions across 98 files — same stale pattern; also, current `main` already has `crates/compat-config/tests/quic_reject_config.rs` and QUIC-reject support (`subscription_url_quic_reject`), so this experimental branch is very likely superseded already. | Stale, likely superseded. | Skip; current `main` appears to already contain the shipped version of what this branch experimented with. Confirm in a later batch before deleting. |
| `deps/singbox-1.14.1` | 3 | sing-box version bump — `main`'s `deploy/lib/versions.env` already pins `1.14.1`, so this is very likely already-merged/superseded. | Superseded. | No action; confirm and consider deleting in cleanup. |
| `docs/fleet-phase8-design` | 3 | Design docs for fleet phase 8 (health/failover) — unrelated to this batch. | Unreviewed. | Defer to fleet/health batch. |
| `feat/fleet-phase8-health` | 4 | Fleet phase 8 health implementation — unrelated to this batch. | Unreviewed. | Defer to fleet/health batch. |
| `feat/minimal-vpn-business-web` | 12 | Business/web-facing scope, unrelated to node data plane. | Unreviewed. | Defer; not a node-dp concern. |
| `feat/platform-v2` | 1 | Single commit, unreviewed. | Unreviewed. | Low priority, single commit — cheap to review in a later batch. |
| `feat/privacy-plus-dev-gate` | 1 | Single commit, unreviewed. | Unreviewed. | Same as above. |
| `fix/real-vps-acceptance-blockers` | 14 | VPS acceptance test/CI fixes. | Unreviewed, moderately stale (14 commits). | Defer to whichever batch owns VPS acceptance CI. |
| `fix/vpn-admin-sudo-path-and-certbot-timeout` | 1 | Single commit — sudo path / certbot timeout fix in `vpn-admin`. | Unreviewed but low-risk given size. | Cheap to review/cherry-pick in a later batch (operational fix, not security-critical). |
| `fix/youtube-relay-udp` | 4 | YouTube/relay UDP handling — `main` already has `relay_role_policy.rs` UDP-relay-refusal tests (`udp_relay_capability_is_refused`) and `hiddify_relay_udp.rs`; likely superseded. | Possibly superseded. | Confirm before reviving. |
| `claude/fix-provisioning-url`, `claude/singbox-vpn-audit-v6nqyz`, `claude/singbox-vpn-release-hardening-bryfv9`, `claude/singbox-youtube-failure-hx1y59` | 1 each | Single-commit, topically narrow (provisioning URL fix, an earlier audit doc, release hardening, a YouTube failure fix). | Unreviewed. | Cheap (1 commit each) to review individually in later batches; none looked egress/pivot-relevant by name. |
| `claude/youtube-hiddify-root-cause-6151jb` | 3 | YouTube/Hiddify root-cause investigation — `main` already has extensive YouTube/Hiddify handling (`GOOGLE_EGRESS_DOMAINS`, hairpin, vision-off experiment, `docs/YOUTUBE_FINAL_ROOT_CAUSE.md`), so likely superseded. | Possibly superseded. | Confirm before reviving. |
| `release/v1.0.0-rc.9-vps-acceptance-controller`, `release/v1.0.0-stable-evidence` | 1 each | Historical release-evidence branches. | Archival. | No action; candidates for deletion in a cleanup batch, not reconciliation. |
| `automation/cut-v0.1.3` | 6 | Release-cut automation, unrelated to node data plane. | Unreviewed. | Defer to release-automation batch. |
| `dependabot/*` (3 branches) | 3 each | Automated dependency bumps (`rand` 0.10.3, Rust minor/patch, GitHub Actions minor/patch). | Current, machine-generated. | Normal dependency hygiene — merge/review independently of this security remediation effort, via normal dependabot flow. |

## Cross-repo remediation plan (vpn-web)

`docs/security/ARCANA_CROSS_REPO_REMEDIATION_PLAN_2026-09-27.md` was
**not found in `singbox-vpn`** (the task's named path
`docs/reviews/SINGBOX_VPN_PRODUCTION_READINESS_AUDIT_2026-09-27.md` also
does not exist in this repo — see the report for how this gap was
handled). Both were found instead in the sibling `vpn-web` checkout at
`D:\David610\vpn-web\docs\security\ARCANA_CROSS_REPO_REMEDIATION_PLAN_2026-09-27.md`
and were read from there; `vpn-web`'s own audit doc is
`docs/reviews/ARCANA_CONTROL_PLANE_REMEDIATION_2026-09-27.md`, also
read. Its table entries `E-01`/`E-02` (C-16 egress isolation, relay
loopback exception) are the basis for Phase 2/4 of this batch. Its
`| singbox-vpn | remediation/node-dp-egress-isolation | NODE-DP | E-01, E-02 |`
row assumed that branch would carry the fix — as found above, it does
not; the fix in this batch is implemented directly on the working
branch instead.

## Summary for batch 2+

- No branch in this repo currently implements C-16 egress isolation,
  the relay loopback-pivot fix, or a structural reserved-probe
  principal — this batch's Phase 2–4 work (see commits on
  `claude/arcana-data-plane-production-final`) is the first
  implementation of all three, not a port.
- `remediation/node-agent-job-safety` and `remediation/supply-singbox-pin`
  are small, clean, cherry-pickable commits for their respective later
  batches (job safety, supply chain) — not merged here, per scope.
- The `chatgpt/phase2-*`, `chatgpt/full-plan-completion`, and
  `feat/relay-hardening-live-vps` branches share a "wholesale deletion
  vs. current main" diff signature — they diverged before major
  restructuring (the `compat-config` crate's current route-rule/relay
  architecture) and are not safely mergeable or cherry-pickable at the
  diff level. Any later batch that wants ideas from them should read
  individual commit messages/diffs (`git log <branch> --oneline`), not
  attempt a merge.
- Several branches (`deps/singbox-1.14.1`, `experiment/server-side-quic-reject`,
  `fix/youtube-relay-udp`, `claude/youtube-hiddify-root-cause-6151jb`)
  appear superseded by what is already on `main` — worth confirming and
  deleting in a repo-cleanup pass rather than reconciling.
