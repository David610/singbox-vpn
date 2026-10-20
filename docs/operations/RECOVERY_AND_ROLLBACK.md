# Rollback and recovery guide (staging-verified items marked)

Every row: detection -> diagnosis -> safe recovery -> escalation. Evidence class
in brackets. Nothing here changes DNS, credentials or other nodes.

| Incident | Detect | Diagnose | Safe recovery | Escalate when |
|---|---|---|---|---|
| Fresh install fails midway | installer exits nonzero, prints rollback | `/var/log` + installer output; `vpn-admin doctor` | installer rolls itself back for a fresh install and restores ufw/packages/accounts it created [staging-verified 2026-10-20, run 1]; fix cause, re-run | rollback leaves residue (`uninstall.sh --yes`) |
| Re-install over a working node | n/a (operator action) | fingerprint config/users/keys/cert/subscription before and after | `install.sh` with the same flags is idempotent: state and tokens identical [staging-verified] | any fingerprint differs |
| Failed update | `update.sh` nonzero | `/etc/vpn/backups/<ts>`, update log | transactional rollback restores binaries, units, tree, config; users.json/keys are never rewound [rollback with injected failure: see lifecycle result in the handoff] | `UPDATE FAILED - ROLLBACK ALSO FAILED` (I-32 history) |
| Update killed mid-transaction | `update.sh` refuses: "an update transaction did not finish cleanly" | host health: services + `vpn-admin doctor`; marker names `backup_dir`, `prev_opt_dir`, `staged_src_dir` | healthy host: `deploy/lib/recover-interrupted-update.sh` (dry run), then `--yes`, then re-run the update [staging-verified: kill -9 after SWITCHING left state identical; recovery then repair OK]. Unhealthy host: restore from `backup_dir` as the error text describes | services down or doctor FAIL |
| Bad config render | `sing-box check` fails in `render-config` | doctor L4 drift lines | renderer keeps the previous live config; fix the input (deployment.toml/users) and re-render. Never hand-edit a rendered config (INV-11) | repeated render failure |
| sing-box start-limit outage | `systemctl is-failed sing-box`, doctor | `journalctl -u sing-box`, restart count | `systemctl reset-failed sing-box && systemctl start sing-box`; avoid bursts of user operations (I-10/I-11) | > 3 restarts / 10 min |
| Certificate expiry/mismatch | doctor L2, `subscription_acceptance.py` | SAN vs `public_host` | `certbot renew --dry-run`, then real renew; hostname must equal certificate SAN and subscription host | < 7 days left |
| Shorts stop playing on the exit | `deploy/lib/monitoring/shorts_health.py` DEGRADED/CRITICAL | probe JSON: refused signature (`OK` + clock stuck, bot wall) | owner decision (hairpin target/new egress). **Never** fall back to `direct` for the Google set (INV-6). `google-egress-hairpin clear` returns to the plain exit and Shorts fail | CRITICAL |
| Locked out of SSH | cannot log in | provider console | use the provider console; `ssh-harden.sh rollback` restores password login; ufw LIMIT rule removal: `ufw delete limit OpenSSH && ufw allow OpenSSH` | - |
| Hiddify iOS "no such file" | user report | device evidence per `docs/clients/HIDDIFY_IOS.md` | client-side: delete/reinstall app, re-import profile; no server action helps | recurs after reinstall |

## Host-level rollback of this staging environment
Every persistent change is listed with its rollback in
`docs/handoffs/2026-10-20/HOST_CHANGES.md`. Full removal of the VPN:
`bash /opt/singbox-vpn/deploy/almalinux/uninstall.sh --yes` (restores ufw to its
prior inactive state, removes accounts/packages it created).

## Recurring failure mechanisms and how they are now prevented
1. **Success summaries above FAIL lines** (acceptance-test.sh): verdict now
   counts non-required failures; `ACCEPTANCE_STRICT=1` fails them.
2. **A test that cannot fail**: injected-failure tests must prove the injection
   fired (log line), or they are INVALID.
3. **Tests reading live host state**: leaked groups, installed paths, real
   sshd/ufw. Use the hermetic runner; keep tests off the live host.
4. **Template fixes that never reach nodes**: nginx vhost only changes via
   installer re-run (I-45).
5. **Counting the wrong thing as success**: Shorts probe counts only genuine
   Shorts pages.
