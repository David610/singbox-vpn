# VPS access and host-trust handoff — fin1 (62.238.46.190, fin1.xn----8sba1cvaif4e.com)

Dated record, 2026-10-23. **Nothing here has been run on the host.** No production connection succeeded in this session.

> ## Status update (2026-10-23, later) - read this first
>
> The sections below are the original handoff, kept as written. Several statements in it are now superseded:
>
> | Original statement | Now |
> |---|---|
> | "Nothing here has been run on the host. No production connection succeeded." | Superseded. The agent session itself runs **on this host**, so the earlier "SSH attempts" were the machine connecting to itself, and the integration tests of 2026-10-23 ran here (they were later removed; see `docs/qualification/2026-10-23/README.md`, corrections log). |
> | "The staging key is not in `authorized_keys`" (section 1) | Superseded. The owner installed it; key login works. The owner's **own** key is still not installed. |
> | "Trust prerequisite: were the five root logins yours?" (section 3) | **Answered.** The owner stated on 2026-10-23 that all five were theirs and authorized (user-reported). This resolves login attribution only. It does not prove ownership of the source networks or that the host is secure. |
> | "Server qualified = BLOCKED" (end of section 5) | Still BLOCKED: the VPN is not installed and no exact artifact exists. |
>
> Remaining before any install: owner's own public key installed and a key login proven from a **second** session; provider-console recovery confirmed; then password login disabled
> (`deploy/lib/ssh-harden.sh verify` first), with the rollback in section 7 below. Host-specific findings are kept in private evidence, not in this repository.


## 1. What is missing, precisely

| Fact | Evidence |
|---|---|
| The server answers on 22/tcp and offers **publickey and password** | `Permission denied (publickey,password)` for `root@62.238.46.190` (two attempts: the staging key, and default auth). No further attempts were made. |
| The session holds one key: ED25519 `SHA256:mfcHitVEwnl27Bf2LUcjLnb/Yx1WAS4g5L3/MjyEodc`, comment `fin1 singbox-vpn staging deploy key 2026-10-22` (public half is in `~/.ssh/singbox_staging_deploy.pub` of the agent environment; the private half never leaves it) | `ssh-keygen -lf` |
| That key is **not in the host's `authorized_keys`** for root (or the host was rebuilt/changed since) | the server refused it |
| The session has no root password and none was requested | — |

So the missing item is exactly one of: (a) an authorized key for some account, or (b) a password. This repository's policy
prefers (a): key-only, least privilege, no password in chat.

## 2. Smallest action from the owner to grant access (provider console, ~5 minutes)

1. Open the provider (Hetzner Cloud) console for the server and use the web console / rescue-free console login as root.
2. Append the public key (a single line, public half only) to `/root/.ssh/authorized_keys`, mode 0600, dir 0700:
   `ssh-ed25519 AAAA… fin1 singbox-vpn staging deploy key 2026-10-22`
   (Ask for the line; it is public. If you prefer a fresh key, generate one and send only the `.pub`.)
3. Say "access granted". The session will then run only the read-only checks in §4, in order, and stop.

A **restricted temporary account is feasible and preferred over root**: create `qa` with `useradd -m -s /bin/bash qa`, put the
same public key in `/home/qa/.ssh/authorized_keys`, and grant only what §4 needs via a sudoers drop-in limited to the commands
listed there (`visudo -f /etc/sudoers.d/qa-readonly`). Remove the account, its sudoers file and the key afterwards. This keeps
root key-only access out of scope until the trust decision (§3) is made.

Minimum permission for the read-only phase (§4): read `/var/log/auth.log` (or `journalctl -u ssh`), `ss -tulpn`, `nft list ruleset`,
`ufw status`, `systemctl status`, `sshd -T`, `df`, and `last -ai`. No write access. The install phase additionally needs root.

## 3. The trust prerequisite (issue #167 / I-86)

The owner message "the VPS is mine and it is safe" is recorded as the owner's assertion. It does **not** by itself answer the
open question, which is narrow and checkable: **were these five successful root password logins yours?**

| When (UTC, 2026-10-22) | Source |
|---|---|
| 08:42, 08:44, 09:10 | 78.109.69.17 |
| 13:07, 13:58 | 185.194.184.53 |

