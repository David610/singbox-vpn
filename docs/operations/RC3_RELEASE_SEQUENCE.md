# v1.3.1-rc.3 — exact branch / PR / CI / release sequence (for owner authorization; NOTHING here has been executed)

Why: the published `v1.3.1-rc.2` re-creates the I-54 IPv6 breakage on install, update and repair, and its updater has none of the
recovery/repair guards. The production updater installs only **CI-built, cosign-attested** assets, so a corrected node cannot be
produced by hand: the corrected source has to go through the repository's own release workflow. Versioning follows `docs/RELEASE.md`:
workspace version stays `1.3.1`, the prerelease identity is the tag `v1.3.1-rc.3` (draft notes: `docs/release-notes/v1.3.1-rc.3-DRAFT.md`).

State at preparation: `origin/main` = `de8f7d5` (unchanged, re-fetched 2026-10-21); the branch contains it (fast-forwardable), 74+ commits ahead.
Staging evidence for the exact source: `docs/handoffs/2026-10-21/` (matrix runs, candidate provenance, monitor, profile checks).

## Authorizations, in order (each is separate)
| # | Action | Command | Effect / risk | Rollback |
|---|---|---|---|---|
| R1 | Push the branch | `git push -u origin staging/2026-10-20-remediation` | creates a remote branch; `ci.yml` runs on every branch push (no release, no publish, no write scope: `permissions: contents: read`) | `git push origin --delete staging/2026-10-20-remediation` |
| R2 | Fix CI-only failures | (I iterate on the branch; each push re-runs CI) | none beyond R1; CI has never run on this branch, so expect some findings (os-matrix, audit, release-container-smoke, singbox-validate are not reproducible on this host) | revert commits |
| R3 | Open a PR into `main` | `gh pr create --base main --head staging/2026-10-20-remediation` (repo has no PR template; description from `docs/handoffs/2026-10-21/`) | review surface only | close PR |
| R4 | Merge (owner decision on merge style; a merge commit keeps the 74 commits' history) | via GitHub | `main` changes | revert merge commit |
| R5 | Tag the merged `main` commit | `git tag -a v1.3.1-rc.3 -m "..." <merge-sha> && git push origin v1.3.1-rc.3` | triggers `release.yml`: validate-tag, version, policy, CI gate, build in the pinned AlmaLinux 8.10 container (glibc baseline), SBOM/attestation, runtime-compat, **publishes a GitHub PRERELEASE**, verifies the published bootstrap | delete the prerelease and tag (assets of a published tag must never be silently replaced: cut `rc.4` instead) |
| R6 | I install it on the staging node | `update.sh --version v1.3.1-rc.3` from the installed tree | real attested upgrade rc.2 -> rc.3 on the staging node only | transactional rollback (proven by the matrix) |

R5 is the point of no return for the tag name: nothing is published before it. Stable `v1.3.1` additionally needs the device matrix (`docs/RELEASE.md`) and is out of scope.

## What I run after R5 (all on the staging node, nothing else)
1. Real upgrade `rc.2 -> rc.3` with attested assets; `vpn-admin doctor --protocol --require-protocol`; `nft list table inet arcana_egress_isolation | grep -c 'skuid !='` must be 0; `ip -6 neigh` gateway not FAILED.
2. `update.sh --repair` on rc.3 (hash guard must pass because binaries equal the recorded release) repeated twice; IPv6 stays healthy.
3. The 16-stage interruption matrix again with `TARGET=v1.3.1-rc.3` on the immutable revision (two runs), because the payload is then the real corrected release.
4. Rollback proof (`--version v1.3.0 --allow-downgrade` is documented; the transactional rollback is covered by the matrix).
5. Release-gate report (`docs/handoffs/2026-10-21/RELEASE-GATE.md`) recomputed from machine-readable results.

## What is NOT covered by any of this
YouTube's decisions (Shorts), native app behaviour, UDP on your networks, provider firewall, reboot and power-loss recovery: see the gate report.
