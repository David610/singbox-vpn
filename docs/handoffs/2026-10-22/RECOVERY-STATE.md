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
None of the reconstructions needs the VPN. Next is owner input: SSH key, backup destination, and approval to install and provision identities. See the plan in this commit's report.

## Milestones
| # | Work | Origin | SHA | Tests |
|---|---|---|---|---|
| 0 | Checkpoint | — | `dd22c7492bfe501221b02cb51d4f066890f5784e` | — |
| 1 | I-81 disk/inode build budget, shared Cargo target, scoped prune (`deploy/lib/preflight.sh`, `qualify-local.sh`) | RECONSTRUCTED (original `3d7c005` MISSING) | `a0964336d72bf6eb8fd232335712860163cbe210` | `bash deploy/lib/tests/test-preflight-build-budget.sh`: 19/19 ok on fin1, 2026-10-22. Regression: test-preflight-curl-retry and test-installer-hardening pass. shellcheck is not installed (UNVERIFIED). |
| 2 | I-83: outcome `MEDIA_PLAYBACK_FAILED` and a separate `explanation` field (default `UNKNOWN`), plus `failure_mode` and `context`. Fields are in `deploy/lib/monitoring/shorts_health.py`. Install and `--check` refuse a reverted classifier. | RECONSTRUCTED (originals `016f682`/`7e822e8` MISSING) | `f75b863bbe25557a2812362eb12fb34b0f8bd209` | `python3 -m unittest test_shorts_health` (in `deploy/lib/tests`): 40/40 OK. It fails against the `df47737` classifier (1 failure, 9 errors). `test-shorts-monitor-install.sh` PASS (17 checks). Not deployed: no VPN install exists. |
| 3 | `deploy/lib/diagnostics/af_capture.py`, with `capture`, `status` and `analyze` subcommands. It counts IPv4/IPv6 TCP and UDP for tunnel ingress and Google egress in 5 s bins, stores aggregates only, and writes no pcap. The capture expires automatically, with a 1800 s cap and a 256-byte snap length, and uses `-e` frame lengths. It returns `VALID_OVERLAP` or `NO_OVERLAP_INCONCLUSIVE` with a reason. | RECONSTRUCTED (corrected original MISSING) | `6eb57b4c3ee996f0c9f21dee4f95b5905ee7eefe` | `python3 -m unittest test_af_capture` (in `deploy/lib/tests`): 15/15 OK on recorded line shapes. Mutation checks against reintroduced defects all fail the suite: dropping `[|tcp]`, removing 2a00:1450::/32, and accepting a 1-bin overlap. No live capture was run, because the VPN is not restored. |

### Differences from the historical description (reconstructions)
- I-81: the exact original thresholds are unknown. The defaults are a 10 GiB reserve and an 8 GiB build estimate, both overridable. They are not yet measured on this host; `qualify-local.sh` records `target_dir_mib` after each run. Disposable output must carry a marker file (`.singbox-vpn-disposable`), so unmarked old dirs are never deleted.
- I-83: the explanation evidence is new interface. It is a `reference` result in the probe output plus `--native-evidence`, and the probe does not produce `reference` yet. Until it does, failures stay UNKNOWN, which is the conservative behaviour. Before the change, causes were `EGRESS_REFUSED`/`PLAYBACK_STALLED`/`MIXED`; now the cause is `MEDIA_PLAYBACK_FAILED` and the detail is in `failure_mode`.
- af_capture: the original CLI, file format and live status layout are unknown, so these are new. The canary windows are supplied by the operator, because packets cannot distinguish the agent's canaries from the phone. Bins are in UTC unix seconds.
- If the original session exports arrive, diff them against these files first and keep any tested behaviour the originals had that these lack.

## CI on `85fe2c7` (run 39799935640, 2026-10-22)
- 20 jobs succeeded, including `shell` (shellcheck and all `deploy/lib/tests/*.sh`), `test`, `fmt`, `clippy`, `release-build` and every os-matrix job. `offline-full-verdict` was skipped.
- 2 jobs failed: `audit` (the "Run cargo audit" step) and `singbox-validate` (the "Real VLESS+REALITY and Hysteria2 interoperability tests" step). Both **also fail on the untouched baseline `df47737`**, so they are not caused by the reconstructions.
- These are the "two failing jobs" whose fixes were reportedly in the MISSING `e93f29d`. They are the next engineering item, after which the restoration plan proceeds (`RESTORATION-PLAN.md`).

