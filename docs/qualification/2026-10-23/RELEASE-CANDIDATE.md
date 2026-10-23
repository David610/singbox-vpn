# Release-candidate preparation — 2026-10-23

**No tag was created, no release published, nothing deployed.** A prerelease tag push (`v*-*`) auto-publishes through `release.yml`, so no tag
was pushed. `v1.3.1-rc.3` does not exist.

## What exists today

| Item | Value | Evidence |
|---|---|---|
| Base | `main` `548a376803c367c76519df2e59976cd89016044f`, tree `d60a2fad789b10bacf8aa8fb53671024d130873e` | `git rev-parse` |
| CI on that exact SHA | CI and Security both `success` | GitHub Actions runs for 548a376 |
| Workspace version / tag contract | 6 crates at `1.3.1`; `check-release-version.sh v1.3.1-rc.3` PASS; `check-workspace-version-consistency.sh` PASS; a wrong tag (`v1.3.2`) is refused | local run |
| Latest published | `v1.3.0` (Latest), `v1.3.1-rc.2` (pre-release) | `gh release list` |
| Rust build determinism | two clean `cargo build --release --locked` builds of 548a376 (same path, separate target dirs, same toolchain 1.94.1) are **bit-identical** for all four binaries (hashes below) | local |
| SBOM | `generate-sbom.sh --version v1.3.1-rc.3`: CycloneDX 1.5, 193 components, 20 license expressions; **includes the provisioning agent** (I-91) | local |
| Incomplete evidence is rejected | `release_evidence.py --require DEVICE_VERIFIED` on a record with CI success but no device cells exits 1 (`supported_level: NONE`, "schema_version must be 1"); 18 + 9 unit tests for the validator and gate report pass | local |

Local-build hashes of `main` 548a376 (for the **determinism** check only; they are not the release artifact, which is built in the release container):

```
24392757f954d018a9f4af3e2680d08d72b06ae1cf6298f0f0fadd6c028db6bc  vpn
672ebc278e4312dd5e2c34c8692a239d47dcdeafffcbd18f45ee26cead692558  vpn-admin
b5428d819f0171a50acd373f8e6f18d527b2db988c287e4a7a60041eb6fdfaea  vpn-provisioning-agent
d50b35bb198b4dfd280fb4f99aa0756b5913831ce260d674623b09c264773ebc  subscription
fa7665c0a154463e1595b6f8653889f8dc27448a6adaa3ae2abff54b111e440b  singbox-vpn-sbom.cdx.json
```

## Not verified here

| Item | Status | Why |
|---|---|---|
| Container build (`build-release-x86_64.sh`), glibc baseline, packaging | UNVERIFIED | no Docker on this host |
| Cross-host reproducibility (different path/host) | UNVERIFIED | only same-path, same-host double build run |
| Cosign signature, SLSA provenance, SBOM attestation | UNVERIFIED | produced only by `release.yml` on a real tag |
| Binary SHA-256 of the artifact, artifact manifest, configuration digest | UNVERIFIED | no artifact exists |
| Install / upgrade / rollback / SIGKILL-interrupt of the exact artifact | BLOCKED | needs a trusted host (see `docs/handoffs/2026-10-23/VPS-ACCESS-AND-TRUST.md`) |
| Native playback on the exact artifact | BLOCKED | needs an iPhone: `docs/operations/IPHONE_SHORTS_MATRIX_RUNBOOK.md` |
| W16/W17/W19 upstream sing-box risks | UNVERIFIED | not re-tested; the pin (1.14.1) is unchanged, nothing here claims an upstream fix |

## The candidate must contain

The release candidate is whatever `main` is after review. Today's open work that must be merged (or consciously excluded) first:

| Change | Repo / PR | Why it matters for the candidate |
|---|---|---|
| strict-tightening revocation adoption | singbox-vpn #186 | revocations by entitlement are otherwise never applied (I-99) |
| docs link/anchor gate + stale-reference fixes | singbox-vpn (docs branch) | CI keeps docs honest |
| registry regeneration + this report | singbox-vpn (report branch) | single source of issue truth |
| provisioning-agent quota work | singbox-vpn #172 (draft) | **exclude** from rc.3 unless I-100/I-102 are decided and the paired vpn-web change is deployed first; it is held on purpose |

## Cutting rc.3 (owner-authorized step; do not run before approval)

1. Merge the PRs above after full CI + Security on each exact merge SHA (required checks are now enforced on `main`, issue #170).
2. Confirm the version contract: `bash deploy/lib/check-release-version.sh v1.3.1-rc.3`.
3. Record: commit, tree, `git archive` sha256, then push the tag (this publishes a pre-release by design).
4. From the workflow outputs record: tarball sha256, SBOM and license inventory hashes, cosign bundle verification, provenance (`docs/SUPPLY_CHAIN_SECURITY.md`).
5. Fill a `docs/release-acceptance/v1.3.1-rc.3.evidence.json` **only** with results from that exact commit and binary digest, then run `release_evidence.py --require <level>`.

## Objective rollback triggers (to validate before activation on a host)

Any of: `vpn doctor` FAIL on either protocol after install; sing-box not active 60 s after install/update; install or update exits non-zero;
external probe cannot complete a REALITY or Hysteria2 handshake; backup run fails to upload and verify; any native matrix cell FAIL on a
previously passing behaviour. Rollback path: `update.sh` rollback and recovery (`docs/operations/RECOVERY_AND_ROLLBACK.md`); the lifecycle tests
for SIGKILL/rollback exist in CI, **but have not been run against this candidate on a host**.
