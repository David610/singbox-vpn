# Runbook: two servers, two directions, any compatible pair

For a private deployment (3-10 people). Two servers, **A** and **B**, each reachable by name. Result: profile
**A->B** (client enters A, exits at B) and profile **B->A** (enters B, exits at A). Background and security model:
[BIDIRECTIONAL.md](BIDIRECTIONAL.md); pairing details: [PAIRING.md](PAIRING.md); general install notes:
[INSTALLATION.md](INSTALLATION.md). Replace `A`, `B`, `a.example.org`, `b.example.org`; nothing depends on any
particular address. Never paste secrets into chat, tickets or shell history; they travel only through the SSH pipes
below.

## 0. Requirements (each server)

| Item | Requirement |
|---|---|
| OS / CPU | AlmaLinux 9 x86_64 (SUPPORTED); Rocky 9, Ubuntu 22.04/24.04, Debian 12/13 best-effort. amd64 only |
| RAM / disk | 1 GB RAM (works; watch memory), >= 1 GB free disk for install plus backups |
| Network | public IPv4; TCP 22, 443, 8443 and 80 (certificate issuance) open at host AND provider firewall; UDP 443 unused by relays |
| DNS | an `A` record per server (`a.example.org`, `b.example.org`) pointing at its own IP; `dig +short` must match before installing |
| Access | root or sudo, SSH key login from your workstation, `bash curl tar` present |
| Clock | NTP active (certificates and tokens depend on it) |

Both servers run as **relays** with exit principals. Do not install either as an `exit`.

## 1. Install each server (one command, pinned and verified)

Run on A, then on B (fetch the installer from the SAME tag; it verifies `SHA256SUMS` and the Sigstore attestation):

    TAG=v1.4.0-rc.1
    curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$TAG/install.sh \
      | sudo bash -s -- --version $TAG --dry-run \
        --domain a.example.org --reality-handshake-server www.cloudflare.com --role relay --node-id a

Read the report (`READY TO INSTALL`), then run the same command again without `--dry-run`. On B use
`--domain b.example.org --node-id b`. Node ids are permanent: choose short, lowercase names.

Then enable exit principals on both:

    printf '\n[exit_principals]\nenabled = true\n' | sudo tee -a /etc/vpn/deployment.toml >/dev/null
    sudo vpn-admin render-config
    sudo vpn-admin doctor            # must show 0 FAIL

(If `/etc/vpn/deployment.toml` is elsewhere, `vpn-admin --help` shows `--config`.) The generated sing-box JSON is
never edited by hand; `render-config` rebuilds it and rolls back on a validation failure.

## 2. Back up before any change

    sudo install -d -m 0700 /root/backups
    sudo vpn-admin backup --output /root/backups/$(hostname)-$(date -u +%Y%m%dT%H%M%SZ).tar
    # copy it off the server over SSH, encrypted (e.g. age); it contains secrets

## 3. Add a friend and pair both directions

Choose a short label per person, e.g. `maria`. Use `set -o pipefail` and run from your workstation.

    # A->B: user on A, exit principal issued BY B and accepted ON A
    ssh A 'sudo vpn-admin user create --name maria-ab --json'     # note the user id; the URL shown is NOT the pinned one
    ssh B 'sudo vpn-admin pair issue-exit --label maria-ab' \
      | ssh A 'sudo vpn-admin pair accept-exit --user <maria-ab-id-on-A> --endpoint-id b-via-a --bundle-stdin'

    # B->A: user on B, exit principal issued BY A and accepted ON B
    ssh B 'sudo vpn-admin user create --name maria-ba --json'
    ssh A 'sudo vpn-admin pair issue-exit --label maria-ba' \
      | ssh B 'sudo vpn-admin pair accept-exit --user <maria-ba-id-on-B> --endpoint-id a-via-b --bundle-stdin'

    ssh A 'sudo vpn-admin pair status'; ssh B 'sudo vpn-admin pair status'   # each: 1 paired route, credentials filled

Only the bundle travels on stdout; credentials appear in no argument or log. One direction only? Run only that block.

### The two Hiddify links (send each privately, one at a time)

`user create` prints the subscription URL once (the token is never stored). Append the pinned selector:

    A->B (enters A, exits B):  https://a.example.org:8443/sub/<token-from-A>?format=singbox&compat=hiddify-pinned
    B->A (enters B, exits A):  https://b.example.org:8443/sub/<token-from-B>?format=singbox&compat=hiddify-pinned

Each shows ONE server in Hiddify (named `<exit> - via <entry>`). Name them for the user ("Maria via A to B", "Maria via
B to A"). A lost link: `ssh A 'sudo vpn-admin user rotate-token <id>'` mints a new one (the old one stops).

## 4. Verify before handing out

    ssh A 'sudo vpn-admin doctor --protocol'; ssh B 'sudo vpn-admin doctor --protocol'

Then from a client with each link, open an IP-echo site: profile A->B must show **B's** IP, B->A must show **A's**.
`doctor` proves the rendered policy and that the file on disk matches current authorization; only `doctor --protocol`
and this client test show that the RUNNING process behaves that way.

## 5. Friend lifecycle (per direction, independent)

| Action | Command (on the entry server unless stated) |
|---|---|
| Pause | `vpn-admin user disable <id>` (entry user; `user enable <id>` to resume) |
| Renew / set expiry | `vpn-admin user set-expiry <id> --expires-at <unix>`; `user clear-expiry <id>` |
| Rotate credentials | `vpn-admin user rotate-credentials <id>`; new link: `user rotate-token <id>` |
| Revoke fully | `user remove <id>` on the entry server AND `user remove <id of peer-LABEL>` on the exit server (ids from `user list`) |
| Revoke one direction only | do the above for that direction's user/principal; the other direction is untouched |

## 6. Operate

    sudo vpn-admin doctor                     # health, certificate expiry, policy
    sudo vpn-admin backup --output ...        # before every update
    sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version vX.Y.Z    # one server, then doctor, then the other
    sudo /opt/singbox-vpn/deploy/almalinux/update.sh --repair            # reconcile without changing version
    sudo vpn-admin restore <archive>          # same role AND same node id only (refused otherwise)

Update A, run `doctor --protocol` and a client test, only then B. A failed update rolls back automatically; if not,
`restore` the pre-update archive and `update.sh --version <previous>`.

## 7. An existing exit that must become a relay

A node's role is never rewritten. Treat it as a migration: back up and copy off-host; record the old `node_id` (a
restore of the old exit archive needs exactly that id and `--role exit`); `uninstall.sh`; reinstall as in step 1 with
`--role relay`; pair again (old exit credentials die with the reinstall). Rollback = uninstall, reinstall as
`--role exit --node-id <old id>`, `vpn-admin restore <old archive>`. Detailed plan: BIDIRECTIONAL.md "Migration".

## Limits

TCP only through either relay (no UDP/QUIC). Google/YouTube behaviour depends on the exit's IP reputation and is not
qualified for every direction; test Shorts per profile on a real device before promising it.
