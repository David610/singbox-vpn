# Russian relay + foreign exit with YouTube Shorts (step by step)

An operator procedure for two servers: a **relay** (entry) in Russia and an **exit** abroad. Clients connect to the relay;
general traffic leaves from the exit; Google/YouTube traffic is sent back through the relay (the **hairpin**) because
YouTube may refuse Shorts to the exit's IP while accepting the Russian one.

Status: followed end to end on two Ubuntu hosts on 2026-10-27 (scripted rebuild in about three minutes; a client run on the
relay saw the exit's IP for general traffic and HTTP 200/204 for Google, YouTube, a Shorts page and thumbnails, about
29 MB/s, no client errors; the owner reported it working on a phone). This is **not** a release qualification: the
native-device matrix in [DEVICE_ACCEPTANCE_TESTS.md](DEVICE_ACCEPTANCE_TESTS.md) was not run, and Shorts depend on each IP.
Replace `RELAY_IP`, `EXIT_IP`, `relay.example.org`, `exit.example.org` with your own. Run commands as root, one at a time.

## 0. Before you start

1. **DNS:** an `A` record per server, DNS only (not proxied): `relay.example.org` → `RELAY_IP`, `exit.example.org` → `EXIT_IP`.
2. **Certificate limit:** every install requests a Let's Encrypt certificate. The limit is **5 per exact hostname per week**.
   A reinstall that hits it fails with `rateLimited` and `sing-box` never starts. Use a fresh hostname for a repeat build.
3. **Ports**, in the host firewall and the provider's panel: TCP 22, 80, 443, 8443 and UDP 443.
4. **SSH:** `ss -tlnp | grep -w sshd` must print a line. If not: `apt-get update && apt-get install -y openssh-server && systemctl enable --now ssh`.

## 1. Install (both servers; wipe an old install first with `/opt/singbox-vpn/bin/singbox-vpn-uninstall --yes`)

Exit (abroad):

    TAG=v1.4.0
    curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$TAG/install.sh | bash -s -- --version $TAG --non-interactive --ssh-port 22 --domain exit.example.org --reality-handshake-server www.nvidia.com

Relay (Russia):

    TAG=v1.4.0
    curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/$TAG/install.sh | bash -s -- --version $TAG --non-interactive --ssh-port 22 --domain relay.example.org --reality-handshake-server www.nvidia.com --role relay --node-id ru1

## 2. Preflight on the relay: the decoy name must not stall

    curl -sk -o /dev/null --max-time 5 --resolve www.nvidia.com:443:EXIT_IP https://www.nvidia.com/; echo rc=$?

Run it three times; every run must print `rc=0`. `rc=28` means the Russian network stalls TLS to the exit for that decoy name.
Every relayed connection starts with that handshake, so everything times out. In the 2026-10-27 build `www.cloudflare.com`
stalled (4 of 4) while `www.nvidia.com`, `www.samsung.com`, `www.asus.com` and `www.hetzner.de` passed. If yours stalls, pick
another name and repeat steps 1 and 2 on both servers (the exit's decoy is part of what the relay stores when it pairs).

## 3. Relay: TLS spoof and your user

    vpn-admin tls-spoof set --sni ya.ru
    vpn-admin user create --name me --json

The Russian network also drops YouTube handshakes, which the spoof works around ([TLS_SPOOF.md](TLS_SPOOF.md)). The second
command prints the user `id` and the subscription URL **once**; copy both. Lost link: `vpn-admin user rotate-token <id>`.

## 4. Pair the exit

On the exit:

    vpn-admin pair issue-exit --label me --out /root/me-bundle.json

On the relay (it starts the connection; a copy started from the exit stalled between some hosts):

    scp root@EXIT_IP:/root/me-bundle.json /root/me-bundle.json
    chmod 600 /root/me-bundle.json
    vpn-admin pair accept-exit --user <id from step 3> --endpoint-id exit-via-relay --bundle-file /root/me-bundle.json
    vpn-admin pair status

`pair status` must show 1 paired route with `users_with_credential=1`. Then `shred -u /root/me-bundle.json` on both.
`issue-exit` and `issue-hairpin` never overwrite their `--out` file: run each once per label.

## 5. Hairpin (Google/YouTube leaves through the relay)

On the relay:

    vpn-admin pair issue-hairpin --label exit --out /root/exit-hairpin.json
    scp /root/exit-hairpin.json root@EXIT_IP:/root/exit-hairpin.json

On the exit:

    chmod 600 /root/exit-hairpin.json
    vpn-admin pair accept-hairpin --bundle-file /root/exit-hairpin.json
    vpn-admin pair status

The exit must report Google/YouTube egress hairpinned through the relay. Then `shred -u /root/exit-hairpin.json` on both.
Details and rollback (`vpn-admin google-egress-hairpin clear`): [PAIRING.md](PAIRING.md),
[YOUTUBE_FIX_INVARIANTS.md](YOUTUBE_FIX_INVARIANTS.md).

## 6. Verify and use

On both servers `vpn-admin doctor --protocol` must end with `SERVER-SIDE CHECKS PASSED`. In Hiddify, delete old profiles,
import the URL exactly as printed (it already ends in `?format=singbox&compat=hiddify-pinned`; adding anything gives HTTP
400), connect, and check that an IP-check page shows `EXIT_IP`. Then play ten or more different Shorts, signed in and in
Incognito. Change the root passwords and delete any leftover bundle files.

## What this does not guarantee

- Shorts depend on each IP's reputation and on the network. Test every new pair on a real phone.
- The hairpin adds hops for YouTube only; expect it to be slower than other traffic.
- A decoy name that passes on one Russian network may stall on another; repeat step 2 from where your users are.
- Both servers are best-effort Ubuntu here; AlmaLinux 9 is the supported tier ([BIDIRECTIONAL_RUNBOOK.md](BIDIRECTIONAL_RUNBOOK.md)).

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| Install ends with `rateLimited` | Let's Encrypt limit for that hostname; use a new hostname or wait for the printed retry time |
| Hiddify says Timeout; every site fails | Decoy name stalls from the relay to the exit: step 2 |
| YouTube works only without the hairpin, or freezes | Spoof missing on the relay: step 3; check `vpn-admin tls-spoof show` |
| `accept-exit` says no such user | Wrong id; take it from `vpn-admin user list` on the relay |
| Two users named `me` | Created twice; `vpn-admin user remove <id>` for the one without the pairing |
