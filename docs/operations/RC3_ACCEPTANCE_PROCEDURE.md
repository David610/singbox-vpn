# v1.3.1-rc.3 acceptance procedure (what I run after R5; prepared 2026-10-21, NOT executed)

Scope: accept the **exact CI-built, cosign-attested rc.3 artifact**. Not a development build, not the working tree, and never the published
`v1.3.1-rc.2`: rc.2 is the known-broken starting point of the upgrade rehearsal (I-54) and is excluded from final production acceptance.
The R1-R6 authorizations are in `RC3_RELEASE_SEQUENCE.md`; this file is what happens once R5 has produced a release.

Every row names the evidence file and the gate in `docs/handoffs/<date>/GATES.json` it can move. A row that was not run is UNVERIFIED, never PASS.

## A. Artifact identity (no host needed)
| # | Check | Command | Pass |
|---|---|---|---|
| A1 | Release is the intended one | `gh release view v1.3.1-rc.3 --json tagName,targetCommitish,isPrerelease,assets` | tag `v1.3.1-rc.3`, prerelease, target = the merge commit the owner tagged (R4/R5) |
| A2 | Assets complete | the release lists, each with `.sha256` and `.sigstore.json`: `singbox-vpn-x86_64-unknown-linux-gnu.tar.gz`, `singbox-vpn-src.tar.gz`; plus `singbox-vpn-sbom.cdx.json` (+`.sha256`, `.sigstore.json`) and `singbox-vpn-licenses.json` (+`.sha256`) | all present (names from `release.yml`) |
| A3 | Checksums | `sha256sum -c` on each `.sha256` | all OK |
| A4 | Attestation (anonymous, as a fresh VPS does it: no `gh` login) | `cosign verify-blob-attestation --bundle <asset>.sigstore.json --certificate-identity https://github.com/David610/singbox-vpn/.github/workflows/release.yml@refs/tags/v1.3.1-rc.3 --certificate-oidc-issuer https://token.actions.githubusercontent.com --type https://slsa.dev/provenance/v1 <asset>` (exactly what `verify_release_attestation()` in `update.sh` runs; the updater refuses without it, there is no checksum-only fallback) | verifies for the binary archive, the source archive and the SBOM |
| A5 | Source = tag | extract `singbox-vpn-src.tar.gz`; compare with `git archive v1.3.1-rc.3` | identical; `Cargo.lock` unchanged; the release job's glibc-baseline check passed |
Evidence: `evidence/rc3/artifact-*.txt` -> gate ART-1.

## B. Upgrade rehearsal on the staging node (R6, owner-authorized; transactional, rollback proven by the matrix)
| # | Step | Pass |
|---|---|---|
| B0 | Snapshot first: `tar` of `/etc/vpn`, `/var/lib/singbox-vpn`, `/opt/singbox-vpn`; record `vpn doctor --protocol` and the nft egress table | files + hashes recorded |
| B1 | Put the node on the **published rc.2** (the real "users on rc.2" state). The node is on a dev build, so this is a downgrade rehearsal: `update.sh --version v1.3.1-rc.2 --allow-downgrade` | node reproduces I-54 (`meta skuid != 995 accept` present) - this proves the rehearsal starts from the defect |
| B2 | `update.sh --version v1.3.1-rc.3` from the installed tree (attested assets) | exit 0; `UPDATE COMPLETE`; no marker/staging/lock left |
| B3 | I-54 fixed by the artifact itself | `nft list table inet arcana_egress_isolation | grep -c 'skuid !='` = 0, positive `skuid <uid>` rules present; `ip -6 neigh` gateway not FAILED; IPv6 ping OK |
| B4 | Identity | `vpn doctor --protocol --require-protocol`: 0 FAIL, **no** "release identity INCONSISTENT" (binaries equal the recorded release), REALITY + Hysteria2 self-tests OK |
| B5 | Credentials/subscription unchanged | same user UUIDs; `client_profile_check.py` 103/103 against the served profiles; nginx/subscription health |
| B6 | Repair on a properly installed release | `update.sh --repair` succeeds **twice**, binaries unchanged, IPv6/isolation still correct (contrast: on the dev build it is *refused*; that refusal is correct and is not this test) |
| B7 | Idempotency | `update.sh --version v1.3.1-rc.3` again: no-op, exit 0 |
| B8 | Rollback | `update.sh --version <previous> --allow-downgrade` (or the transactional rollback), then forward again; creds and protocols intact after each |
| B9 | Interruption matrix on the real payload | `TARGET=v1.3.1-rc.3` via `deploy/lib/run-interrupt-matrix.sh`, two runs, 16/16 each, identical (the previous matrix used the published rc.2 payload) |
Evidence: `evidence/rc3/staging-*` -> gates LIF-3, LIF-1 (re-run on the real payload), NET-1.

## C. Fresh installation (needs a DISPOSABLE host - owner action)
The installer cannot be tested "fresh" over a live installation, and a container cannot honestly stand in (systemd, nginx, nftables, 443). Use the
existing harness: `docs/VPS_ACCEPTANCE_TEST.md` / `.github/workflows/vps-acceptance.yml` (`lifecycle-acceptance.sh`) on a new disposable VPS with its own DNS
name and **no production credentials**. To avoid consuming ACME issuances, the harness supports restoring a certificate snapshot (see its header); otherwise
one issuance for the disposable name is expected, not for `fin1...`.
| # | Pass |
|---|---|
| C1 | `install.sh --version v1.3.1-rc.3 --domain <disposable>` completes; scratch workspace cleaned, no orphan cosign downloads in /tmp (I-33) |
| C2 | doctor 0 FAIL, both protocols, subscription reachable, `ufw`/nft correct, IPv6 gateway OK |
| C3 | the lifecycle gate's stages (update, repair, interruption, rollback, reboot, egress policy 16c) end `LIFECYCLE GATE: PASS` |
Evidence: the harness log + `release-acceptance` record -> gates LIF-4, LIF-5 (reboot).

## D. After the artifact is accepted (not part of artifact acceptance)
Real Shorts (`EGRESS_ACCEPTANCE_RUNSHEET.md`), native clients (`IPHONE_ACCEPTANCE_PROCEDURE.md`), SSH key-only, I-60 decision. A release is production-ready
only when the computed gate report says READY; a stable `v1.3.1` additionally needs the device matrix (`docs/RELEASE.md`).

## Abort rules
Stop and do not continue to the next row on: a failed attestation (A4), any FAIL in doctor, a change in credentials, IPv6 gateway FAILED, an updater error that
leaves a marker, or any test marked FLAKY in the CI run for this tag (`FLAKY-POLICY.md`: a flaky mandatory gate blocks; a prerelease used only to obtain the
attested artifact may proceed with an explicit, recorded owner waiver).
