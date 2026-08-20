# Installation

Canonical, detailed installation and operations reference for singbox-vpn.
`README.md` is a short quick start; this document covers the full support
matrix, provider/DNS setup, distribution-specific notes, and troubleshooting.
[docs/SUPPORTED_PRODUCT.md](SUPPORTED_PRODUCT.md) is authoritative whenever
another document disagrees with it on scope.

## Supported Linux distributions

`deploy/lib/os.sh` detects the OS family (`rhel`/`debian`), package manager
(`dnf`/`apt`), and firewall backend (`firewalld`/`ufw`), and the installer
warns (but does not refuse to continue) outside the **SUPPORTED** tier below.
These tiers describe actual evidence, not aspiration — see
[SUPPORTED_PRODUCT.md](SUPPORTED_PRODUCT.md) for the full authoritative
matrix and how each tier was earned.

| Distribution | Tier |
|---|---|
| AlmaLinux 9 x86_64 | **SUPPORTED** |
| Rocky Linux 9 | RECOGNIZED / BEST-EFFORT |
| Ubuntu 22.04 LTS / 24.04 LTS | RECOGNIZED / BEST-EFFORT |
| Debian 12 / 13 | RECOGNIZED / BEST-EFFORT |
| Amazon Linux 2023 | CI-TESTED |
| RHEL 9 / CentOS Stream 9 | RECOGNIZED / BEST-EFFORT |
| Oracle Linux 9 and anything else | UNSUPPORTED |

Architecture: **amd64 is the supported target.** arm64 has a working
`detect_arch()`/release-build implementation path but no verified live-host
install — treat it as RECOGNIZED / BEST-EFFORT, not SUPPORTED.

## Requirements

- A VPS matching one of the distributions above, root or sudo access, a
  public IPv4 address, and roughly 1 GB RAM or more.
- A domain or subdomain pointed at the VPS (see DNS setup below).
- [Hiddify](https://hiddify.com) or another sing-box-compatible client.

## DNS setup

Create an `A` record pointing your domain to the VPS:

```text
vpn.example.com → YOUR_VPS_IPV4
```

Check it before installing:

```bash
dig +short vpn.example.com
```

The returned IP must match your VPS. Do not add an `AAAA` record unless
IPv6 is configured and working.

### Cloudflare DNS

Set the VPN hostname to **DNS only** (grey cloud), never **Proxied** (orange
cloud). REALITY and Hysteria2 both need a direct connection to the VPS; they
do not work through Cloudflare's HTTP proxy.

## Provider firewall

The installer only manages the VPS's own OS firewall (`firewalld` or
`ufw`). Most cloud providers enforce a **separate** network-level firewall
in front of the VM (AWS Security Groups, GCP firewall rules, Azure NSGs)
that the installer cannot see or change — both layers must independently
allow a port before traffic reaches the service.

