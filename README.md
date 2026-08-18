<div align="center">

# singbox-vpn

**Your own VPN server, without building your own VPN stack.**

Deploy on a VPS → scan the QR → connect with Hiddify.

<br>

`VLESS + REALITY` &nbsp; `Hysteria2` &nbsp; `sing-box`

<br>

![Platform](https://img.shields.io/badge/server-AlmaLinux%209-222?style=flat-square)
![Users](https://img.shields.io/badge/designed%20for-%E2%89%A410%20users-222?style=flat-square)
![Self-hosted](https://img.shields.io/badge/self--hosted-yes-222?style=flat-square)

</div>

### One server is enough.

```text
┌─────────────┐          ┌─────────────┐          ┌─────────────┐
│     VPS     │          │  QR / URL   │          │   Hiddify   │
│  sing-box   │ ───────► │ subscription│ ───────► │ phone / PC  │
└─────────────┘          └─────────────┘          └─────────────┘
```

- **VLESS + REALITY** over TCP/443
- **Hysteria2** over UDP/443
- automatic TLS
- user management
- backup and restore
- complete offline uninstall

No custom client application or web control panel is required.

> Built for small groups of users.

> Release status: the supported path has an owner-reported smoke pass on a
> real AlmaLinux 9 VPS with Hiddify on an iPhone. The detailed device matrix
> is still partial, so this is not a claim that every client, network, or
> transport combination has been verified. See
> [Device acceptance tests](docs/DEVICE_ACCEPTANCE_TESTS.md).

## Quick start

### Requirements

You need:

- **AlmaLinux 9 x86-64 VPS**
- root or sudo access
- public IPv4
- about 1 GB RAM or more
- domain or subdomain pointing to the VPS
- Hiddify

Example:

```text
vpn.example.com → 203.0.113.10
```

## 1. Open the ports

| Port | Protocol | Purpose |
|---|---|---|
| SSH port | TCP | Server administration |
| 80 | TCP | TLS certificate validation |
| 443 | TCP | VLESS + REALITY |
| 443 | UDP | Hysteria2 |
| 8443 | TCP | Subscription endpoint |

The installer configures the firewall inside the VPS. Your VPS provider or cloud firewall must be configured separately.

The installer opens the VPS operating-system firewall for TCP/80 temporarily
during initial HTTP-01 certificate issuance. Certbot renewals need TCP/80 to
remain reachable from the public Internet. On AlmaLinux, keep it open after
installation unless you configure another ACME challenge method:

```bash
sudo firewall-cmd --permanent --add-service=http
sudo firewall-cmd --reload
sudo certbot renew --dry-run
```

Your provider firewall or AWS Security Group must allow TCP/80 as well. DNS-01
is an alternative, but this project does not configure it automatically.

### AWS EC2

Add these inbound Security Group rules:

```text
SSH          TCP   22     YOUR_IP/32
HTTP         TCP   80     0.0.0.0/0
REALITY      TCP   443    0.0.0.0/0
Hysteria2    UDP   443    0.0.0.0/0
Subscription TCP   8443   0.0.0.0/0
```

Replace `22` if you use another SSH port.

> TCP/443 and UDP/443 are separate rules. Opening TCP/443 does not open Hysteria2.

## 2. Configure DNS

Create an `A` record pointing your domain to the VPS:

```text
vpn.example.com → YOUR_VPS_IPV4
```

Check it:

```bash
dig +short vpn.example.com
```

or:

```bash
nslookup vpn.example.com
```

The returned IP should match your VPS.

### Cloudflare users

Set the VPN hostname to **DNS only** (grey cloud), not **Proxied** (orange
cloud). REALITY and Hysteria2 require a direct connection to the VPS and do
not work through Cloudflare's normal HTTP proxy.

Do not add an `AAAA` record unless IPv6 is configured and working.

## 3. Install

### Stable

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

Replace `vpn.example.com` with your domain.

The stable installer resolves the latest non-prerelease tag, downloads source
and binaries from that exact version, and verifies them against the release's
`SHA256SUMS`. If no stable release is available, it exits before modifying the
server rather than silently installing mutable branch source.

### Development

For disposable VPS testing only:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo VPN1_CHANNEL=dev bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

A successful installation prints the first user's **subscription URL and QR code**.

## 4. Connect with Hiddify

1. Open **New Profile**
2. Scan the QR code or paste the subscription URL
3. Select **REALITY** or **Hysteria2**
4. Connect
5. Check your public IP

Your public IP should now be the VPS IP.

**Native YouTube app fails while Safari plays YouTube fine?** See
`docs/clients/HIDDIFY_IOS.md`'s "Native YouTube app fails while Safari
plays YouTube fine" section and `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`
for an opt-in `?compat=tcp-only` diagnostic subscription mode.

# Commands

## Users

```bash
# List users
sudo vpn user list

# Create user + QR
sudo vpn user create --name alice --qr

# Replace subscription token
sudo vpn user rotate-token USER_ID --qr

# Disable / enable
sudo vpn user disable USER_ID
sudo vpn user enable USER_ID

# Remove
sudo vpn user remove USER_ID
```

Use the `USER_ID` shown by `vpn user list`.

> Treat subscription URLs like passwords.

## Server health

```bash
# Status
sudo vpn status

# Diagnostics
sudo vpn doctor

# Protocol test
sudo vpn doctor --protocol --require-protocol

# Services
systemctl status sing-box vpn-subscription

# Logs
journalctl -u sing-box -u vpn-subscription --no-pager -n 100
```

## Backup

```bash
sudo vpn backup
```

Backups contain sensitive VPN configuration. Store them securely.

## Uninstall

Offline uninstall:

```bash
sudo /opt/vpn1/bin/vpn1-uninstall --yes
```

Recovery fallback:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh \
  | sudo bash -s -- --yes
```

External provider firewall rules must be removed separately.

# Troubleshooting

## Hiddify shows `timeout`, but the VPN works

A Hiddify latency test timing out does not necessarily mean the VPN is broken.

Check the actual connection:

1. Connect
2. Open a website
3. Check your public IP
4. Confirm it changed to the VPS IP

If traffic works and the IP changed, the tunnel works.

Try **REALITY** manually instead of `auto`, then test **Hysteria2** separately.

Do not reinstall the server just because Hiddify reports `timeout`.

## Hiddify says connected, but the IP does not change

Make sure Hiddify is using **VPN/TUN mode**, not proxy-only mode.

On iOS, also check:

```text
Settings
→ General
→ VPN & Device Management
```

A Hiddify VPN configuration should exist.

Reconnect and check your public IP again.

## REALITY works but Hysteria2 does not

```text
REALITY    TCP/443
Hysteria2  UDP/443
```

Check UDP/443 in:

- VPS provider firewall
- AWS Security Group
- operating-system firewall
- client network

On AWS, TCP/443 and UDP/443 require separate rules.

If REALITY works reliably, you can continue using it without Hysteria2.

## Hysteria2 works but REALITY does not

### Windows

```powershell
Test-NetConnection vpn.example.com -Port 443
```

### Linux / macOS

```bash
nc -vz vpn.example.com 443
```

Then check the server:

```bash
sudo vpn doctor --protocol --require-protocol
journalctl -u sing-box --no-pager -n 100
```

## Nothing connects from AWS

Verify the Security Group:

```text
TCP/443
UDP/443
TCP/8443
TCP/80
SSH port
```

AWS Security Groups and the VPS operating-system firewall are separate. Both must allow the required traffic.

## Subscription URL times out

```bash
curl -v https://vpn.example.com:8443/healthz
sudo ss -lntup
sudo vpn doctor
```

Also verify TCP/8443 in your provider firewall.

If you configured another subscription port, use that port instead.

## Subscription URL returns 404

A user ID is not a subscription token.

Get a fresh URL:

```bash
sudo vpn user list
sudo vpn user rotate-token USER_ID --qr
```

Use the exact URL printed by the command.

## DNS points to the wrong server

```bash
dig +short vpn.example.com
```

It should return your VPS IP.

Otherwise, correct the DNS `A` record and wait for DNS caches to update.

## `sudo vpn` says `command not found`

Try:

```bash
sudo /usr/local/bin/vpn status
```

Some systems use a restricted sudo `PATH`.

## Port 8443 is already in use

Check:

```bash
sudo ss -lntp | grep :8443
```

Use another subscription port:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --subscription-port 8444 \
    --reality-handshake-server www.cloudflare.com
```

Also open TCP/8444 in your provider firewall.

## Port 443 is already in use

```bash
sudo ss -lntup | grep :443
```

REALITY needs TCP/443 and Hysteria2 uses UDP/443.

Stop or reconfigure the conflicting service before installation.

## Installation stopped halfway through

Run:

```bash
sudo vpn doctor
```

If `vpn` is not available yet:

```bash
systemctl status sing-box vpn-subscription
journalctl -u sing-box -u vpn-subscription --no-pager -n 100
```

Fix the reported DNS, network or firewall problem and rerun the installer.

# Security

If a subscription URL leaks, then rotate the subscription token:

```bash
sudo vpn user rotate-token USER_ID --qr
```

To immediately revoke a user:

```bash
sudo vpn user disable USER_ID
```

This project does not guarantee:

- Tor-style anonymity
- protection from a compromised VPS
- access from every country or network
- protection against VPS IP blocking
- protection after credentials are leaked

The VPS provider still controls the underlying server infrastructure.

# Supported configuration

```text
Server       AlmaLinux 9 x86-64
Topology     One VPS
Users        ≤10 trusted users
Primary      VLESS + REALITY / TCP 443
Secondary    Hysteria2 / UDP 443
Data plane   sing-box
Clients      Hiddify / sing-box compatible
Domain       Custom domain
```

# Documentation

- [Supported product boundary](docs/SUPPORTED_PRODUCT.md)
- [AlmaLinux deployment runbook](docs/ALMALINUX_DEPLOYMENT.md)
- [Client setup](docs/clients/README.md)
- [Device acceptance status](docs/DEVICE_ACCEPTANCE_TESTS.md)
- [Security model](docs/SECURITY_MODEL.md)
- [Recovery](docs/RECOVERY.md)

# License

Apache License 2.0. See [LICENSE](LICENSE).