- If **all five were you** (for example your phone/mobile carrier and a second network): reply "A3 confirmed: all five were mine".
  That resolves the unexplained-login blocker. It does not fix the still-open exposure: root password login enabled and
  thousands of failed attempts (PRE-INSTALL-CHECKLIST A1–A3).
- If **any is unknown**: treat the host as untrusted. Do not install on it. Take a provider snapshot, preserve the logs (§4.1), and
  rebuild from a clean image with key-only root, then restore only data you can verify (RESTORATION-PLAN.md).

Independent check without trusting the host's own claims: the provider console's *Access logs / Actions* and your phone's
carrier/Wi-Fi public IP history for those timestamps; compare with the two addresses above.

## 4. Safe host-trust checks, in order (all read-only)

Run from the provider console or the restricted account, ideally after a snapshot.

1. **Evidence capture** (changes nothing but creates one root-only directory):
   `sudo bash deploy/lib/ssh-auth-evidence.sh --since '2026-10-22 00:00'`. Output on stdout is redacted and safe to paste in #167; the
   raw directory stays on the host until copied off over an encrypted channel.
2. Identity: `hostname -f`, `cat /etc/os-release`, `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` (compare with the
   host key fingerprint you saw when you first logged in after the rebuild), `uptime`.
3. Persistence review: `last -ai | head -40`, `lastb | head` (count only), `cat /root/.ssh/authorized_keys` (expect empty or only
   your keys — **no unknown keys**), `ls -la /root /etc/cron.* /etc/systemd/system | head -80`, `systemctl list-units --type=service --state=running`,
   `ss -tulpn` (expect only sshd until install), `find / -xdev -newermt 2026-10-22 -type f -perm -4000 2>/dev/null | head`.
4. Pre-install readiness: PRE-INSTALL-CHECKLIST.md sections A–D (key-only SSH proven with `deploy/lib/ssh-harden.sh verify`
   *before* disabling password login — never the other way round; backups A4–A6; install identities A7).

Stop at the first unexplained finding and report it.

## 5. After trust is established: exact qualification sequence

Only after A3 is confirmed (or a clean rebuild), A1/A2 (key login proven, password login off), A4 (snapshot) and A5/A6 (off-host
encrypted backup target) are done, and the release candidate exists (RELEASE-CANDIDATE.md):

1. Install the **exact candidate** (tag + commit + sha256) by the documented installer; record `vpn status` and the binary digest.
2. `vpn doctor` and `vpn doctor --protocol` for both transports (REALITY and Hysteria2).
3. `bash deploy/lib/tests/` is **not** run on the production node; use the clean-host runner only if documented
   (`deploy/lib/run-shell-tests-clean-host.sh`).
4. First real backup: `state-backup.sh run`; then a **restore test** into a scratch directory/VM; the backup counts only after it
   uploads and verifies.
5. Reboot test, service-kill and interrupted-update recovery (docs/operations/RECOVERY_AND_ROLLBACK.md).
6. External probe from another network: `python3 deploy/lib/external_probe.py --host fin1.xn----8sba1cvaif4e.com --expect-ip 62.238.46.190 …`
   (see docs/operations/ACCESS_FIREWALL_REBOOT_EXTERNAL_RUNBOOK.md for the exact arguments).
7. Then, and only then, the native matrix: docs/operations/IPHONE_SHORTS_MATRIX_RUNBOOK.md.

Each step produces evidence stored under `docs/release-acceptance/<tag>.evidence.json` via `deploy/lib/release_evidence.py`.
Until then: server qualified = **BLOCKED**, device verified = **BLOCKED**.

## 7. Rollback for SSH hardening (added 2026-10-23)

Do this only after a key login is proven from a second session and the provider console is confirmed to work.

1. Keep one root session open throughout; never close it until the new session is verified.
2. Before editing, copy the config: `cp -a /etc/ssh/sshd_config.d/50-cloud-init.conf /root/50-cloud-init.conf.bak` and `cp -a /etc/ssh/sshd_config /root/sshd_config.bak`.
3. Validate before reload: `sshd -t` must print nothing. Reload (not restart): `systemctl reload ssh`.
4. In a new terminal, log in with the key. If it fails, in the still-open session restore the backups and `systemctl reload ssh`.
5. If every SSH session is lost: use the provider console (web console login as root) to restore the two backup files and run `systemctl reload ssh`.
6. Recovery path to verify beforehand, not after: the provider console can open a root shell on this server without SSH.
