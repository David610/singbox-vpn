# Access hardening, firewall, reboot and external-network acceptance — runbook

Order matters. Nothing here disables the only working login method. Every step lists
what to expect, how to undo it, and which evidence file it produces. Steps marked
**[OWNER]** need your hands (a second machine, a phone, the provider console).

## 0. Safety net (do this first)
* **[OWNER]** Confirm you can reach the provider console (Hetzner Cloud console / rescue) for 62.238.46.190. It is the recovery route if SSH is lost.
* Keep one root SSH session open for the whole of step 1 and do not close it.
* Current SSH state (read-only): `bash deploy/lib/ssh-harden.sh status` -> expect `passwordauthentication=yes`, `authorized public keys: 0`. SSH is already rate-limited by ufw (`LIMIT`).

## 1. SSH key migration (password login stays ON until proof exists)
1. **[OWNER]** On your own machine: `ssh-keygen -t ed25519 -C fin1-admin` (set a passphrase). Keep the private key; send only `~/.ssh/id_ed25519.pub`.
2. On the node: `bash deploy/lib/ssh-harden.sh add-key --pubkey-file <file>` (or `--pubkey 'ssh-ed25519 AAAA...'`). It refuses private keys and garbage, is idempotent, sets mode 0600, and prints the fingerprint.
3. **[OWNER] From a different machine or network, in a NEW terminal** (the first session stays open): `ssh -i ~/.ssh/id_ed25519 root@62.238.46.190 true` must succeed with **no password prompt**.
4. `bash deploy/lib/ssh-harden.sh verify` -> `VERIFIED` only if sshd's own log shows `Accepted publickey for root ... <that fingerprint>`. A password login, or a login by another key, does not count (tested).
5. `bash deploy/lib/ssh-harden.sh disable-passwords` -> writes `/etc/ssh/sshd_config.d/00-singbox-vpn-hardening.conf`, runs `sshd -t`, reloads, then checks the **effective** value is `no`; on any failure it rolls itself back.
6. **[OWNER]** Open yet another new session with the key: must work. Then confirm a password attempt is refused: `ssh -o PubkeyAuthentication=no root@62.238.46.190` -> `Permission denied`.
7. Rollback at any time: `bash deploy/lib/ssh-harden.sh rollback` (from the still-open session or the provider console).
Evidence: the `status`/`verify` outputs; then `python3 -I deploy/lib/check_deployment_contract.py` must show `security.ssh_password_authentication` and `ssh_permit_root_login` as PASS.

## 2. Firewall verification
* Host (read-only): `ufw status verbose` — expect default deny incoming, rules for OpenSSH (LIMIT), 443/tcp, 443/udp, 8443/tcp, v4 and v6. `bash deploy/almalinux/acceptance-test.sh` must say `PASSED` for the ufw lines; its verdict also states how many non-required checks failed.
* Provider firewall (not visible from the node): **[OWNER]** in the Hetzner console confirm inbound 22/tcp, 443/tcp, 443/udp, 8443/tcp are allowed (and nothing else needs to be). Record it as a screenshot or a list in the acceptance record. Until then provider-level exposure is UNVERIFIED.
* External reachability is proven in step 4, not here.

## 3. Reboot test (service-affecting: schedule a window; it ends any session on the node)
1. `python3 -I deploy/lib/reboot_check.py capture /root/pre-reboot.json`
2. Keep the provider console open. `systemctl reboot` (**[OWNER] approval needed**; downtime ~1-2 min).
3. After it is back: `python3 -I deploy/lib/reboot_check.py capture /root/post-reboot.json && python3 -I deploy/lib/reboot_check.py compare /root/pre-reboot.json /root/post-reboot.json`.
4. PASS requires: different `boot_id`; sing-box, vpn-subscription, nginx, ssh, ufw, vpn-egress-isolation active again; identical credentials/config hashes, routing fingerprint, firewall rules, listeners, sshd effective settings; the egress-isolation table is **uid-scoped**; and, if the host has global IPv6, the gateway neighbour is not `FAILED` (the I-54 regression guard).
5. Then `vpn-admin doctor --protocol --require-protocol` and `python3 -I deploy/lib/check_deployment_contract.py`.
Note: on the published rc.2 the egress-isolation script is the ND-breaking one; a reboot reapplies it. Run this test with a corrected release (or re-apply the fixed script) or expect `FAIL`.

## 4. External-network acceptance (the vantage this host cannot provide)
Run from a machine **outside** Hetzner's network (home broadband, then again on a phone hotspot):
```
python3 external_probe.py --host fin1.xn----8sba1cvaif4e.com --expect-ip 62.238.46.190 \
        --url-file ./sub-url.txt --sing-box /path/to/sing-box --out external-<network>.json
```
(`sub-url.txt` holds the subscription URL on one line; the tool never prints it. Without `--url-file/--sing-box` the data-plane step is listed as SKIPPED, not passed.) Expect: DNS A correct, TCP 443/8443/22 reachable, TLS verified without `-k`, and real REALITY **and** Hysteria2 clients carrying traffic with the node's egress IP. A Hysteria2 failure here with everything else green points at UDP/443 filtering on that network (historical I-21), not at the node: repeat from a second network before concluding.

## 5. What closes what
| Item | Closed by | Class |
|---|---|---|
| I-39 SSH password/root login | step 1 + contract PASS | server verified |
| Provider firewall | step 2 owner confirmation | owner-attested |
| Reboot persistence | step 3 `compare` PASS | server verified |
| External reachability, UDP/443 | step 4 from >= 2 networks | external-network |
Nothing in this runbook verifies a native client; that is `docs/DEVICE_ACCEPTANCE_TESTS.md`.
