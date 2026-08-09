# ALMALINUX_DEPLOYMENT.md

Production runbook for the Hiddify/VLESS-REALITY/Hysteria2 compatibility
stack on AlmaLinux 9. Every command below is copy-pasteable. This
deploys the compatibility (sing-box) data plane only — the native
`direct-tls`/`noise-quic` stack stays on `deploy/local/` (see
`docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §14).

## Prerequisites

- A fresh AlmaLinux 9 VPS with a public IPv4 (IPv6 optional), root SSH
  access, `firewalld` and SELinux enforcing (the default).
- A domain name pointed at the VPS (an A record for both
  `vpn.example.com` and, if different, `sub.example.com`). REALITY's
  disguise handshake target and Hiddify's own TLS validation both
  benefit from a real domain, not a bare IP.
- A Rust toolchain installed (e.g. via `rustup`) — the installer builds
  from source, it does not download prebuilt Rust binaries.
- A TLS certificate for `sub.example.com` (Let's Encrypt via a reverse
  proxy — see "Subscription HTTPS" below). REALITY itself does **not**
  need a certificate for `vpn.example.com` (it dials a real site's TLS
  handshake as a disguise; sing-box owns that logic).

## Fresh install

```bash
git clone <this-repo-url> vpn1 && cd vpn1
sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=sub.example.com \
  ./deploy/almalinux/install.sh
```

This installs OS packages, builds `vpn-admin`/`vpn-subscription-svc`,
downloads and pins `sing-box` (checksum-verified when upstream
publishes one — see `docs/COMPATIBILITY_VERSIONS.md`), creates
`vpn-subscription`/`sing-box` service users, generates the REALITY
keypair (refuses to overwrite an existing one), renders
`/etc/vpn/deployment.toml`, installs systemd units, configures
firewalld, sets the SELinux file context on the sing-box binary, and
starts both services.

## Subscription HTTPS (reverse proxy)

`services/subscription` binds `127.0.0.1:9100` only (spec §27). Put a
TLS-terminating reverse proxy in front of it on `8443/tcp` — we do not
build custom TLS termination. Minimal nginx example:

```nginx
server {
    listen 8443 ssl http2;
    server_name sub.example.com;
    ssl_certificate     /etc/letsencrypt/live/sub.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/sub.example.com/privkey.pem;
    location / {
        proxy_pass http://127.0.0.1:9100;
    }
}
```

Obtain the certificate with `certbot --nginx -d sub.example.com`
(HTTP-01 on port 80, which is free since REALITY/Hysteria2 only use
443). Renewal: certbot's systemd timer handles this automatically;
`nginx -s reload` is enough after renewal (no compatibility-stack
service needs to restart, since TLS termination lives entirely in
nginx).

## Creating the first user

```bash
sudo vpn-admin --config /etc/vpn/deployment.toml user create --name test
```

Prints:

```
User created: user_xxxxxxxx

Subscription:
https://sub.example.com:8443/sub/<token>
```

This URL is shown exactly once (spec §26 — only the token's hash is
persisted). Copy it now.

## Testing with Hiddify

See `docs/HIDDIFY_ANDROID.md` for the end-user steps. As the operator,
you can sanity-check the subscription body yourself first:

```bash
curl -s "https://sub.example.com:8443/sub/<token>?format=singbox" | jq .
curl -s "https://sub.example.com:8443/sub/<token>?format=uri"
```

## Service status / logs

```bash
systemctl status sing-box vpn-subscription
journalctl -u sing-box -f
journalctl -u vpn-subscription -f
sudo vpn-health-check
```

## Firewall

```bash
sudo ./deploy/almalinux/firewall.sh
sudo firewall-cmd --list-all
sudo ss -tulpn
```

## Disabling / removing a user

```bash
sudo vpn-admin --config /etc/vpn/deployment.toml user disable user_xxxxxxxx
# old credentials now rejected: sing-box config was regenerated + reloaded
# with that user excluded (spec §29).
sudo vpn-admin --config /etc/vpn/deployment.toml user remove user_xxxxxxxx
```

## Credential rotation

| What | Command | Client impact |
|---|---|---|
| Subscription token | `vpn-admin user rotate-token <id>` | Old subscription URL stops working; VLESS/Hysteria2 credentials unchanged, so already-connected clients using the sing-box native app config keep working until they re-fetch. |
| VLESS UUID / Hysteria2 password | not yet a single command — `user remove` + `user create` for that person | User must re-import a fresh subscription URL. |
| REALITY keypair | `vpn-admin init --rotate` | **Every** client using this server must re-import (spec §12/§30: high impact, do this deliberately, rarely). |
| TLS certificate (subscription reverse proxy) | `certbot renew` (automatic) | None — reverse proxy only. |

## Updating

```bash
sudo ./deploy/almalinux/update.sh
```

Builds, validates the re-rendered sing-box config, restarts services,
runs the health check, and automatically rolls back binaries + config
if the health check fails (spec §48/§49).

## Backup

Back up:

- `/etc/vpn/deployment.toml`
- `/etc/vpn/compat/reality/` (REALITY private key — losing this means
  regenerating and every client re-importing)
- `/etc/vpn/compat/users/users.json`
- Reverse proxy TLS/ACME state (e.g. `/etc/letsencrypt/`)

Do **not** need to back up: `/etc/vpn/compat/sing-box/config.json`
(regenerable from `users.json` + the REALITY key via
`vpn-admin render-config`), logs, `target/`.

Restore: reinstall the OS packages/binaries with `install.sh` (it will
skip secret generation if the restored files are already present at
their expected paths — `reality/private.key` existing is what triggers
the "refuse to overwrite" path), copy the backed-up `/etc/vpn` tree
back into place with correct ownership/permissions, then
`./deploy/almalinux/render-config.sh`.

## Rollback

See `deploy/almalinux/update.sh` — it keeps a timestamped backup under
`/etc/vpn/backups/` and rolls back automatically on health-check
failure. To roll back manually to a specific backup:

```bash
sudo cp /etc/vpn/backups/<timestamp>/vpn-admin /usr/local/bin/vpn-admin
sudo cp /etc/vpn/backups/<timestamp>/vpn-subscription-svc /usr/local/bin/vpn-subscription-svc
sudo cp /etc/vpn/backups/<timestamp>/config.json /etc/vpn/compat/sing-box/config.json
sudo systemctl restart vpn-subscription
sudo systemctl reload-or-restart sing-box
```

## Uninstall

```bash
sudo ./deploy/almalinux/uninstall.sh                 # keeps /etc/vpn
sudo ./deploy/almalinux/uninstall.sh --purge-state --purge-firewall  # full removal
```

## Known limitation of this deployment doc

This runbook was written and syntax-checked
(`bash -n deploy/almalinux/*.sh`) but **not executed against a real
AlmaLinux host** in this development session — the sandbox has no root
network capability, no `dnf`, and no real domain/VPS. Treat the first
real run as the actual validation pass and update
`docs/CLIENT_COMPATIBILITY.md` with the outcome.
