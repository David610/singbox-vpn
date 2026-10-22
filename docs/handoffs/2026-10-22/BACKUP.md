# Encrypted deployment-state backup (fin1)

Tool: `deploy/lib/backup/state-backup.sh`. Test: `deploy/lib/tests/test-state-backup.sh`, which uses real `age` and runs in CI.
**Nothing has been backed up yet**, because no deployment state exists: the VPN is not installed and the original
identities were lost before any backup existed. Hetzner Backups enabled now cannot bring those back.

## What is protected
`/etc/vpn` covers deployment.toml, user records, REALITY keys, Hysteria2 and obfs passwords, subscription tokens and the
monitor env. `/etc/letsencrypt` holds the certificate and its key. `/var/lib/singbox-vpn` holds the install-state manifest and runtime state.
These are tarred and streamed straight into `age`, so plaintext never touches disk. The archive embeds a per-file SHA-256 manifest.

## Keys (owner-controlled; fin1 holds only PUBLIC material)
| Purpose | Generated where | Private half lives | Public half goes |
|---|---|---|---|
| Owner SSH login | Termius on the iPhone | Termius vault | `/root/.ssh/authorized_keys` via `ssh-harden.sh` |
| **Backup decryption** (dedicated, separate from login) | Termius on the iPhone, as a second ED25519 key named `fin1-backup` | Termius vault, plus an exported copy in an independent password manager or iCloud Keychain note | `/etc/vpn/backup-recipients.txt` |
| Storage upload | fin1 (`/root/.ssh/fin1_backup_upload`) | fin1 only. It can write to the backup directory only and cannot decrypt anything | the Storage Box sub-account's authorized keys |

age accepts an `ssh-ed25519` public key as a recipient. The test proves that encryption to it and a restore with its private key work end to end. Using a dedicated key means losing or rotating the login key never strands the backups.
**The owner must keep the backup key's private half outside the iPhone too.** A lost phone without a Termius sync or an
exported copy means the backups cannot be decrypted.

## Destination
Recommended: a Hetzner **Storage Box**, ordered in the Hetzner console. Set it up as follows:
- Enable SSH support.
- Create a **sub-account** whose base directory is `fin1-backups`, so fin1's key can reach nothing else.
- fin1 connects over SFTP on port 23 with `StrictHostKeyChecking=yes`, against a pinned host key in `/etc/vpn/backup-known_hosts`.
- The fingerprint is compared against the one Hetzner publishes for Storage Boxes before it is trusted.

Config is kept in `/etc/vpn/backup.env` (mode 0600, no secrets): `BACKUP_SFTP_TARGET=<sub-account>@<box>.your-storagebox.de:fin1-backups`,
`BACKUP_SFTP_PORT=23`, `BACKUP_SFTP_KEY=/root/.ssh/fin1_backup_upload`, `BACKUP_SFTP_KNOWN_HOSTS=/etc/vpn/backup-known_hosts`.

## Each backup run (`state-backup.sh run`, via systemd units in `deploy/lib/backup/`)
1. `create` writes the encrypted archive (mode 0600) and its `.sha256` to `/var/backups/singbox-vpn`.
2. `upload` sends both, downloads the archive back and compares SHA-256. `VERIFIED off-host` is printed only on a match.
   A failed upload fails the unit, so it shows in `systemctl --failed`, and pruning is skipped.
3. `prune` keeps the 3 newest **local** archives (`BACKUP_KEEP_LOCAL`). fin1 never deletes remote copies.

Triggers: `vpn-state-backup.timer` runs daily (persistent, so a missed run happens at the next boot), and `vpn-state-backup.path`
runs after every change to `/etc/vpn/compat/users/users.json`, which covers user creation, removal and rotation. The units are installed and
enabled only after `backup.env`, the recipient and the pinned host key exist. The first run is watched by hand to completion.

`/etc/vpn/backup.env` is sourced as root, so the tool refuses it unless it is owned by root and is neither group- nor world-writable. The sftp
target must be `user@host:dir` with plain characters and no `..`.

**Known limit:** fin1's upload key can also delete or overwrite remote files. An attacker who gains root on fin1 could wipe
the off-host copies. The protection must live where fin1 cannot reach it: enable **Storage Box snapshots**
(automatic, e.g. daily with 14 kept) in the Hetzner console. The sub-account cannot touch snapshots.

## Restore (proven by the restore drill; never tested on fin1 itself)
1. On the new host, download the newest `*.tar.age` and `.sha256` from the Storage Box. Then run `state-backup.sh verify FILE`.
2. Put the `fin1-backup` private key there temporarily, as a 0600 file pasted from Termius.
3. Run `state-backup.sh restore-drill FILE KEY /root/restore-scratch`, which decrypts and checks every file against the manifest.
4. Shred the key file. It is never left on the host.
5. Copying from the scratch dir into `/etc/vpn` and the other live paths is an explicit, owner-approved step. It is followed by the installer's
   repair/doctor and protocol self-tests, and the old subscriptions keep working only if those pass.

## Before the install: manual snapshot
In the Hetzner console: fin1 → Snapshots → Create snapshot, named `pre-install-2026-10-XX`. It is an exact image to roll back to.
Snapshots and Hetzner Backups are provider-side, unencrypted disk images. They complement this off-host encrypted copy, not replace it.