## Session 3 (2026-10-22): CI repair, I-83 review, backup tooling
| # | Work | Commit | Evidence |
|---|---|---|---|
| 4 | `audit`: lock bumps rustls-webpki 0.103.15→0.103.16 (RUSTSEC-2026-0382) and webpki-roots 1.0.9→1.0.10 (RUSTSEC-2026-0381). Both are transitive via reqwest's rustls in the provisioning agent; no ignores added. | `da68f9b9a5570f81f5d970cf8f4379a7d6d19cd3` | CI `audit` succeeded on runs 39807126424, 39807318906 and 39807978407 |
| 5 | `singbox-validate` step 10: an exact interop manifest (12 tests) replaces `expected=11`. | `da68f9b` and `d850d81` (ANSI fix) | Unit tests 13/13. CI run 39807978407: step 10 passed. Step 12 then failed; see below. |
| 6 | I-83 review: the explanation `EGRESS_REFUSED` becomes `EGRESS_DEPENDENT_FAILURE`, never emitted as a refusal. | `2d0da8dd47754cd6fdd3be6085d370d88089facb` | `test_shorts_health` 42/42; the monitor installer test passes |
| 7 | Encrypted off-host state backup (`deploy/lib/backup/state-backup.sh`, `BACKUP.md`) | `3df309c9248517f17769625a9797f5be71568f2c` | `test-state-backup.sh` 20/20 on fin1 with real age 1.0.0; CI `shell` succeeded on run 39807978407 |

**Masked failure found:** `singbox-validate` step 12, the simulated two-node Google hairpin, fails at `vpn-admin init` for node T
(`AssertionError: init T`). It never ran while step 10 was failing, so it is pre-existing and not caused by these commits.
The next commit makes the assertion report init's stderr, so CI shows the cause.

| 8 | The hairpin simulation (`singbox-validate` step 12; it had never run in CI) uses a private `SINGBOX_VPN_LOCK_PATH`. The host lock gave EACCES at `vpn-admin init`. | `5cbd398` (diagnostic), `1ed9693755cfdcb872041a101055a94906cf5a93` (fix) | see below |

**CI GREEN at `1ed9693755cfdcb872041a101055a94906cf5a93`** (run 39810417865): 22/22 jobs succeeded, 0 failed, 0 skipped. That includes
`audit`, all `singbox-validate` steps (the 12-test interop manifest, the hairpin simulation and its mutation checks), `shell` (the backup test
with real age), `test`, `fmt`, `clippy`, `release-build`, `offline-full-verdict` and every os-matrix job. This is CODE_QUALIFIED
evidence only. It says nothing about a deployed host, a device or Shorts.

## Installation prerequisites (state at this commit)
| Prerequisite | State |
|---|---|
| Corrected source at an exact commit, CI green | MET: `1ed9693` |
| Disk budget (I-81) | MET: 34 GB free against an 18 GB build need |
| Owner SSH key installed and password login disabled | NOT MET: waiting for the owner's public key. Over 3,300 failed root password attempts are already in the log. |
| Pre-install Hetzner snapshot | NOT MET (owner) |
| Backup destination and dedicated backup recipient | NOT MET (owner) |
| Owner authorization to install and provision 3 new identities | NOT GIVEN |
| DNS for fin1.xn----8sba1cvaif4e.com points at this host | to be checked immediately before install |

## Session 4 (2026-10-22): pre-install review
- `cad9f64`: backup hardening. It refuses a writable env file, validates the sftp target, and adds `prune`, `run`, and systemd timer and path units. 30/30 on fin1.
- `92ba6a4`: I-81 guard on the installer and updater source builds. **CI 22/22 green, run 39812810442.**
- `PRE-INSTALL-CHECKLIST.md`: owner actions A1–A7, the install-source review, the identity plan (secrets suppressed during install; owner-run commands deliver the links) and the production-readiness gates.
- Security observation: 5 successful root **password** logins since the rebuild, from 78.109.69.17 and 185.194.184.53. The owner has been asked to confirm (A3).
