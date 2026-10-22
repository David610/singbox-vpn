# fin1 restoration plan (prepared 2026-10-22; NOTHING here is executed or authorized yet)

Each step names its owner input. Steps without one run autonomously once their predecessors are done. Every host change is
logged with its rollback in `docs/handoffs/2026-10-22/HOST_CHANGES.md` (a new file created at the first change).

## 0. Provider safety net (owner, about 1 minute, phone)
In the Hetzner Console app or website, open server fin1 → **Backups → Enable**. This gives daily full-disk backups kept by the
provider, which would have preserved the lost commits. It is independent of everything below and needs nothing from fin1. A manual **Snapshot** before step 4 gives an exact pre-install image.

## 1. SSH (existing tool: `deploy/lib/ssh-harden.sh`)
1. The owner sends one public key (e.g. Termius → Keychain → Generate key → ED25519 → share/copy the public key).
2. `ssh-harden.sh add-key --pubkey '…'`. Only the fingerprint is printed back for the owner to compare.
3. The owner opens a second Termius session with the key, keeping the current one open. Then `ssh-harden.sh verify` must show
   `Accepted publickey` with that fingerprint in sshd's own log.
4. Only then `ssh-harden.sh disable-passwords`. A third session confirms the key still works, and `ssh-harden.sh rollback`
   stays ready. The Hetzner web console (VNC) remains the out-of-band path. Root login is kept key-only rather than disabled, so the recovery path stays simple.
5. Firewall: comes from the installer (step 4), which resolves the real SSH port and refuses to activate otherwise.

## 2. Encrypted off-host deployment backup (no tool exists yet: to be built, test-first)
- **Encryption:** `age` to an owner-held **public** recipient. fin1 never holds the decryption key, so neither a stolen
  backup nor a compromised fin1 exposes old backups. The simplest recipient is the same ssh-ed25519 public key as step 1
  (age accepts SSH keys). Restore then needs that private key on the new host, temporarily. A separate dedicated key is
  stronger, but it is a second secret for the owner to keep.
- **Destination (owner chooses one):**
  - (a) a Hetzner Storage Box, ordered in the Hetzner console. fin1 gets an SFTP sub-account restricted to one directory.
  - (b) any SFTP/S3 bucket the owner controls.
  - Not GitHub: it is the source backup only.
- **Content:** `/etc/vpn` (deployment.toml, identities, REALITY/Hysteria2 material, subscription records), certificate
  keys, the install-state manifest and the monitor state. Logs, raw captures and build output are excluded. There is a
  manifest of paths and SHA-256 sums, and a plaintext copy never leaves fin1.
- **Verification:** each upload is downloaded back and its checksum compared, and the archive listing is checked against
  the manifest. A decryption test is possible only where the private key is present, so it is a restore drill on a disposable
  host (never on fin1), done with the owner.
- **Schedule:** after every identity or user change, plus daily, via a systemd timer.

## 3. Identities (owner approval required)
The originals are gone with the old disk, and no backup of them was ever verified. Proposal: create three new users with
the installer's own provisioning; the names are listed when approval is requested. Every old subscription and profile
stays dead, and the owner re-imports one new subscription link per user on the iPhone. Nothing is created until approved,
and the credentials are delivered only through the subscription URL (never in chat or Git).

## 4. Install (owner approval required)
- **Source:** the corrected tree, either the staging branch plus `recovery/2026-10-22` once CI is green, or a later
  merged ref. It is installed with `SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 --ref <exact sha>`.
  That is an explicitly unverified development install, recorded as such, and **never rc.2**, because rc.2 reintroduces I-54.
- **Build:** one source build into the shared, budgeted target from I-81, with `CARGO_BUILD_JOBS` bounded. The host has
  34 GB free against an 18 GB need.
- **Missing CI corrections:** the fixes reportedly in `e93f29d` are MISSING. The CI run on this branch shows which jobs
  still fail; fix those first, test-first. No attested rc.3 exists, so this host stays at DEVELOPMENT/staging level until
  a CI-built, signed release is published with owner approval.
- **DNS and certificate:** `preflight_check_hostname_resolves_here fin1.xn----8sba1cvaif4e.com` must match 62.238.46.190
  and 2a01:4f9:c014:13bd::1 before certbot runs. Only one certificate issuance is needed, well inside Let's Encrypt limits.
- **Acceptance (protocol-level, not "ports listen"):**
  - REALITY and Hysteria2 self-tests;
  - the subscription fetch;
  - doctor with zero unexplained failures;
  - I-54: `fe80::1` REACHABLE and outbound IPv6 working after install, update and `--repair`;
  - tunnel-to-host isolation;
  - `install-shorts-monitor.sh --check`, including the I-83 classifier contract.

## 5. Monitoring
Installed by `install-shorts-monitor.sh` from the installed tree, never by a hand copy. External alert delivery
(`SHORTS_ALERT_CMD`) still needs a destination; the owner picks one, e.g. a Telegram bot or an email relay. Until then
alerts are local only, and monitoring is not production-ready.

## 6. iPhone (owner, after step 4 passes)
1. **Migration:** delete the old fin1 profiles in Hiddify, import the one new subscription link, and confirm both REALITY and Hysteria2 connect.
2. **Minimal Shorts retest** (the old results describe the old install):
   - Hysteria2 + YouTube Incognito: 3 Shorts.
   - REALITY + Incognito: 3 Shorts.

   Report WORKS or FAILS for each.
3. **Only if Incognito still fails:** the synchronized A/B run. A = Hysteria2 signed-in (working), B = Hysteria2 Incognito
   (failing), with the same Short, device and network. `af_capture.py capture` runs for each window, the GO message is sent
   only after `status` shows the phone's tunnel live, and each window gets an `analyze` with its exact start and end.
   Equal IPv6 shares in A and B mean the address family is **not** the discriminator.
4. **No routing change** (hairpin, IPv6 reject, decoy, new VPS) without a separate, explained, owner-approved experiment.

## 7. Native acceptance
REALITY and Hysteria2, each signed-in and Incognito, plus a normal video, a subscription update, a reconnect, Wi-Fi/cellular
and a sustained session. Each is recorded as native-device evidence. Nothing unperformed is ever marked PASS.
