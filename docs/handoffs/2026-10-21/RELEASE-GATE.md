# Release gate report — revision `97ef78f (code qualified at 3570927; diff = cfg(test) message + registry generator)`

**Verdict: FAIL** (computed; mandatory gates: PASS=8, FAIL=1, FLAKY=1, BLOCKED=6, UNVERIFIED=10)

## Source qualification

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| SRC-1 Rust fmt, clippy, rustdoc, workspace tests (1097) | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/qualify-3570927/SUMMARY.tsv`; `docs/handoffs/2026-10-21/evidence/qualify-3570927/cargo-test-totals.txt`; `docs/handoffs/2026-10-21/evidence/qualify-3570927/ENVIRONMENT.txt`. Run on an immutable export of 3570927. HEAD 97ef78f differs only by a cfg(test) assertion message and the registry generator. |
| SRC-2 Shellcheck, hermetic shell suite, Python tooling tests, workflow contract | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/qualify-3570927/SUMMARY.tsv`. 107/107 gates PASS at 3570927. The earlier 625a34f run's single FAIL was a launch-environment error (kept: qualify-625a34f-FAILED-launch-env). |
| SRC-3 No unresolved flaky tests | yes | **FLAKY** | `docs/handoffs/2026-10-21/FLAKY-POLICY.md`; `docs/handoffs/2026-10-21/evidence/staging-3570927/devrebuild-attempt1-FAILED.log`; `docs/handoffs/2026-10-21/evidence/staging-268808d/candidate-268808d-devrebuild.log`. I-71 (reality_decoy_budget, 1 failure, client-abandoned authenticated handshake) and I-75 (admin service reload burst, 1 failure, verify-window hypothesis). Neither reproduced; neither fixed; policy: blocks any production release. |
| SRC-4 GitHub CI on the branch (os-matrix, audit, container smoke, singbox-validate, security) | yes | **UNVERIFIED** | -. Branch never pushed (R1 needs owner authorization). |
| SRC-5 Secrets scan and release provenance checks in CI | yes | **UNVERIFIED** | -. Run by security.yml / release.yml; not reproducible locally. |

## Artifact qualification

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| ART-1 CI-built, cosign-attested rc.3 artifact (checksums, SBOM, provenance) | yes | **BLOCKED** | -. Needs R1..R5 (docs/operations/RC3_RELEASE_SEQUENCE.md); nothing pushed, tagged or published. |

## Lifecycle acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| LIF-1 16-stage interruption matrix, two identical runs, updater at 268808d (update.sh/install.sh/recovery/harness byte-identical at HEAD) | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/matrix-A-268808d/matrix.jsonl`; `docs/handoffs/2026-10-21/evidence/matrix-B-268808d/matrix.jsonl`; `docs/handoffs/2026-10-21/evidence/matrix-A-268808d/baseline.json`. 16/16 each, 0 differences, same harness hash. Payload is the published rc.2 (not a CI-built corrected release). |
| LIF-2 Staging dev build of the qualified revision: I-54 fixed (skuid 995), IPv6 ND, doctor 0 FAIL, repair guard refuses | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/staging-3570927/doctor-protocol-97ef78f.txt`; `docs/handoffs/2026-10-21/evidence/staging-3570927/repair-guard-97ef78f.txt`; `docs/handoffs/2026-10-21/evidence/staging-3570927/devrebuild-attempt2-OK.log`. Development build, not a release; attempt 1 failed its pre-install tests (I-75) and installed nothing. |
| LIF-3 Attested upgrade rc.2 -> rc.3, repeated --repair on the installed release, rollback | yes | **BLOCKED** | -. Needs the CI-built artifact (ART-1). |
| LIF-4 Fresh installation with the final installer (isolated, no public certificate issuance) | yes | **UNVERIFIED** | -. Not run. Unit/hermetic tests only (installer-scratch-cleanup etc.). |
| LIF-5 Reboot acceptance (services, egress table, IPv6 ND, ufw, both protocols) | yes | **BLOCKED** | -. Needs owner authorization and confirmed out-of-band access. |

## Network acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| NET-1 REALITY and Hysteria2 self-tests on the installed build | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/staging-3570927/doctor-protocol-97ef78f.txt`. Loopback throwaway clients; not a native client. |
| NET-2 Tunnel isolation (host-local services unreachable through REALITY/Hysteria2), tested through real tunnels at this revision | yes | **UNVERIFIED** | -. Behavioural test not repeated at this revision this session. |
| NET-3 Google hairpin routing, fail-closed and mutations (SIMULATED two-node) | no | **PASS** | `docs/handoffs/2026-10-21/evidence/hairpin/clean-1.json`; `docs/handoffs/2026-10-21/evidence/hairpin/mut-direct-route.json`. Simulation only: proves routing and fail-closed, nothing about any real address. |

