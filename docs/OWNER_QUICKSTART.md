# Owner quickstart: two servers, two directions

One page. Full detail and rationale: [BIDIRECTIONAL_RUNBOOK.md](BIDIRECTIONAL_RUNBOOK.md). Replace `A` and `B` with your
two servers (SSH names), `a.example.org`/`b.example.org` with their host names. Needs the same release on both.

**1. Install (each server, fresh Linux; dry run first).**

    TAG=v1.4.0-rc.2    # the release you chose
    curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$TAG/install.sh \
      | sudo bash -s -- --version $TAG --non-interactive --role relay --node-id a \
        --domain a.example.org --reality-handshake-server www.cloudflare.com        # B: --node-id b --domain b.example.org

**2. Enable two-way mode (each server).**

    printf '\n[exit_principals]\nenabled = true\n' | sudo tee -a /etc/vpn/deployment.toml >/dev/null
    sudo vpn-admin render-config && sudo vpn-admin doctor

**3. Add a family member (both directions).** `set -o pipefail`; the bundle travels only through the pipe.

    ssh A 'sudo vpn-admin user create --name anna-ab --json'    # keep the printed URL: Anna's link "enters A, exits B"
    ssh B 'sudo vpn-admin pair issue-exit --label anna-ab' | ssh A 'sudo vpn-admin pair accept-exit --user <anna-ab id> --endpoint-id b-via-a --bundle-stdin'
    ssh B 'sudo vpn-admin user create --name anna-ba --json'    # link "enters B, exits A"
    ssh A 'sudo vpn-admin pair issue-exit --label anna-ba' | ssh B 'sudo vpn-admin pair accept-exit --user <anna-ba id> --endpoint-id a-via-b --bundle-stdin'

Send the two links privately (one message each) with `docs/clients/ONBOARDING_RU.md`. Do not append anything to the URL.

**4. Disable / revoke / renew / replace** (on the entry server, ids from `vpn-admin user list`):

    sudo vpn-admin user disable <id>              # pause (enable to resume)
    sudo vpn-admin user remove <id>               # revoke; also remove peer-<label> on the OTHER server
    sudo vpn-admin user set-expiry <id> --expires-at <unix>
    sudo vpn-admin user rotate-token <id>         # new link, old one stops

**5. Is it working?** `sudo vpn-admin doctor --protocol` on both. From a client: profile "enters A" must show B's IP, and the
reverse. Stop one server and confirm the profile fails (it must never show another country).

**6. Update safely.** Back up, update one server, check, then the other:

    sudo vpn-admin backup --output /root/backups/$(hostname)-$(date -u +%Y%m%dT%H%M%SZ).tar     # copy off-host, encrypted
    sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version vX.Y.Z

**7. Restore.** Same role and node id only (otherwise refused): `sudo vpn-admin restore <archive>`; a changed role means a
reinstall with the original `--role --node-id`, then restore.

**8. Hiddify or YouTube stops working.** Try the other profile first. Hiddify: re-import the pinned link, no extra
settings. YouTube: depends on the exit IP's reputation and the network; Shorts are not guaranteed on every exit. Send
`sudo vpn-admin doctor --report` output (secrets are redacted) when asking for help.
