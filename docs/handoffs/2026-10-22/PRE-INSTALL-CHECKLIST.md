# fin1 pre-install checklist and production-readiness report

Updated 2026-10-22. Each status is PASS, FAIL, BLOCKED or UNVERIFIED, and every PASS names its evidence. Code-level PASSes come from CI
and the hermetic tests; they are not deployment, device or Shorts evidence.

## A. Owner actions, in order (nothing is installed until all of A is done)
| # | Action (iPhone) | Status |
|---|---|---|
| A1 | Send the `fin1-owner` **login** public key (Termius → Keychain → Generate Key → ED25519) | BLOCKED (owner) |
| A2 | Open a second Termius session with that key, then confirm. Password login is disabled only after `ssh-harden.sh verify` sees `Accepted publickey`. | BLOCKED on A1 |
| A3 | Confirm the 5 successful root password logins since the rebuild were you: 78.109.69.17 (08:42, 08:44, 09:10 UTC) and 185.194.184.53 (13:07, 13:58 UTC) | BLOCKED (owner) |
| A4 | Hetzner console: enable **Backups** on fin1, then **Snapshot** `pre-install-clean` | BLOCKED (owner) |
| A5 | Send the `fin1-backup` **public** key (a second ED25519 key), and keep its private key outside the phone (password manager or other secure storage) | BLOCKED (owner) |
| A6 | Order a Storage Box. Enable SSH and **snapshots** (e.g. daily, 14 kept). Create a sub-account with base dir `fin1-backups`. Send its hostname and username. | BLOCKED (owner) |
| A7 | Reply **"approve install + identities"**, which covers sections C and D below | BLOCKED (owner) |

After A6 I create the upload key on fin1 and send you its **public** half for the sub-account. I then pin the Storage Box host key
against Hetzner's published fingerprint and run a first real `state-backup.sh run` on a dummy file set. That backup **counts as working
only after it uploads and verifies**.

