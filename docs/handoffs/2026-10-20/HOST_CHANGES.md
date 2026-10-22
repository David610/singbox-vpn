# Persistent host changes on 62.238.46.190 (staging) — with rollback

Source of truth: `/root/host-changes/CHANGES.log` on the host, copied verbatim below (UTC). Secrets are not recorded.

```
2026-10-20T07:35:37Z created /root/host-changes (change log)
2026-10-20T07:35:45Z install: rustup (toolchain 1.94.1) in ~/.rustup ~/.cargo; rollback: rustup self uninstall
2026-10-20T07:35:45Z install: python venv /root/venv-probe + playwright + chromium (PLAYWRIGHT_BROWSERS_PATH=/root/pw-browsers); rollback: rm -rf /root/venv-probe /root/pw-browsers
2026-10-20T07:36:07Z apt install python3.14-venv build-essential pkg-config libssl-dev git curl (rollback: apt remove)
2026-10-20T08:07:04Z STAGING INSTALL of singbox-vpn from branch HEAD (user-authorized 2026-10-20); rollback: bash deploy/almalinux/uninstall.sh --yes; pre-install state /root/backup-pre-install
2026-10-20T08:34:51Z singbox-vpn INSTALLED on staging (branch HEAD 98de537); ufw enabled (OpenSSH,443/tcp,443/udp,8443/tcp); nginx+certbot packages; LE cert for fin1; services sing-box vpn-subscription nginx + timers. Rollback: bash /opt/singbox-vpn/deploy/almalinux/uninstall.sh --yes (restores ufw to inactive)
2026-10-20T08:44:17Z ufw: replaced 'allow OpenSSH' with 'limit OpenSSH' (brute-force rate limit); rollback: ufw delete limit OpenSSH && ufw allow OpenSSH; backups /root/backup-pre-install/ufw.user*.rules.before-limit
2026-10-20T09:46:08Z /root/venv-tests (pip: deploy/lib/tests/requirements-egress-lab.txt) for the egress lab Rust test; rollback rm -rf
2026-10-20T09:47:29Z lifecycle update test: rsync repo -> /opt/singbox-vpn, update.sh --dev-rebuild (with injected failure first); rollback: re-run install.sh
2026-10-20T10:27:51Z release lifecycle test: uninstall dev install, install published v1.3.0 (then update to v1.3.1-rc.2); rollback: uninstall.sh --yes; ufw LIMIT SSH rule will need re-applying
2026-10-20T10:47:57Z ufw: re-applied 'limit OpenSSH' after release-lifecycle reinstall; staging now runs published v1.3.1-rc.2 binaries; update.sh --repair executed from the branch tree
2026-10-20T10:52:24Z nft: applied branch nftables-egress-isolation.sh (uid-scoped rejects) to the live table inet arcana_egress_isolation; before-state saved in /root/evidence/arcana-egress-table.before.nft; rollback: nft -f that file (restores the ND-breaking original) or vpn-egress-isolation.service restart from the installed tree
2026-10-20T11:11:31Z interrupted-update test (kill -9 of update.sh --repair mid-switch, then recovery run); rollback: none needed unless doctor fails; re-run update.sh --repair
2026-10-20T11:12:48Z removed interrupted-update marker after verifying health and absence of prev tree (copy /root/evidence/int-marker.json)
2026-10-20T11:12:59Z removed stale /opt/.singbox-vpn-prev-* and /opt/.singbox-vpn-update-staging.* named by the interrupted-update marker
```

Also present, not in the log: `/root/venv-probe`, `/root/pw-browsers`, `/root/venv-tests`, `/root/evidence`, `/root/qa-secrets` (0700; QA subscription URLs — delete when done), `/root/backup-pre-install`. Dependency/Rust/Chromium installs: rollback `rustup self uninstall`; `rm -rf` the venv/browser dirs; `apt remove` the packages listed.

Current staging state: published v1.3.1-rc.2 binaries; update.sh/ nftables-egress-isolation.sh from this branch applied; ufw active with `limit OpenSSH`; SSH password login STILL ENABLED.

2026-10-21T13:xx Z `chmod 700 /root/backups; find /root/backups -type f -exec chmod go-rwx {} +; chmod 600 /root/qa-secrets/body.tmp` - secret-bearing backup archives were 0644 (default umask); tightening only, no rollback needed. See docs/handoffs/2026-10-21/CREDENTIAL-AUDIT.md.
