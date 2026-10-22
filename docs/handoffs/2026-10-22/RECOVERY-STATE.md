# Recovery state: 2026-10-22 (fin1 rebuilt)

Live checkpoint. It contains no secrets. Update it at each milestone.

## Baseline
- GitHub baseline: `df47737eaca761f9a2a9bb30c91aa7000b98168d` (`staging/2026-10-20-remediation`). `main` is `de8f7d5`.
- Recovery branch: `recovery/2026-10-22`, created from the baseline. Pushes use a repo-scoped deploy key with strict host-key checking. They never go to `main` and are never forced.
- Local HEAD: see `git log -1` on this branch. Each milestone below records its pushed SHA.

## Rebuilt VPS (fin1, 62.238.46.190)
- Fresh Ubuntu 22.04.5, kernel 5.15.0-190. The disk was rebuilt on 2026-10-22.
- No VPN runtime: no sing-box, nginx, `/etc/vpn`, `/opt/singbox-vpn`, or `/root/evidence`.
- SSH: password and root login are enabled and `authorized_keys` is empty. This is temporary. Do not disable password login until owner-key and console access are verified.
- Disk: 38 GB total, about 34 GB free, inodes 2% used. 2 vCPU, 3.8 GB RAM.

## Surviving code
Everything on GitHub up to `df47737` (94 commits ahead of `main`).

## MISSING (not reconstructed, not declared irrecoverable)
The range `df47737..e93f29d` plus `016f682` and `7e822e8`. Ordering and intermediate commits are UNKNOWN. Known hashes:
- `3d7c005`: 111-gate qualification and the I-81 disk preflight.
- `c68ce65`: bookkeeping; the R1 bundle was cut from this revision.
- `e93f29d`: the last local HEAD, with CI audit and sing-box validation fixes.
- `016f682`: the I-83 monitor classification.
- `7e822e8`: 53 monitor and capture tests.

Missing files also include the corrected `deploy/lib/diagnostics/af_capture.py` and `/root/evidence/R1/*.bundle`.

Possible source: the nine original Claude Code cloud sessions. The owner's export is pending. If those exports arrive, diff them against the reconstructions below before replacing anything.

## Historical test results (previous host and revisions; NOT run here)
- 111/111 gates (`3d7c005`) and 109/109 (`19e93fd`).
- 1109 Rust tests.
- 16/16 interrupt matrix, twice.
- 103/103 profile checks.
- 53 monitor and capture tests (`7e822e8`).

## Shorts, native iPhone (old installation, 2026-10-22)
| Connection | YouTube state | Result |
|---|---|---|
| VPN off | Incognito | WORKS |
| fin1 REALITY | Signed in | WORKS |
| fin1 REALITY | Incognito | FAILS |
| fin1 Hysteria2 | Incognito | FAILS |

Retest after restoration. A 155-byte UMP response from automated Chromium does not prove an IP ban.

## Authorizations
- Granted: the repo-scoped deploy key; pushes to `recovery/2026-10-22`.
- Not granted:
  - VPN install or credential creation;
  - SSH changes;
  - E1 changes;
  - a hairpin, IPv6 reject, decoy change, or new VPS;
  - using or logging out the full-account `gh` login;
  - a release or tag.

## Risks
- Disk: a build can refill it. I-81 is the next milestone.
- Backups: no encrypted off-host backup exists for deployment state. GitHub holds source only.

## Next safe operation
Reconstruct `af_capture.py` from fixtures, test-first. No live capture yet.

## Milestones
| # | Work | Origin | SHA | Tests |
|---|---|---|---|---|
| 0 | Checkpoint | — | `dd22c7492bfe501221b02cb51d4f066890f5784e` | — |
| 1 | I-81 disk/inode build budget, shared Cargo target, scoped prune (`deploy/lib/preflight.sh`, `qualify-local.sh`) | RECONSTRUCTED (original `3d7c005` MISSING) | see `git log` | `bash deploy/lib/tests/test-preflight-build-budget.sh`: 19/19 ok on fin1, 2026-10-22. Regression: test-preflight-curl-retry and test-installer-hardening pass. shellcheck is not installed (UNVERIFIED). |
| 2 | I-83: outcome `MEDIA_PLAYBACK_FAILED` and a separate `explanation` field (default `UNKNOWN`), plus `failure_mode` and `context`. Fields are in `deploy/lib/monitoring/shorts_health.py`. Install and `--check` refuse a reverted classifier. | RECONSTRUCTED (originals `016f682`/`7e822e8` MISSING) | see `git log` | `python3 -m unittest test_shorts_health` (in `deploy/lib/tests`): 40/40 OK. It fails against the `df47737` classifier (1 failure, 9 errors). `test-shorts-monitor-install.sh` PASS (17 checks). Not deployed: no VPN install exists. |