## B. Installation source review (exact revision: the recovery-branch HEAD at install time, CI green)
| Area | Finding | Status |
|---|---|---|
| Source | `recovery/2026-10-22` at an exact SHA with full CI green (last green: `92ba6a4`, run 39812810442, 22/22) | PASS (code) |
| Provenance | A source install is `SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 --ref <sha>`, an **unverified development install**. It is not an attested release, and no corrected attested release exists. rc.2 is never used (I-54). | Recorded |
| I-54 | `nftables-egress-isolation.sh` scopes every reject with a **positive** `meta skuid` match (lines 87–94), and doctor FAILs the old negated form (`30ed91d`) | PASS (code); host check after install |
| I-81 | The qualification build and now the installer/updater source builds run a disk and inode budget with bounded jobs (`92ba6a4`). Free: 34 GB against an 18 GB need. | PASS (code) |
| I-83 | The monitor classifier and installer contract check (`f75b863`, `2d0da8d`). `install-shorts-monitor.sh` refuses a reverted classifier. | PASS (code); install after VPN |
| REALITY/Hysteria2 | 12 real sing-box interop tests in CI (`singbox-validate`, manifest-checked) | PASS (code) |
| Hairpin simulation | First CI execution ever, now green (`1ed9693`); not used on fin1 (no hairpin authorized) | PASS (code) |
| Ubuntu 22.04 | `ubuntu:22.04` is in CI's os-matrix (green); `deploy/lib/os.sh` uses the ufw backend for Ubuntu | PASS (code) |
| SSH and firewall | The installer resolves the real SSH port with `preflight_resolve_ssh_port` and fails closed without it | PASS (code); verify after install |
| DNS | `fin1.xn----8sba1cvaif4e.com` → 62.238.46.190; no AAAA record. The installer checks it before certbot. | PASS (now); recheck at install |
| TLS | One certbot issuance for the domain (well inside Let's Encrypt limits) | UNVERIFIED until install |
| Secrets in output | The installer creates the first user (`default`) and normally **prints its subscription URL and QR**. Run with `SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1`, so the session only shows "credential generated: yes". | Mitigated |
| Binary hashes | Recorded after the build: `sha256sum /usr/local/bin/{vpn-admin,vpn,vpn-subscription-svc}` plus the sing-box pin from `deploy/lib/versions.env` | after install |

## C. Identity replacement plan (needs A7; nothing is created before)
All old profiles and subscriptions are **invalid**, because their keys were destroyed with the old disk. In Hiddify, delete every old fin1 profile.
| Old user | Replacement | How | Link delivered |
|---|---|---|---|
| `default` | new `default` | created by the installer's stage 16 (suppressed output) | `vpn user list` then `vpn user rotate-token <ID of default> --qr` (it takes the ID, not the name), run **by you** in Termius, so the QR and URL show only on your screen |
| `arcana-selftest` | new reserved self-test principal | managed by the installer and updater (`RESERVED_SELFTEST_USER_NAME`); never given to a person | none |
| `iphone-test` | new `iphone-test` | `vpn user create --name iphone-test --qr`, run **by you** in Termius | on your screen only |

The credentials live in `/etc/vpn/compat` with the installer's permissions (root:vpn-compat 02750 dirs, 0600/0640 files). The first encrypted backup
runs right after (the `.path` unit fires on `users.json` changes). Nothing goes to Git, chat or logs.

## D. Install and verification sequence (after A1–A7)
1. Recheck the snapshot, SSH key-only login, disk, DNS and the CI-green SHA. Write a `HOST_CHANGES.md` entry.
2. Install: `SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 bash install.sh --ref <sha> --domain fin1.xn----8sba1cvaif4e.com --non-interactive`,
   run in a persistent `systemd-run` unit with its log and exit status recorded, and the output redacted before anything is saved.
3. Verify each item, with protocol-level evidence (not "a port is listening"):
   - doctor reports 0 unexplained FAIL;
   - `vpn-admin doctor --protocol` passes a real REALITY handshake and real Hysteria2 authentication;
   - the subscription fetch works;
   - IPv4 and IPv6 egress work, and `fe80::1` is REACHABLE (I-54);
   - tunnel-to-host isolation holds (`c16_isolation_check.py`);
   - ufw has SSH open and nothing unexpected;
   - the certificate is valid;
   - after `systemctl restart` of each unit and one reboot, everything is healthy again;
   - `install-shorts-monitor.sh --check`, including the I-83 contract;
   - the first `state-backup.sh run` prints VERIFIED off-host.
4. Record the binary hashes and evidence under `docs/handoffs/2026-10-22/evidence/`, privacy-minimized, then commit and push.

## E. iPhone after install (minimal)
1. Delete the old fin1 profiles in Hiddify. In Termius, run `vpn user list`, then `vpn user rotate-token <ID> --qr` for `default`, then copy the URL into Hiddify (+ → paste from clipboard).
2. Connect on REALITY, then on Hysteria2: confirm each loads a website.
3. YouTube **Incognito** → 3 Shorts on Hysteria2, then 3 on REALITY. Report WORKS or FAILS for each.
4. If Incognito fails: run the planned synchronized test, Hysteria2 signed-in (A) against Incognito (B), with `af_capture.py`. GO is given only after `status` shows the phone live.

## F. Production-readiness gates (current)
| Gate | Status | Evidence / reason |
|---|---|---|
| Source qualification and CI | PASS | run 39812810442 (22/22) at `92ba6a4201f03f723f92c7553e0ecc16c264177c` |
| Attested release artifact | FAIL | none exists for the corrected source |
| Installation | BLOCKED | A1–A7 |
| Upgrade / repair / rollback on the new host | UNVERIFIED | historical 16/16 matrix results were for the old host |
| IPv4/IPv6, I-54, DNS | UNVERIFIED | no install |
| Tunnel isolation | UNVERIFIED | no install |
| REALITY / Hysteria2 (host) | UNVERIFIED | CI interop only |
| Subscriptions / native clients | UNVERIFIED | no install |
| Signed-in Shorts | UNVERIFIED | the pre-rebuild native result (WORKS) is from the old install |
| Anonymous Shorts | FAIL (historical) / UNVERIFIED (new host) | the pre-rebuild native result FAILED on both protocols |
| Monitoring and alerts | BLOCKED | no install; no external alert destination chosen |
| SSH and security | FAIL | password root login is open, with over 3,300 failed attempts (A1–A3) |
| Backup durability | BLOCKED | A5, A6; the tool is tested (30/30), but no real off-host backup exists |
| Disk-capacity protection | PASS (code) | I-81 tests; 34 GB free |
| Reboot and recovery | UNVERIFIED | no install |
| Performance | UNVERIFIED | no install |
