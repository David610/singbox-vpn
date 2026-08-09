# ALMALINUX_DEPLOYMENT.md

Production runbook for the Hiddify/VLESS-REALITY/Hysteria2 compatibility
stack. Despite the filename/directory name (kept for backwards
compatibility with existing links), this deployment path now supports
the RHEL family (AlmaLinux, Rocky Linux, RHEL 9) and the Debian family
(Ubuntu 22.04/24.04, Debian 12/13) — see `deploy/lib/os.sh`. Every
command below is copy-pasteable. This deploys the compatibility
(sing-box) data plane only — the native `direct-tls`/`noise-quic` stack
stays on `deploy/local/` (see `docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md`
§14).

**Most installs should use the one-command bootstrap** described in the
top-level [`README.md`](../README.md#quick-install):

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash
```

That command downloads this repo, auto-detects your public IP, issues a
real (non-self-signed) TLS certificate automatically via
[sslip.io](https://sslip.io) + certbot, and runs everything below for
you — no domain, no manual certbot step, no Rust toolchain (it prefers
a prebuilt release binary and only falls back to `cargo build` when
none exists yet). The rest of this document describes what that
one-liner does internally, and how to run every stage manually if you
want full control (your own domain, hand-provisioned certs, etc).

## Prerequisites (manual path)

- A fresh, supported VPS with a public IPv4 (IPv6 optional), root SSH
  access.
- Optionally, a domain name pointed at the VPS (an A record for both
  `vpn.example.com` and, if different, `sub.example.com`) if you don't
  want the automatic `sslip.io`-based hostname. REALITY's disguise
  handshake target does not need a domain at all — VLESS+REALITY works
  identically either way.
- A Rust toolchain installed (e.g. via `rustup`) if you want to force a
  from-source build instead of the installer's prebuilt-release path.
- A TLS certificate for `sub.example.com` (Let's Encrypt via a reverse
  proxy — see "Subscription HTTPS" below), unless you let the installer
  provision one automatically. REALITY itself does **not** need a
  certificate for `vpn.example.com` (it dials a real site's TLS
  handshake as a disguise; sing-box owns that logic).

## Two independent TLS certificates — do not confuse them

This deployment uses **two separate TLS certificates for two separate
purposes**. They are not interchangeable, and this doc previously
described them loosely enough to invite confusion — being explicit here
on purpose:

| | Hysteria2 TLS | Subscription HTTPS |
|---|---|---|
| Hostname (example) | `vpn.example.com` | `sub.example.com` |
| Port | `443/udp` | `8443/tcp` |
| Consumed by | `sing-box` directly (`tls.certificate_path`/`key_path` on the `hysteria2` inbound) | `nginx` (reverse proxy in front of the loopback-only Rust subscription service) |
| File location | `/etc/vpn/compat/hysteria/{cert,key}.pem` | `/etc/letsencrypt/live/<SUBSCRIPTION_HOST>/{fullchain,privkey}.pem` |
| Ownership | `root:sing-box 0640` | managed by certbot, read by nginx (root) |

They **may** reuse the same domain/certificate if `PUBLIC_HOST` and
`SUBSCRIPTION_HOST` are the same value and you deliberately copy one
cert to both locations — nothing in this stack assumes that silently.
By default `install.sh` treats them as fully independent and requires
both to be separately provisioned before it will start the
corresponding service.

(VLESS+REALITY needs **no certificate at all** — REALITY dials a real
site's TLS handshake as a disguise; sing-box owns that logic entirely.)

## Certificates

Both certificates above must exist **before** running `install.sh` —
the installer does not run an ACME client for you (task requirement:
don't hand-roll a second ACME integration; a single explicit operator
step is simpler and has fewer moving parts for a boring V1). Provision
them with certbot (or your own ACME client of choice):

```bash
sudo dnf install -y certbot
# Hysteria2 cert (consumed directly by sing-box):
sudo certbot certonly --standalone -d vpn.example.com \
  --non-interactive --agree-tos -m admin@vpn.example.com
sudo install -d -m 0750 -o root -g sing-box /etc/vpn/compat/hysteria
sudo install -m 0640 -o root -g sing-box \
  /etc/letsencrypt/live/vpn.example.com/fullchain.pem /etc/vpn/compat/hysteria/cert.pem
sudo install -m 0640 -o root -g sing-box \
  /etc/letsencrypt/live/vpn.example.com/privkey.pem /etc/vpn/compat/hysteria/key.pem

# Subscription HTTPS cert (consumed by nginx) — same command, different
# hostname; skip if SUBSCRIPTION_HOST equals PUBLIC_HOST and you intend
# to reuse the cert above instead.
sudo certbot certonly --standalone -d sub.example.com \
  --non-interactive --agree-tos -m admin@sub.example.com
```

`install.sh`'s "certificates" stage refuses to start `sing-box.service`
if the Hysteria2 cert/key are missing or invalid (`openssl x509
-checkend`) — it prints the exact commands above and exits non-zero
rather than starting a service that is guaranteed to fail. If the
subscription cert isn't present yet, install still completes (the
compatibility transport doesn't depend on it) but the nginx vhost is
left unconfigured and the final status explicitly says `SUBSCRIPTION
HTTPS: NOT CONFIGURED` — re-run `install.sh` once the cert exists.

Renewal: certbot's systemd timer handles both certs automatically.
After the Hysteria2 cert renews, re-copy it into
`/etc/vpn/compat/hysteria/` (preserving `root:sing-box 0640`) and
`sudo systemctl reload-or-restart sing-box` — a `certbot renew` deploy
hook is the natural way to automate this on a real host, not
implemented by this repo. After the subscription cert renews, `nginx -s
reload` is enough (TLS termination lives entirely in nginx, no
compatibility-stack service needs to restart).

## Fresh install (manual, own domain)

```bash
git clone <this-repo-url> vpn1 && cd vpn1
sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=sub.example.com \
  ./deploy/almalinux/install.sh
```

Explicit stages (each logged as `[install] === [N/17] ... ===`; any
stage failing aborts the script before "Install complete." is ever
printed):

1. preflight (root, OS/arch detection, disk/RAM/connectivity/DNS,
   port-conflict checks, existing-install detection)
2. OS packages (`dnf` on RHEL family, `apt` on Debian family — incl.
   `nginx`, `firewalld`/`ufw`, `certbot`)
3. host configuration (uses `PUBLIC_HOST` if set; otherwise
   auto-detects the public IP and assigns an `sslip.io` hostname)
4. vpn1 binaries (prebuilt release if available and checksum-verified;
   otherwise `cargo build --release`, auto-installing `rustup` if
   `cargo` is missing)
5. sing-box installation (pinned version, checksum-verified when
   published, `LICENSE` copied alongside the binary)
6. users/groups (`vpn-subscription`, `sing-box` service accounts)
7. directories (ownership matrix below)
8. certificates (Hysteria2 TLS required — auto-issued via certbot when
   no domain was manually supplied; subscription TLS optional — see
   "Certificates" above)
9. REALITY keys (`vpn-admin init`, re-owned to `root:sing-box`/
   `root:vpn-subscription` per file)
10. server config (`vpn-admin render-config` — **not** wrapped in
    `|| true`; a render/validate failure aborts install)
11. nginx/subscription HTTPS (auto-configured if the subscription cert
    is present; skipped with a clear status otherwise)
12. firewall (`firewalld` on RHEL family, `ufw` on Debian family: SSH,
    443/tcp, 443/udp, 8443/tcp only)
13. SELinux (RHEL family only — fcontext labeling for the binary and
    secret-serving directories)
14. systemd (unit install + daemon-reload)
15. validation + start (`sing-box check`, then enable + start both
    services)
16. first user + acceptance test (auto-creates a `default` user if none
    exists yet; `vpn-health-check` must pass)
17. summary (prints the subscription URL and, if `qrencode` is
    installed, a terminal QR code)

### File ownership (docs/PRODUCTION_HARDENING_PLAN.md #1)

Every path is owned by the group that actually needs to read it at
runtime — `sing-box` (`User=sing-box`) and `vpn-subscription`
(`User=vpn-subscription`) never share a group, and neither can read the
other's secrets:

| Path | Owner | Mode | Read by |
|---|---|---|---|
| `/etc/vpn/compat/reality/private.key` | `root:sing-box` | `0640` | sing-box |
| `/etc/vpn/compat/reality/public.key`, `short_id.txt` | `root:vpn-subscription` | `0640` | vpn-subscription |
| `/etc/vpn/compat/users/users.json` | `root:vpn-subscription` | `0640` | vpn-subscription |
| `/etc/vpn/compat/hysteria/{cert,key}.pem` | `root:sing-box` | `0640` | sing-box |
| `/etc/vpn/compat/sing-box/config.json` | `root:sing-box` | `0640` | sing-box |

Verify this on a live host with
`sudo ./deploy/almalinux/acceptance-test.sh` (checks both nominal
ownership/mode *and* effective `sudo -u <user> test -r <path>` access).

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
sudo vpn-admin --config /etc/vpn/deployment.toml user remove user_xxxxxxxx
```

Both commands render + validate + apply the new config, then reload
`sing-box` (`systemctl reload-or-restart`) and verify it comes back
active — the command does not report success until the running server
has actually stopped accepting the old credentials. If the reload
itself fails, the previous config is restored and reloaded back, and
the command exits non-zero explaining exactly that (see
`docs/PRODUCTION_HARDENING_PLAN.md` #4/#7) — it never prints "user
disabled successfully" while the old credentials are still live.

## Credential rotation

| What | Command | Client impact |
|---|---|---|
| Subscription token | `vpn-admin user rotate-token <id>` | Old subscription URL stops working; VLESS/Hysteria2 credentials unchanged, so already-connected clients using the sing-box native app config keep working until they re-fetch. |
| VLESS UUID only | `vpn-admin user rotate-vless <id>` | User must re-import; Hysteria2 credentials unaffected. |
| Hysteria2 password only | `vpn-admin user rotate-hysteria <id>` | User must re-import; VLESS credentials unaffected. |
| Both VLESS UUID + Hysteria2 password | `vpn-admin user rotate-credentials <id>` | User must re-import both transports. |
| REALITY keypair | `vpn-admin init --rotate` | **Every** client using this server must re-import (spec §12/§30: high impact, do this deliberately, rarely). |
| TLS certificate (Hysteria2) | manual re-copy + `systemctl reload-or-restart sing-box` (see "Certificates" above) | None — same cert, just renewed. |
| TLS certificate (subscription reverse proxy) | `certbot renew` (automatic) + `nginx -s reload` | None — reverse proxy only. |

All four `rotate-*`/`disable`/`enable`/`remove` commands go through the
same render→validate→apply→reload→verify→rollback-on-failure path.

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

`uninstall.sh` never deletes `/etc/vpn` (REALITY private key,
`users.json`, TLS material) without the explicit `--purge-state` flag,
and never touches firewall rules without `--purge-firewall`.
`--purge-state` prints exactly what it is about to remove before doing
so.

## Acceptance test

```bash
sudo ./deploy/almalinux/acceptance-test.sh
```

Verifies OS version, SELinux enforcing, firewalld active, file
ownership/permissions (nominal *and* effective per-user access via
`sudo -u <user> test -r/-w`), both services active, `sing-box check`,
Hysteria TLS cert validity/expiry, the subscription reverse proxy
reachable over HTTPS, and that ports 1080/9000/9100 are not publicly
listening. Never prints secrets. Exits non-zero on any required
failure. Distinguishes a container smoke test (package/script syntax
only) from a real VM/VPS run (SELinux/firewalld/systemd/low-port
behavior) — see the script's own header comment.

## SELinux troubleshooting

`install.sh` labels the sing-box binary and the directories it reads
from (`fcontext` + `restorecon`), which should be sufficient on a
stock AlmaLinux 9 policy. If `sing-box` still fails to start or bind
443 with SELinux enforcing, check for AVC denials before doing anything
else:

```bash
sudo ausearch -m AVC -ts recent
sudo journalctl -u sing-box --since "5 min ago"
```

If a denial appears, generate the smallest policy addition for exactly
that denial (`audit2allow`) rather than disabling enforcement:

```bash
sudo ausearch -m AVC -ts recent | audit2allow -M vpn-compat-local
sudo semodule -i vpn-compat-local.pp
```

**`setenforce 0` is not an acceptable production fix** — it disables
SELinux confinement for the entire host, not just this service.

## Known limitations of this deployment doc

This runbook and the scripts it documents are syntax-checked
(`bash -n`/`shellcheck deploy/almalinux/*.sh`, both clean) and the
underlying Rust logic (ownership/permission modes, reload/rollback,
credential generation) is unit- and integration-tested. The scripts
themselves have **not been executed against a real AlmaLinux 9 host, a
real VPS, or a real domain/DNS/TLS setup** in this development
session — this sandbox has no root network capability, no `dnf`, and
no real VPS. Treat the first real run (`install.sh` followed by
`acceptance-test.sh`) as the actual validation pass, and record the
outcome in `docs/CLIENT_COMPATIBILITY.md`'s manual acceptance log —
do not treat this document's existence as proof of a validated
deployment.
