# Release qualification report

Decision: NO-GO. Source baseline `6825fcf491fd44e80543edc0b076c5b114cade16`;
integration branch `integration/2026-10-22-production-qualification` adds
#162/#148 and qualification fixes. Obtain its exact commit with
`git rev-parse HEAD`; never bind candidate evidence to the baseline SHA.
Any remote qualification result must name that exact integration commit.

The latest release API lists v1.3.1-rc.2; stable remains v1.3.0. The rc.3
ref lookup returns 404 and workspace/package core version is 1.3.1.
**Next proposed version: v1.3.1-rc.3**, after final checks and authorization.
No tag, binary archive, checksum, SBOM or attestation was produced here.
No corrected artifact can be installed or accepted from this session yet.

| Gate | Evidence / result |
|---|---|
| Baseline full CI | PASS: [39818433277](https://github.com/David610/singbox-vpn/actions/runs/39818433277), 22/22 jobs on exact baseline |
| Baseline Security | PASS: [39818433134](https://github.com/David610/singbox-vpn/actions/runs/39818433134), both CodeQL jobs |
| Local baseline Python | PASS: 176 tests, stdlib discovery |
| Focused new release evidence suite | PASS: 18 tests after failing-before proof |
| Focused capture suite | PASS: 16 tests after failing-before proof |
| Local shell regressions | PASS: build budget, own-address update backfill, safe interrupted-update cleanup, SIGPIPE lint, release version/archive/SBOM/reproducibility/stability contracts |
| Integration Python / registry regeneration | PASS: 183 tests; includes signed-in/Incognito gates, coverage boundaries and issue retention |
| Shorts monitor installer fixture | PASS: historical wrong classifier reproduced; 17 installer checks pass; a stubbed installation is not live monitoring |
| Actual binary reproducibility | UNVERIFIED: the similarly named shell test proves source/pin/installer contracts, not a double binary build |
| Local Rust fmt/clippy/tests/audit and interoperability | BLOCKED: cargo/rustc/sing-box unavailable; shell HTTPS proxy refuses connection |
| Full integration CI and Security | UNVERIFIED until fresh exact-commit runs complete; old PR/base results are not substituted |
| Artifact runtime/ABI/attestation | BLOCKED: no authorized corrected release artifact |
| Clean install / upgrade / repair / downgrade refusal / rollback / interrupted update / reboot | BLOCKED for exact RC: no trusted accessible disposable host/artifact; historical results are retained only as history |
| TLS issuance and actual renewal | BLOCKED: no accessible authorized deployment; zero-attempt dry-run is not PASS |
| Network/provider firewall/egress isolation | UNVERIFIED for rebuilt fin1; baseline CI namespace results are a different scope |
| Native client/device acceptance | BLOCKED: no device, exact RC or deployed server |
| Off-host backup and restore | BLOCKED: local age unavailable; no destination/recipient or real upload/restore proof |

Local commands (run from repo root; complete logs under `/workspace/audit-evidence`):

Machine-readable local results/log hashes: [LOCAL-EVIDENCE.json](LOCAL-EVIDENCE.json).
Refreshed refs, changed files, checks and rulesets: [GITHUB-BASELINE.json](GITHUB-BASELINE.json).

```sh
python3 -m unittest discover -s deploy/lib/tests -p 'test_*.py'
python3 -m unittest discover -s deploy/lib/tests -p test_release_evidence.py
python3 -m unittest discover -s deploy/lib/tests -p test_af_capture.py
bash deploy/lib/tests/test-preflight-build-budget.sh
bash deploy/lib/tests/test-update-backfills-own-address.sh
bash deploy/lib/tests/test-recover-interrupted-update.sh
bash deploy/lib/tests/test-no-pipefail-sigpipe-race.sh
bash deploy/lib/tests/test-release-version-contract.sh
bash deploy/lib/tests/test-release-archive-contract.sh
bash deploy/lib/tests/test-release-sbom.sh
bash deploy/lib/tests/test-release-reproducibility.sh
bash deploy/lib/tests/test-release-stability-regressions.sh
bash deploy/lib/tests/test-shorts-monitor-install.sh
bash deploy/lib/check-release-version.sh v1.3.1-rc.3
```

No privileged fixture was run on a live node. The tested shell fixtures were
inspected for temporary roots/stubs first. I-54 namespace behavior and I-65
repair drift fixtures require root/network/mount privileges here and are not
claimed locally executed. Main's exact-SHA CI exercises them.

Historical issue verification scope:

| ID | Current code/test path | Current host/artifact |
|---|---|---|
| I-31 | updater `update_backfill_own_address`; local backfill test passes; CI hairpin/C-16 interop | UNVERIFIED |
| I-53 | `lint_pipefail_pipes.py`; local adversarial detection/here-string test passes | No production SSH operation performed |
| I-54 | `nftables-egress-isolation.sh`: positive UID rejects; doctor rejects old form; baseline CI behavioral netns test | UNVERIFIED; never install rc.2 as corrected release |
| I-56 | updater transaction recovery, `recover-interrupted-update.sh`; local path-adversary cleanup fixture passes | Whole real SIGKILL lifecycle matrix on intended RC required |
| I-65 | updater repair executable-hash drift guard; root/mount fixture in baseline CI | Helper scripts are not covered by executable hashes alone; exact-release repair required |
| I-68 | evidence probe returns page path; monitor has executable unit/installer tests | No current live probe or delivered alert |
| I-81 | installer/updater source-build guards and managed cache; local build-budget fixture passes | Actual free disk/inodes/build footprint UNKNOWN |
| I-83 | outcome/explanation separated; monitor tests and installer classifier contract | Native cause still UNKNOWN |

Supported clients cannot be promoted by these results. Pinned sing-box 1.14.1
has exact-baseline CI interop. Hiddify-core historical loopback evidence is
not current native Hiddify evidence. INCY, Shadowrocket, official native
sing-box apps, Hiddify iOS/Android/desktop need real import/refresh/revocation/
both-supported-transport testing. v2rayNG is a VLESS-only fallback; do not
promise Hysteria2. The product currently documents <=10 trusted users; the
100-paying-user target needs workload, churn, request-budget and operations
qualification, not an infrastructure rewrite or extrapolation from four flows.
