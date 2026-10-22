# Corrected build installed on the staging node (NOT a published release)

- Source: commit `82df3d92a44dca501a26a3f89b4354d86b66b83d` (branch staging/2026-10-20-remediation), exported with `git archive`
  to /root/candidate/82df3d9 and synced byte-identically into /opt/singbox-vpn (verified by a per-file sha256 diff). The previous
  rc.2 tree is backed up in /root/backups/opt-singbox-vpn-rc2-tree-*.tgz.
- Built with: `update.sh --dev-rebuild` run from /opt/singbox-vpn (workspace tests incl. the real-core lab with the pinned
  aioquic 1.3.0 venv passed first; `cargo build --release --locked`). Toolchain pinned by rust-toolchain.toml; sing-box 1.14.1.
- Installed executables (sha256, first 16): vpn-admin b4ddc91cd8810187, vpn b4ddc91cd8810187, vpn-subscription-svc e19230faa9d1f4b3.
- Evidence: /root/evidence/candidate-82df3d9-devrebuild.log; first attempt (wrong source tree) kept as ...attempt1 / 4caffc7 log.
- Result on the node: IPv6 gateway neighbour REACHABLE, IPv6 ping OK, egress-isolation table uses the positive `meta skuid 995` form,
  doctor: 0 FAIL, REALITY and Hysteria2 loopback self-tests pass; `update.sh --repair` is refused (hash drift guard), node unchanged.

## What this is not
- Not a CI-built, attested release: the production updater only installs prebuilt, cosign-attested assets built in the pinned
  AlmaLinux 8.10 container (glibc baseline), which requires pushing a tag. That needs the owner (see owner actions).
- install-state.json still names v1.3.1-rc.2 (dev-rebuild does not rewrite the manifest). Doctor therefore reports a release-identity
  mismatch by design; with the repair guard this is safe, and it disappears when a real corrected release is installed with
  `update.sh --version <tag>`.

## The I-54 chain, end to end
1. Published rc.2 ships nftables-egress-isolation.sh with `meta skuid != UID accept` -> breaks IPv6 ND (proved again: after the
   interruption matrix updated the node to rc.2, `fe80::1 FAILED`, IPv6 100% loss).
2. Corrected tree: positive per-uid rejects; doctor FAILs the bad form (and the corrected updater's own acceptance check refused an
   install whose live table was still the bad one: attempt 2).
3. `--repair` could restore rc.2's script -> now refused unless the installed executables ARE the recorded release.
4. A real fix for rc.2 users needs a published corrected release; until then rc.2 -> any update with an older updater is unprotected.
