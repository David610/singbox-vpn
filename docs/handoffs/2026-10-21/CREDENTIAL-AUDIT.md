# Credential audit (2026-10-21, staging 62.238.46.190) - no secret value appears anywhere in this file

Method: collect the node's live secret values from their restricted files (REALITY private key, Hysteria2 obfuscation password, every user's UUID and
Hysteria2 password in `users.json`, the QA subscription URL parts), then search for them. Only labels and lengths are ever printed
(`deploy/lib/scan_known_secrets.py`; the same collector drove the host sweep below).

| Surface | Result |
|---|---|
| Commits to be pushed (`origin/main..HEAD`, 85 commits) and the HEAD tree | **clean** (R1 preflight) |
| journald (this boot, all units) | clean |
| `/var/log/*` incl. auth.log, syslog, ufw.log, dpkg.log, cloud-init, kern.log | clean |
| nginx access and error logs | clean; the subscription vhost uses `access_log off`, a token-safe error log exists |
| `~/.bash_history` | clean |
| `/tmp`, `/var/tmp`, `/root/evidence` (text files < 3 MB) | clean |
| Evidence copied into the repo (`docs/handoffs/**`) | clean (covered by the tree scan) |
| `/root/backups` | holds the live credentials **by design** (archives of `/etc/vpn`); permissions were too loose, see below |
| sing-box log level | `fatal` (no per-connection data) |
| Runtime secret files | `private.key` 0640 root:sing-box; `hysteria_obfs_password.txt`, `users.json` 0640 root:vpn-subscription; `key.pem` 0640 root:sing-box; `config.json` 0640 root:sing-box; `shorts-health.env` 0640 root:root |

## Finding and fix (host change, reversible)
`/root/backups/*.tgz` (including `etc-vpn-pre-268808d-*.tgz`) and `/root/qa-secrets/body.tmp` were mode 0644, `/root/backups` 0755. They are inside `/root`
(0700) so no other local user could reach them, but a secret-bearing archive should not rely on its parent directory. Cause: backups taken with the
default umask during this session. Fixed: `chmod 700 /root/backups`, `chmod go-rwx` on every file under it, `chmod 600 /root/qa-secrets/body.tmp`.
Verified: 0 group/world-readable files remain under `/root/backups`. Rule going forward: take backups of `/etc/vpn` or `/var/lib/singbox-vpn` under `umask 077`.
Rollback: not needed (tightening only).

## NOT audited (stays UNVERIFIED/BLOCKED)
* GitHub Actions logs, uploaded artifacts and the release archives of rc.3: they do not exist until CI runs (R1) and a tag is built (R5). The release job must
  be checked for secret leakage in its logs and in the SBOM/provenance bundles.
* Provider snapshots/backups, anything outside this VPS, and the owner's own copies of the QA subscription URLs.
* Whether any credential was exposed before this session (an incident review is out of scope; rotating is the only safe response if in doubt).

## Safe eventual cleanup (do not run while QA is in progress)
1. After the iPhone acceptance: `vpn user remove <qa-user-id>` (invalidates the QA subscription), then `rm -rf /root/qa-secrets`.
2. Keep exactly one verified backup per purpose; delete the rest with `shred -u` (the archives of earlier `/etc/vpn` states carry credentials that are still valid today):
   `/root/backups/etc-vpn-pre-*`, `var-lib-pre-*`, and the `opt-singbox-vpn-*.tgz` trees (they are source, not secrets, but contain old installer state).
3. If any credential is ever believed exposed: rotate REALITY keys / Hysteria2 obfuscation password and each user's credentials (this restarts sing-box and requires re-import).