| Port | Protocol | Purpose |
|---|---|---|
| SSH port | TCP | Server administration |
| 80 | TCP | TLS certificate issuance/renewal (temporary at install, permanent if you want renewals to keep working — see [TLS / Certbot](#tls--certbot)) |
| 443 | TCP | VLESS + REALITY |
| 443 | UDP | Hysteria2 |
| 8443 | TCP | Subscription endpoint (configurable via `--subscription-port`) |

### AWS EC2 example

Add these inbound Security Group rules:

```text
SSH          TCP   22     YOUR_IP/32
HTTP         TCP   80     0.0.0.0/0
REALITY      TCP   443    0.0.0.0/0
Hysteria2    UDP   443    0.0.0.0/0
Subscription TCP   8443   0.0.0.0/0
```

Replace `22` if you use a custom SSH port. TCP/443 and UDP/443 are
**separate** rules — opening TCP/443 does not open Hysteria2.

Test external reachability from a machine other than the VPS itself:

```bash
curl -sS --max-time 5 http://<host>/ -o /dev/null -w '%{http_code}\n'        # TCP/80
curl -sS --max-time 5 https://<host>:8443/ -o /dev/null -w '%{http_code}\n'  # subscription HTTPS
```

Any HTTP status code (even an error) means the port is reachable; a timeout
or connection-refused means the provider firewall layer is blocking it.

## Installation

### Stable release

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

Resolves the latest non-prerelease tag, downloads source and binaries from
that exact version, and verifies them against the release's `SHA256SUMS`.
If no stable release exists yet, it exits before touching the server rather
than silently installing mutable branch source.

### Development install

For disposable VPS testing only — unverified branch source, no checksum:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo VPN1_CHANNEL=dev bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

### Custom SSH port

The installer never activates the firewall before your real SSH port is
positively identified and explicitly allowed. Pass it if auto-detection is
inconclusive:

```bash
... | sudo bash -s -- --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com \
    --ssh-port 2222
```

### Custom subscription port

```bash
... | sudo bash -s -- --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com \
    --subscription-port 8444
```

Open the matching TCP port in your provider firewall too.

A successful installation prints the first user's subscription URL and QR
code. See the top-level `README.md` for the short Hiddify connection steps.

## Distribution-specific notes

### AlmaLinux / Rocky / RHEL / CentOS Stream (dnf, firewalld)

`certbot` is **not** in BaseOS/AppStream on this family — it ships only via
EPEL. The installer enables `epel-release` automatically before installing
`certbot` (skipped on Amazon Linux 2023 — see below). SELinux (`fcontext` +
`restorecon`) is applied automatically; see
[SELinux troubleshooting](#selinux-troubleshooting).

### Amazon Linux 2023 (dnf, firewalld)

AL2023 ships `certbot` directly in its own repositories and does **not**
support EPEL at all — the installer detects this `OS_ID` and skips EPEL
enablement. AL2023 also ships `curl-minimal` preinstalled, which owns
`/usr/bin/curl`; the installer only adds the full `curl` package when no
usable `curl` binary is already present, avoiding a package-file conflict.

### Ubuntu / Debian (apt, ufw)

The installer explicitly allows the confirmed SSH port before ever
activating `ufw`, mirroring the firewalld-family safeguard — an operator
connected over SSH is never at risk of being locked out mid-install.

## Firewall behavior

The installer never enables `ufw`/`firewalld` before your real SSH port is
known and allowed, never flushes or resets pre-existing rules, and only
adds its own allow rules for SSH, `443/tcp`, `443/udp`, and the
subscription port. Uninstall removes only the rules singbox-vpn itself
added (tracked via `/var/lib/vpn1/firewall-owned.env`), and restores
firewalld/ufw to whatever enabled/disabled state they were in before
installation if singbox-vpn was the one that activated them.

Re-apply firewall rules manually if needed:

```bash
sudo ./deploy/almalinux/firewall.sh       # RHEL family (firewalld)
sudo ./deploy/almalinux/firewall-ufw.sh   # Debian family (ufw)
sudo firewall-cmd --list-all              # or: sudo ufw status verbose
```

## TLS / Certbot

Two independent TLS certificates are used:

| | Hysteria2 TLS | Subscription HTTPS |
|---|---|---|
| Hostname (example) | `vpn.example.com` | `sub.example.com` |
| Port | `443/udp` | `8443/tcp` |
| Consumed by | `sing-box` directly | `nginx` (reverse proxy) |

VLESS+REALITY needs **no certificate** — it dials a real site's TLS
handshake as a disguise. Both certificates above are auto-issued via
certbot's HTTP-01 challenge if no domain-specific certificate already
exists. **TCP/80 must stay reachable from the public internet long-term**,
not just during install: certbot's renewal timer re-runs the HTTP-01
challenge roughly every 60 days. The installer only opens TCP/80
*temporarily* during install; permanently allow it afterward if you want
renewals to keep working automatically:

```bash
sudo firewall-cmd --permanent --add-service=http && sudo firewall-cmd --reload   # RHEL family
sudo ufw allow 80/tcp                                                            # Debian family
sudo certbot renew --dry-run
```

A renewal hook (`/etc/letsencrypt/renewal-hooks/deploy/vpn1-hysteria.sh`)
refreshes the Hysteria2 certificate copy, reloads and verifies `sing-box`,
and reloads nginx after every successful renewal — no manual step needed.

## Updating / repairing an installation

```bash
sudo /opt/vpn1/deploy/almalinux/update.sh --latest
sudo /opt/vpn1/deploy/almalinux/update.sh --version vX.Y.Z
sudo /opt/vpn1/deploy/almalinux/update.sh --repair   # reconcile without changing versions
```

Updates download and checksum-verify the target release before changing
live state, validate the re-rendered configuration, restart services, run
protocol health checks, and automatically roll back the complete
transaction on failure. A no-argument update is rejected so a version
change never happens by accident.

## User management

```bash
sudo vpn user list
sudo vpn user create --name alice --qr
sudo vpn user rotate-token USER_ID --qr
sudo vpn user disable USER_ID
sudo vpn user enable USER_ID
sudo vpn user remove USER_ID
```

Every reload (`create`/`disable`/`enable`/`remove`/`rotate-*`) briefly
disconnects **every** connected client, not only the user that changed —
sing-box has no in-place config reload for this stack, so each change is a
full stop+start. Reconnecting immediately afterward succeeds.

## Backup

```bash
sudo vpn backup
```

The archive contains sensitive credentials — store it on encrypted,
off-host storage. Restore only a trusted archive:

```bash
sudo vpn restore /path/to/singbox-vpn-backup-<timestamp>.tar
```

See [RECOVERY.md](RECOVERY.md) for full-host recovery and the exact
limitations of an application-state backup.

## Uninstall

```bash
sudo /opt/vpn1/bin/vpn1-uninstall --yes
```

Online fallback if `/opt/vpn1` is missing or damaged:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh \
  | sudo bash -s -- --yes
```

This removes everything singbox-vpn created — `/etc/vpn`, `/opt/vpn1`,
`/var/lib/vpn1`, its systemd units, the nginx vhost, the sing-box binary (if
singbox-vpn installed it), its certbot renewal hook and certificates, its
firewall rules, and its kernel network tuning. It is ownership-aware:
anything that already existed before singbox-vpn (nginx, certbot,
firewalld/ufw, a pre-existing Rust toolchain, unrelated certificates) is
left alone or restored to its previous state. Safe to run more than once.
External provider firewall rules must be removed separately.

## Troubleshooting

### Hiddify shows `timeout`, but the VPN works

A Hiddify latency test timing out does not necessarily mean the VPN is
broken. Connect, open a website, and check your public IP changed instead.
Try **REALITY** manually instead of `auto`, then test **Hysteria2**
separately. Do not reinstall the server just because Hiddify reports
`timeout`.

### Connected, but the public IP does not change

Make sure Hiddify is using **VPN/TUN mode**, not proxy-only mode. On iOS,
check **Settings → General → VPN & Device Management** for a Hiddify VPN
configuration.

### REALITY works but Hysteria2 does not

Check UDP/443 in the VPS provider firewall, the OS firewall, and the
client's own network — on AWS, TCP/443 and UDP/443 require separate rules.
If REALITY works reliably, you can keep using it without Hysteria2.

### Hysteria2 works but REALITY does not

```bash
nc -vz vpn.example.com 443          # Linux/macOS
Test-NetConnection vpn.example.com -Port 443   # Windows PowerShell
sudo vpn doctor --protocol --require-protocol
journalctl -u sing-box --no-pager -n 100
```

### Nothing connects from AWS

Verify the Security Group allows TCP/443, UDP/443, TCP/8443, TCP/80, and
your SSH port — AWS Security Groups and the VPS OS firewall are separate;
both must allow the traffic.

### Subscription URL times out

```bash
curl -v https://vpn.example.com:8443/healthz
sudo ss -lntup
sudo vpn doctor
```

Also verify TCP/8443 (or your custom subscription port) in your provider
firewall.

### Subscription URL returns 404

A user ID is not a subscription token. Get a fresh URL:

```bash
sudo vpn user list
sudo vpn user rotate-token USER_ID --qr
```

### DNS points to the wrong server

```bash
dig +short vpn.example.com
```

Correct the `A` record and wait for DNS caches to update if it does not
match your VPS.

### `sudo vpn` says `command not found`

Some systems use a restricted `sudo` `PATH` (`secure_path`) that excludes
`/usr/local/bin`, where `vpn`/`vpn-admin` are installed. Confirm with
`sudo which vpn-admin`; if it reports "command not found", use the full
path:

```bash
sudo /usr/local/bin/vpn status
```

Or add `/usr/local/bin` to `secure_path` via `sudo visudo` if you want bare
`sudo vpn ...` to work permanently.

### Port 8443 is already in use

```bash
sudo ss -lntp | grep :8443
```

Reinstall with `--subscription-port 8444` (and open TCP/8444 in your
provider firewall) instead.

### Port 443 is already in use

```bash
sudo ss -lntup | grep :443
```

REALITY needs TCP/443 and Hysteria2 uses UDP/443. Stop or reconfigure the
conflicting service before installation.

### Installation stopped halfway through

```bash
sudo vpn doctor
```

If `vpn` is not available yet:

```bash
systemctl status sing-box vpn-subscription
journalctl -u sing-box -u vpn-subscription --no-pager -n 100
```

Fix the reported DNS, network, or firewall problem and rerun the installer
— it detects and repairs an existing partial installation in place.

### SELinux troubleshooting

RHEL family only. If `sing-box` fails to start or bind 443 with SELinux
enforcing, check for AVC denials before doing anything else:

```bash
sudo ausearch -m AVC -ts recent
```

Generate the smallest policy addition for exactly that denial rather than
disabling enforcement:

```bash
sudo ausearch -m AVC -ts recent | audit2allow -M vpn-compat-local
sudo semodule -i vpn-compat-local.pp
```

`setenforce 0` is not an acceptable production fix — it disables SELinux
confinement for the entire host, not just this service.

## See also

- [SUPPORTED_PRODUCT.md](SUPPORTED_PRODUCT.md) — authoritative support
  boundary
- [ALMALINUX_DEPLOYMENT.md](ALMALINUX_DEPLOYMENT.md) — extended
  AlmaLinux/RHEL-family reference (stage-by-stage internals, file
  ownership matrix, credential rotation table)
- [DEPLOYMENT.md](DEPLOYMENT.md) — developer/architecture internals
  (native adaptive stack, Docker services), unrelated to this installer
- [RECOVERY.md](RECOVERY.md) — full-host disaster recovery
- [clients/README.md](clients/README.md) — per-client setup guides
