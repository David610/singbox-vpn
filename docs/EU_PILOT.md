# EU-only private pilot (de1 entry -> fin1 exit)

Scope: a small private group of friends, Hiddify clients, connecting from Russia to a German entry (`de1`), leaving
from a Finnish exit (`fin1`). Google/YouTube leave from the German entry (the approved exception); everything else
leaves from Finland. There is no Russian-hosted server in this design. Release: `v1.3.1-rc.8` on both hosts.

Verdicts used here (nothing below is claimed beyond its evidence; see `qualification/2026-10-26/EU_PILOT_EVIDENCE.md`):

- **EU_STAGING_PASS** - server and core integration work. *Reached.*
- **EU_PILOT_READY** - a real Hiddify device connects with the pilot account and no critical defect is known.
  *Needs the owner's device test (`docs/EU_PILOT_TEST_RU.md`, or the same table on your own phone).*
- **EU_FRIENDS_READY** - the same passes from real Russian networks (mobile and home Wi-Fi) and onboarding/recovery is
  verified. *Needs the Russian friend.*

## What the client sees

One link per friend, always this form (the server refuses every other form that would skip the entry):

    https://de1.xn----8sba1cvaif4e.com:8443/sub/<personal token>?format=singbox&compat=hiddify-pinned

It yields a profile with ONE selectable server, `fin1 · via de1`. Import and refresh use the same URL. Each friend has
a separate relay user (personal token, entry credentials) and a separate dedicated exit credential
(`peer-<label>` on fin1). Revoking one friend touches nobody else.

## Provision a friend (owner workstation, SSH key access to both hosts)

Per friend; choose `NN` and an expiry (14-30 days, renewable). Run from a shell with `set -o pipefail`.

    # 1. relay user on de1 (writes the secret JSON to a root-only file; prints only the id)
    EXP=$(( $(date +%s) + 14*86400 ))
    ssh de1 "sudo bash -c 'umask 077; vpn-admin user create --name friend-NN --expires-at $EXP --json > /root/friend-NN.json'"
    ID=<the id shown by: ssh de1 'sudo vpn-admin user list'>

    # 2. dedicated exit credential, exit -> relay, no file
    ssh fin1 'sudo vpn-admin pair issue-exit --label friendNN --flow vision' \
      | ssh de1 "sudo vpn-admin pair accept-exit --user $ID --endpoint-id fin1-via-de1 --bundle-stdin"

    # 3. check on BOTH hosts: the relay's users_with_credential went up by one and fin1 lists peer-friendNN
    ssh de1 'sudo vpn-admin pair status'; ssh fin1 'sudo vpn-admin pair status'

    # 4. fetch the link into a protected local file, then destroy the server copy
    ssh de1 "sudo python3 -c \"import json;print(json.load(open('/root/friend-NN.json'))['subscription_url_hiddify_pinned'])\"" \
      > friend-NN-LINK.txt     # restrict the folder to your account
    ssh de1 'sudo shred -u /root/friend-NN.json'

Send the link privately, one friend per link. Do not post it, do not put it in tickets, CI or screenshots.
`user create` restarts sing-box once (a few seconds of interruption for connected friends): batch additions.

## Revoke, extend, rotate

| Need | Command (on de1 unless noted) | Effect |
|---|---|---|
| Stop one friend now | `vpn-admin user disable <id>` | credentials rejected immediately; link answers 404 |
| Delete one friend | `vpn-admin user remove <id>`, then on fin1 `vpn-admin user remove <peer-id>` | also frees the label |
| Extend | `vpn-admin user set-expiry <id> --expires-at <unix>` | takes effect on the next refresh |
| Link leaked | `vpn-admin user rotate-token <id>` | old link 404; credentials unchanged |
| Credentials leaked | `vpn-admin user rotate-credentials <id>` | old profile rejected; friend refreshes the profile |
| Pairing step failed | see the failure table in `docs/PAIRING.md` | nothing orphaned since rc.8 |

**Expiry is two-stage**: the link answers 404 at the instant of expiry (no new import or refresh), and the connection
credential is removed at the next reconcile run (every 10 minutes, so within ~10 minutes). Each such run restarts
sing-box and briefly interrupts everyone connected; plan expiries in batches.

## Recovery

| Symptom | Likely cause | Action |
|---|---|---|
| Link answers 404 | disabled, expired or token rotated | check `user list`; `set-expiry` / `enable` / `rotate-token` |
| Profile imports, will not connect, entry page not reachable from the friend's network | entry blocked or unreachable from that ISP | friend's failure type 3 (see test sheet); try the other network; there is no alternate entry in this design |
| Connects, nothing loads | exit down or credential removed | `vpn-admin doctor` on both; `systemctl status sing-box` on fin1 |
| Everything works except Google/YouTube | hairpin principal disabled or relay problem | fail-closed by design; `vpn-admin pair status` on fin1; do not "fix" by clearing the hairpin without a decision (Finnish Google egress is refused for Shorts) |
| After a host reboot | none expected | both hosts were rebooted in qualification; services and the spoof capability come back |
| Broken install | any | `vpn-admin repair` (idempotent, verified) |

Backups: `vpn-admin backup --output FILE` (root, mode 0600, contains secrets; restore refuses a role/node mismatch).

## Operating limits for this small group (grounded in measurements, not a promise)

- Accounts: up to ~10 friends. One separate account and credential each. No shared links.
- Capacity measured through the real chain: ~60 Mbit/s aggregate over 4 parallel streams with the 1-vCPU relay at ~10 %
  CPU and the exit at ~2 % (the workstation link was the limit). CPU is not the constraint; check the providers'
  traffic allowances yourself.
- Do not run bulk/torrent traffic; port 25 is rejected by policy.
- Group changes (adds, expiries) to limit sing-box restarts.
- TCP only: UDP/QUIC through the relay is not carried. If real devices stall on YouTube's QUIC attempts, the supported
  lever is the `compat=quic-reject` profile form (shown in the owner's `user create --json`); it is untested on devices.

## Security items still open (owner)

- Old credential captures in the assistant's working folder: pilot material for **ru1 and Evolus** (potentially still
  valid) - rotate/remove those pilot users, then delete the files. The fin1-host captures are dead (404).
- The Cloudflare token pasted in an earlier session: revoke it.
- Staging root passwords were rotated; SSH is key-only (`vpnadmin`, key `id_ed25519_staging_2026`, no passphrase - protect
  or replace it).
- `/root/pre-rc8-*` backups on both hosts contain secrets (mode 0600): delete once you accept the upgrade.
