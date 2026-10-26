# v1.3.1-rc.8 readiness record (2026-10-26)

Secret-free. Evidence classes: SOURCE, CI, REAL-SINGBOX, STAGING (two disposable VPSs, Europe -> Europe), ENGINE
(Hiddify core CLI), DEVICE. No DEVICE or RUSSIA evidence exists for anything below.

## Candidate
- Code state qualified: `5f27fdd7414bf520c985fde412e1cbc6b9155fc9` (PR #221). The candidate commit is the `main`
  HEAD after the docs-only PR that adds this record; it differs from `5f27fdd` only in documentation
  (`docs/INSTALLATION.md`, `docs/release-notes/v1.3.1-rc.8.md`, this file) and is verified by its own CI run.
- Predecessor: `v1.3.1-rc.7` = `50a59be5244546fc4204e9193e273b98d9bdd3e5` (published, attested). `v1.3.1-rc.8` does
  not exist yet; publication needs separate owner authorization.

## What changed since rc.7 (all merged through protected PRs)
| PR | Merge | Change |
|---|---|---|
| #220 | `09239f1` | `pair issue-*`: only the bundle on stdout; refuse before creating the credential; reserve the output file; revoke the credential and delete a partial file if delivery fails |
| #221 | `5f27fdd` | `hiddify_core_acceptance.py --mode relay`; URL redaction; end-to-end test of the runbook pipe; IPv6-by-hostname evidence |

## Verification
- First-pass CI and Security on `5f27fdd`: attempt 1, all jobs green (including `singbox-validate` with the two-hop
  suite, `release-container-smoke`, `release-build`, `shell`, `audit`, `docs`, seven `os-matrix` images,
  `offline-full-verdict`, CodeQL). No retries were needed for #220 or #221 either (47/47 on the first run).
- Local, first pass, on `5f27fdd`: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings` and
  `cargo doc -D warnings` clean; `cargo test --workspace`: **1230 passed, 0 failed**.
- Python: `test_hiddify_core_acceptance.py` 15 tests (profile analysis incl. bypass mutations, URL redaction).
- Regression proof for the defect: `pair_issue_writes_only_the_bundle_...` FAILED on the unfixed code ("stdout must
  start with the JSON document") and passes now; `/dev/full` tests prove the revoke path; the documented pipe is
  tested byte for byte (`pairing_runbook_pipe_issue_stdout_feeds_accept_stdin_directly`).
- STAGING (rc.7 binaries, unchanged by these fixes): the full two-hop matrix passed; recorded in the
  staging report of 2026-10-26. The rc.8 fixes themselves are covered by SOURCE/CI evidence until staging is
  upgraded.
- ENGINE: the new relay mode was run for real against the staging relay (1/1 PASS, exit egress, profile chain
  verified, negative control fails with exit 1); the unchanged direct mode ran 6/6 on the staging exit.
- IPv6 by hostname: no defect (`IPV6_BY_HOSTNAME.md`).

## Honest gaps
- The pairing fix was proven in CLI tests with fake services and a failing stdout; it has not run on a live host
  because rc.7 is installed there. The first live proof is the staging upgrade below.
- `update.sh` upgrade rc.7 -> rc.8 and interrupted-update recovery have not been run on the staging hosts.
- Ubuntu is best-effort. Russian ISP reachability, native Hiddify apps, UDP/QUIC through the relay, Chromium YouTube
  playback and the nested-Vision hang remain unverified.

## Verdict
rc.8 is **ready to publish as a candidate** (all gates above green, one defect class closed with failing-before
tests). It is not FRIENDS_READY and does not change any Russian-path or device conclusion.

## Publication and staging upgrade (owner-gated)
1. Tag the candidate commit (annotated) and push the tag; the Release workflow builds, attests and publishes.
2. Verify the published assets locally (SHA256SUMS, `gh attestation verify`, source digest = candidate commit).
3. Upgrade staging, exit first: `sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version v1.3.1-rc.8` on fin1,
   then de1. Expect: users and REALITY/Hysteria2 material preserved, pairing and hairpin intact, spoof capability
   unchanged, `doctor` 0 FAIL, `pair status` unchanged, no client disconnect beyond the restart.
4. Re-run the relay-mode Hiddify-core acceptance and the traffic matrix; then issue a NEW test pairing with the
   documented pipe (no `--out` workaround) as the live proof of the fix.