## External-service acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| EXT-1 Real YouTube Shorts playback through a verified-good Google-facing egress | yes | **BLOCKED** | `docs/handoffs/2026-10-21/evidence/monitor-live-escalation.jsonl`. Staging IP is refused (0/5 REFUSED, repeated). Needs an owner-authorized target (OWNER-ACTIONS #3). |
| EXT-2 Shorts monitor: healthy baseline, recovery, alert-clear against a real egress | yes | **BLOCKED** | -. No good egress exists to recover to. |
| EXT-3 Shorts monitor state machine, hermetic (classes, hysteresis, dedup, recovery, expiry) | no | **PASS** | `docs/handoffs/2026-10-21/evidence/qualify-3570927/SUMMARY.tsv`. python-test_shorts_health in the 3570927 qualification; simulation, not real acceptance. |

## Native-client acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| CLI-1 Subscription profiles from the production pipeline, server-side (103 checks) | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/profile-check-268808d.json`. Profiles only; every native app is UNVERIFIED. |
| CLI-2 Hiddify iOS native acceptance | yes | **UNVERIFIED** | -. Needs the iPhone procedure (OWNER-ACTIONS #4), including control step 7b. |
| CLI-3 Hiddify Android native acceptance | yes | **UNVERIFIED** | -. No device. |
| CLI-4 Other officially supported clients (sing-box native, Shadowrocket, v2rayNG, others) | yes | **UNVERIFIED** | -. No device. |

## Security acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| SEC-1 Host firewall: default deny, only 443/tcp+udp and 8443 public, SSH rate-limited, v4+v6 | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/security/posture-20261021.txt`. Provider-level firewall not visible from here. |
| SEC-2 SSH key-only login | yes | **BLOCKED** | `docs/handoffs/2026-10-21/evidence/security/posture-20261021.txt`. Password and root login are on and root has no authorized_keys; needs the owner's public key (OWNER-ACTIONS #2). |
| SEC-3 Credential audit (logs, git history, artifacts, backups, archives) | yes | **UNVERIFIED** | -. Not performed this session. |

## Performance acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| PERF-1 Throughput, concurrency 1/4/8, 5-minute soak, restart recovery (loopback client) | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/perf/perf-2026-10-21.json`. Loopback client sharing host CPU; NOT real-network throughput; measured on the earlier dev build with the same data-plane configuration. |
| PERF-2 REALITY added connection latency <= 40 ms (I-60) | yes | **FAIL** | `docs/handoffs/2026-10-21/evidence/perf/perf-2026-10-21.json`; `docs/handoffs/2026-10-21/evidence/perf/i60-decoy-candidates.json`. 61.9 ms median on the node (Hysteria2 2.6 ms). Lab evidence: a different remote decoy costs 9-11 ms through REALITY (microsoft.com 73 ms), so the gate is reachable without a local decoy; NOT applied, node not re-measured. Threshold unchanged; owner decision (OWNER-ACTIONS #5). |
| PERF-3 Real-network performance and long-term stability (> 5 min) | yes | **UNVERIFIED** | -. Not measured. |

## Operations

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| OPS-1 Shorts monitor deployed from source control and verified (install + --check) | yes | **UNVERIFIED** | -. I-70: install-shorts-monitor.sh --check passes on staging (hermetic test 14/14, 4 mutations), but install/update do not run it and doctor does not call it; promote after the CI run and an owner decision on default deployment. |

