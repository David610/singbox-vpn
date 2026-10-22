# Release gate report — revision `19e93fd (qualified and installed on staging as a development build)`

**Verdict: FAIL** (computed; mandatory gates: PASS=10, FAIL=1, FLAKY=1, BLOCKED=7, UNVERIFIED=8)

## Source qualification

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| SRC-1 Rust fmt, clippy, rustdoc, workspace tests (1097) | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/qualify-19e93fd/SUMMARY.tsv`; `docs/handoffs/2026-10-21/evidence/qualify-19e93fd/cargo-test-totals.txt`; `docs/handoffs/2026-10-21/evidence/qualify-19e93fd/ENVIRONMENT.txt`. 109/109 gates at 19e93fd on an immutable export (1105 workspace tests, 0 failed/ignored). HEAD may carry docs-only commits after it. |
| SRC-2 Shellcheck, hermetic shell suite, Python tooling tests, workflow contract | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/qualify-19e93fd/SUMMARY.tsv`. Includes the new proc-safe, clean-host summary, monitor installer and secret-scan tests. The 625a34f run's single FAIL was a launch-environment error (kept: qualify-625a34f-FAILED-launch-env). |
| SRC-3 No unresolved flaky tests | yes | **FLAKY** | `docs/handoffs/2026-10-21/FLAKY-POLICY.md`; `docs/handoffs/2026-10-21/evidence/staging-3570927/devrebuild-attempt1-FAILED.log`; `docs/handoffs/2026-10-21/evidence/staging-268808d/candidate-268808d-devrebuild.log`. I-71: 1 failure, unexplained, ~150 clean runs, assertion deliberately unchanged, client log + load now captured. I-75: root cause reproduced and FIXED (count-based verify bound); still FLAKY by policy until 3 clean qualification runs on distinct revisions (1 so far: 19e93fd). Either blocks a production release. |
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
| LIF-2 Staging dev build of the qualified revision: I-54 fixed (skuid 995), IPv6 ND, doctor 0 FAIL, repair guard refuses | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/staging-19e93fd/doctor-protocol-19e93fd.txt`; `docs/handoffs/2026-10-21/evidence/staging-19e93fd/repair-guard-19e93fd.txt`; `docs/handoffs/2026-10-21/evidence/staging-19e93fd/devrebuild-OK.log`. Development build of 19e93fd: I-54 fixed (0 negated/3 positive skuid rules), gateway REACHABLE, doctor 0 FAIL, --repair refused. Not a release. The pre-install test gate failed once per attempt on I-71, I-75 earlier and passed on this build. |
| LIF-3 Attested upgrade rc.2 -> rc.3, repeated --repair on the installed release, rollback | yes | **BLOCKED** | -. Needs the CI-built artifact (ART-1). |
| LIF-4 Fresh installation with the final installer (isolated, no public certificate issuance) | yes | **UNVERIFIED** | -. Not run. Unit/hermetic tests only (installer-scratch-cleanup etc.). |
| LIF-5 Reboot acceptance (services, egress table, IPv6 ND, ufw, both protocols) | yes | **BLOCKED** | -. Needs owner authorization and confirmed out-of-band access. |

## Network acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| NET-1 REALITY and Hysteria2 self-tests on the installed build | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/staging-19e93fd/doctor-protocol-19e93fd.txt`. Loopback throwaway clients; not a native client. |
| NET-2 Tunnel isolation (host-local services unreachable through REALITY/Hysteria2), tested through real tunnels at this revision | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/staging-19e93fd/c16-isolation-19e93fd.json`. At 19e93fd, through real sing-box clients for REALITY and Hysteria2: own IPv4/IPv6 :22 and the loopback subscription backend are rejected; the control destination stays reachable. Loopback client on the node (server verified). |
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
| SEC-3a Credential audit of the node and the commits to be pushed (live secrets vs logs, history, tmp, evidence, repo; file permissions; log level) | yes | **PASS** | `docs/handoffs/2026-10-21/CREDENTIAL-AUDIT.md`. No live secret found outside its restricted files; loose backup permissions found and fixed. The audit method is the scan tool (labels only). |
| SEC-3b Credential audit of CI logs, uploaded artifacts and the rc.3 release archives/SBOM | yes | **BLOCKED** | -. Does not exist until CI runs (R1) and a tag is built (R5). |

## Performance acceptance

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| PERF-1 Throughput, concurrency 1/4/8, 5-minute soak, restart recovery (loopback client) | yes | **PASS** | `docs/handoffs/2026-10-21/evidence/perf/perf-2026-10-21.json`. Loopback client sharing host CPU; NOT real-network throughput; measured on the earlier dev build with the same data-plane configuration. |
| PERF-2 REALITY added connection latency <= 40 ms (I-60) | yes | **FAIL** | `docs/handoffs/2026-10-21/evidence/perf/perf-2026-10-21.json`; `docs/handoffs/2026-10-21/evidence/perf/i60-decoy-candidates.json`. 61.9 ms median on the node (Hysteria2 2.6 ms). Lab evidence: a different remote decoy costs 9-11 ms through REALITY (microsoft.com 73 ms), so the gate is reachable without a local decoy; NOT applied, node not re-measured. Threshold unchanged; owner decision (OWNER-ACTIONS #5). |
| PERF-3 Real-network performance and long-term stability (> 5 min) | yes | **UNVERIFIED** | -. Not measured. |

## Operations

| Gate | Mandatory | Status | Evidence / note |
|---|---|---|---|
| OPS-1 Shorts monitor deployed from source control and verified (install + --check) | yes | **UNVERIFIED** | `docs/handoffs/2026-10-21/evidence/staging-19e93fd/shorts-monitor-check.txt`. I-70: install-shorts-monitor.sh --check passes on staging (14-check hermetic test, 4 mutations). Install/update do not run it and doctor does not call it; promote after CI and an owner decision on default deployment. |

