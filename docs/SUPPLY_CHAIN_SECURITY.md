# Release and supply-chain security

Status: 2026-08-22. This document describes the supported stable release path,
not the explicitly untrusted development channel.

## Current trust chain

The operator runs `curl .../main/install.sh | sudo bash`. HTTPS and control of
the canonical GitHub repository authenticate that first root-executed script;
there is no independent signature on it. The script resolves one immutable
release tag, downloads `singbox-vpn-src.tar.gz` and `SHA256SUMS`, checks the archive
digest, and then verifies GitHub artifact provenance with
`gh attestation verify`, pinned to repository `David610/singbox-vpn` and signer
workflow `.github/workflows/release.yml`. Only then does it extract and execute
the release source as root. The deployment installer repeats
checksum and attestation verification for the architecture-specific binary
archive. Production update applies the same checks before live mutation.

SHA256SUMS provides integrity, **not independent authentication**: the archive
and digest are published from the same GitHub trust root. The attestation adds
an Actions OIDC-backed statement binding the artifact digest to this repository.
It does not protect against a malicious/compromised workflow that legitimately
requests an attestation for malicious output.

Every stable release requires provenance and fails closed, with no exception
for historical pre-`v0.1.3` releases. An earlier version of this script
exempted any release whose *version string* claimed to predate `v0.1.3` from
attestation, falling back to checksum-only verification for it. That
exemption was gated entirely on attacker-suppliable release metadata (the tag
name / embedded package version): an attacker with release-publish access —
the exact actor attestation exists to contain — could delete and republish an
old-numbered or malformed tag with malicious content and skip attestation
entirely, while SHA256SUMS still matched because the attacker controlled both
the archive and its checksum manifest. There is no way to keep a
version-gated fallback that closes that hole, so it was removed rather than
tightened: `v0.1.0` through `v0.1.2` are no longer installable through
`install.sh`/`update.sh` (their assets remain on GitHub and can still be
downloaded and inspected manually). There is no environment variable or
operator flag that can bypass provenance for any release.
The development channel still requires both `SINGBOX_VPN_CHANNEL=dev` and
`SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1` and receives no provenance claim.

GitHub artifact attestations are the primary mechanism because releases already
build and publish entirely in GitHub Actions, the signer identity comes from
short-lived OIDC credentials, and verification can be restricted to this
repository and its release workflow. This avoids maintaining a long-lived
minisign/cosign private key or adding two parallel signing ecosystems. A
standalone signed manifest would be reasonable if releases move off GitHub, but
today it would add secret rotation and recovery work without removing trust in
the raw GitHub bootstrap or Actions workflow.

## Threat model

| Threat | Result | Reason |
|---|---|---|
| A. Network corruption | PREVENTS | HTTPS, SHA-256 verification, and digest-bound provenance reject changed release bytes. |
| B. GitHub CDN corruption | DETECTS | Every installable release's artifact must match both the checksum and repository attestation digest. Availability attacks remain possible. |
| C. Release binary changed without checksum | DETECTS | SHA256SUMS fails for every release; provenance also binds the expected artifact digest for every installable release. |
| D. Compromised GitHub account/repository | PARTIALLY MITIGATES | Editing release assets alone cannot mint an Actions attestation, but an attacker able to change trusted `main`, tags, workflows, or invoke privileged Actions may defeat the chain. The initial raw installer remains under this trust root. |
| E. Compromised GitHub Actions workflow | DOES NOT ADDRESS | The workflow can build malicious bytes and legitimately attest them. Branch/ruleset and Actions-permission controls are required outside the artifact format. |
| F. Compromised third-party dependency | PARTIALLY MITIGATES | `Cargo.lock`, `--locked`, `cargo audit`, tests, and provenance record what was built; none proves dependency source is benign. |
| G. Malicious dev branch | DOES NOT ADDRESS | Dev is deliberately mutable/unattested and requires two explicit unsafe opt-ins. |
| H. Local unprivileged attacker | PARTIALLY MITIGATES | Root-owned staging, restrictive modes, atomic replacement, locks, and checksum/provenance-before-extraction reduce local substitution opportunities; a root compromise is out of scope. |
| I. Rollback/downgrade | PREVENTS by default | Normal update refuses an older semantic version. Intentional rollback requires both an explicit `--version` and `--allow-downgrade`. State-schema compatibility remains the operator's responsibility. |

## Reproducibility and release contents

Rust is pinned by `rust-toolchain.toml`; Cargo dependency resolution is pinned
by the committed root `Cargo.lock`, and CI builds/tests with `--locked`.
`deploy/lib/versions.env` pins the upstream sing-box version and per-architecture
archive digests. Production release archives contain only `vpn-admin`,
`subscription`, `README.md`, and `LICENSE`; the separately published source
archive is made with `git archive` from the tagged commit. CI checks tag/package
version agreement and extracts the actual archive using installer expectations.

These controls make builds materially constrained, but **bit-for-bit
reproducibility is NOT VERIFIED**. gzip metadata, linker/compiler behavior,
runner images, native dependencies, and action implementations can affect
bytes. Provenance identifies the workflow/repository that produced an artifact;
it is not a reproducible-build proof.

## Operational requirements and repository settings

Code enforces least-privilege job permissions and requires `id-token: write`
and `attestations: write` only in release build jobs. Repository administrators
should separately configure rulesets/branch protection for `main` and release
tags, require the reusable CI gate before merge, restrict tag/release creation,
limit Actions to approved actions, require review for workflow changes, and
protect any release environment. These are repository-setting recommendations,
not facts verified or changed by this codebase audit.

Backups are master credential bundles. They contain live VLESS/Hysteria2
credentials, REALITY private material, token hashes, and TLS-related state.
Keep them root-only (`0600`), transfer them only over an authenticated encrypted
channel, store them in access-controlled encrypted storage, never attach them to
issues/chat/email, and securely delete superseded copies according to the
operator's retention policy. The archive format does not claim built-in
encryption at rest.

## Failure behavior

For every release, missing `gh`, a missing attestation, a repository-identity
mismatch, or an attestation digest mismatch aborts before extraction or live
update mutation. For every release, malformed/missing SHA256SUMS, missing
assets, checksum mismatch, wrong embedded version, or an unintended downgrade
aborts before mutation. Authentication does
not make compromise impossible: stable installation still begins by executing
`main/install.sh` as root and therefore trusts HTTPS, GitHub repository/account
controls, the referenced Actions workflow and actions, GitHub's OIDC/attestation
infrastructure, runner integrity, locked third-party dependencies, and the
operator's local root environment.
