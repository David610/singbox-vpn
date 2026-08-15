# singbox-vpn

A simple self-hosted VPN for a small group of trusted users.

Install it on one VPS, get a subscription URL or QR code, import it into Hiddify, and connect.

The server uses:

* **VLESS + REALITY** over TCP/443 as the primary transport
* **Hysteria2** over UDP/443 as an optional secondary transport
* **sing-box** as the VPN data plane
* **Hiddify-compatible subscriptions**
* automatic TLS certificate setup
* built-in user management, diagnostics, backup and restore
* complete offline uninstall

No custom client application is required.

> This project is intended for private use by small trusted groups. It is not an anonymity network and does not provide Tor-like anonymity guarantees.

> The supported v1.0 code path is covered by automated checks, but a complete
> real-VPS and real-device acceptance run is still outstanding. Do not treat
> this repository as device-verified; see
> [Supported product](docs/SUPPORTED_PRODUCT.md) and
> [Device acceptance tests](docs/DEVICE_ACCEPTANCE_TESTS.md).

---

## Quick start

### Requirements

For the supported setup you need:

* a VPS with **AlmaLinux 9 x86-64**
* root or sudo access
* a public IPv4 address
* approximately 1 GB RAM or more
* a domain or subdomain pointing to the VPS
* Hiddify (recommended); the client guides also document a limited v2rayNG
  Android fallback

A custom domain is recommended.

Example:

```text
vpn.example.com → 203.0.113.10
```

---

## 1. Configure your VPS firewall

The VPN uses the following public ports:

| Port                | Protocol | Purpose                            |
| ------------------- | -------- | ---------------------------------- |
| 22 or your SSH port | TCP      | SSH administration                 |
| 80                  | TCP      | TLS certificate validation/renewal |
| 443                 | TCP      | VLESS + REALITY                    |
| 443                 | UDP      | Hysteria2                          |
| 8443                | TCP      | HTTPS subscription endpoint        |

If you change the subscription port during installation, allow that port instead of `8443`.

The installer opens the VPS's operating-system firewall for TCP/80 only
temporarily during initial HTTP-01 certificate issuance. Certbot renewals need
TCP/80 again. On the supported AlmaLinux target, keep it open after a
successful install unless you configure another ACME challenge method:

```bash
sudo firewall-cmd --permanent --add-service=http
sudo firewall-cmd --reload
sudo certbot renew --dry-run
```

The provider firewall or AWS Security Group must also allow TCP/80. DNS-01 is
an alternative, but this repository does not configure it automatically.

### AWS EC2

If you use AWS, configure the instance's **Security Group**.

Recommended inbound rules:

```text
SSH          TCP   22     YOUR_IP/32
HTTP         TCP   80     0.0.0.0/0
REALITY      TCP   443    0.0.0.0/0
Hysteria2    UDP   443    0.0.0.0/0
Subscription TCP   8443   0.0.0.0/0
```

Replace port `22` if your SSH server uses another port.

The installer configures the firewall **inside the VPS**, but it cannot modify AWS Security Groups, provider firewalls, router firewalls or other external firewalls.

This is one of the most common reasons for an installation that looks healthy on the server but cannot be reached from a client.

---

## 2. Configure DNS

Create an `A` record:

```text
vpn.example.com → YOUR_VPS_IPV4
```

Verify it:

```bash
dig +short vpn.example.com
```

or:

```bash
nslookup vpn.example.com
```

The returned IP should match your VPS.

### Cloudflare users

The VPN hostname must normally be:

```text
DNS only
```

not:

```text
Proxied
```

In the Cloudflare dashboard this means the cloud should be **grey**, not orange.

REALITY and Hysteria2 need a direct connection to the VPS. Do not put the VPN hostname behind the normal Cloudflare HTTP proxy.

Do not create an `AAAA` record unless IPv6 is actually configured and working on the VPS.

---

## 3. Install

Once a tagged release has been published, run:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

Replace:

```text
vpn.example.com
```

with your own hostname.

The REALITY handshake server shown above is only an example. Its suitability can vary between networks.

The stable installer resolves an immutable tagged release and refuses to fall
back silently to mutable branch source. **No release tag exists yet, so the
stable command above currently exits without changing the host.** Publishing
the first release is a separate release-readiness decision and is not part of
this repository migration.

For development and disposable-host testing only, explicitly opt into the
unverified `main` branch:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo VPN1_CHANNEL=dev bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

This development command is not reproducible and must not be presented as a
production install. When a release exists, the stable installer configures the
server, validates the configuration, creates the initial user and prints the
user's subscription URL and QR code.

---

## 4. Connect with Hiddify

Install Hiddify on your phone or computer.

After installation completes, the server prints a subscription URL and QR code.

In Hiddify:

1. Open **New Profile**
2. Scan the QR code or paste the subscription URL
3. Import the profile
4. Select **REALITY** or **Hysteria2**
5. Connect

Then check your public IP.

It should change to the public IP of your VPS.

If Hiddify says "connected" but your IP does not change, see [Troubleshooting](#troubleshooting).

---

# User management

## List users

```bash
sudo vpn user list
```

Commands use the user's **ID**, not their display name.

Example:

```text
user_dd466bb1-...
```

---

## Add a user

```bash
sudo vpn user create --name alice --qr
```

This prints the new user's subscription URL and QR code.

Treat the subscription URL like a password.

Anyone who has it can obtain that user's connection configuration.

---

## Reprint / replace a subscription token

First find the user:

```bash
sudo vpn user list
```

Then:

```bash
sudo vpn user rotate-token USER_ID --qr
```

---

## Disable a user

```bash
sudo vpn user disable USER_ID
```

Enable again:

```bash
sudo vpn user enable USER_ID
```

---

## Remove a user

```bash
sudo vpn user remove USER_ID
```

Removal is destructive.

---

# Check server health

Run:

```bash
sudo vpn status
```

For detailed diagnostics:

```bash
sudo vpn doctor
```

If you suspect a protocol problem:

```bash
sudo vpn doctor --protocol --require-protocol
```

Check services:

```bash
systemctl status sing-box
systemctl status vpn-subscription
```

Recent logs:

```bash
journalctl -u sing-box -u vpn-subscription --no-pager -n 100
```

---

# Troubleshooting

## Hiddify shows `timeout`, but the VPN still works

Do not assume the VPN is broken only because a latency or connectivity test in Hiddify reports a timeout.

First verify the actual tunnel:

1. Connect.
2. Open a website.
3. Check your public IP.
4. Confirm it changed to your VPS IP.

If normal traffic works and the public IP changed, the important VPN path is working even if a Hiddify health/latency test failed.

Try selecting **REALITY** manually instead of `auto`.

Then test **Hysteria2** separately.

Do not rotate server keys or reinstall the server solely because a Hiddify latency test says `timeout` while real traffic is working.

---

## Hiddify says connected, but my public IP does not change

On iOS in particular, check that Hiddify is actually using VPN/TUN mode rather than a proxy-only mode.

Also verify that iOS granted Hiddify permission to create a VPN configuration.

Check:

```text
Settings
→ General
→ VPN & Device Management
```

A Hiddify VPN configuration should exist.

Then reconnect and check your public IP again.

---

## REALITY works but Hysteria2 does not

REALITY uses:

```text
TCP/443
```

Hysteria2 uses:

```text
UDP/443
```

Check that UDP/443 is allowed by:

* your VPS provider
* AWS Security Groups or another cloud firewall
* the operating-system firewall
* the client network

On AWS, having TCP/443 open does **not** automatically mean UDP/443 is open.

They require separate Security Group rules.

If REALITY works reliably, you can continue using it even if Hysteria2 is unavailable on a particular network.

---

## Hysteria2 works but REALITY does not

First test TCP/443 from another machine:

### Windows

```powershell
Test-NetConnection vpn.example.com -Port 443
```

### Linux/macOS

```bash
nc -vz vpn.example.com 443
```

Then check:

```bash
sudo vpn doctor --protocol --require-protocol
```

and:

```bash
journalctl -u sing-box --no-pager -n 100
```

Also verify that the REALITY handshake server you selected is still reachable and compatible.

---

## Nothing can connect from AWS

Check the AWS Security Group before changing the VPN configuration.

At minimum verify:

```text
TCP/443
UDP/443
TCP/8443
```

and your actual SSH port.

If TLS certificate creation fails, also verify TCP/80.

The VPS operating-system firewall and the AWS Security Group are separate layers.

Both must allow the required traffic.

---

## Subscription URL times out

Test it directly:

```bash
curl -v https://vpn.example.com:8443/healthz
```

Then check whether the port is listening:

```bash
sudo ss -lntup
```

And run:

```bash
sudo vpn doctor
```

If you use AWS or another cloud provider, verify TCP/8443 is also open in the provider firewall.

If you configured another subscription port, replace `8443` everywhere with that port.

---

## Subscription URL returns 404

Do not manually construct the URL using the user's ID.

A user ID such as:

```text
user_1234...
```

is not the subscription token.

Get a fresh subscription URL with:

```bash
sudo vpn user list
sudo vpn user rotate-token USER_ID --qr
```

Use the exact URL printed by the command.

---

## DNS points to the wrong server

Check:

```bash
dig +short vpn.example.com
```

The returned address must match the public IP of your VPS.

If it does not:

1. correct the DNS record
2. wait for DNS propagation/cache expiry
3. test again

If using Cloudflare, make sure the VPN record is **DNS only**.

---

## Cloudflare is enabled and nothing connects

Check the DNS record.

If it has an orange cloud, disable proxying.

Use:

```text
DNS only
```

for the VPN hostname.

Then wait briefly for DNS to update and test again.

---

## `sudo vpn` says `command not found`

Some systems use a restricted sudo `PATH`.

Try:

```bash
sudo /usr/local/bin/vpn status
```

If that works, the VPN installation itself is present.

---

## Port 8443 is already in use

Check:

```bash
sudo ss -lntp | grep :8443
```

You can install with another subscription port:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --subscription-port 8444 \
    --reality-handshake-server www.cloudflare.com
```

Remember to also open that port in your VPS provider firewall.

---

## Port 443 is already in use

Check:

```bash
sudo ss -lntup | grep :443
```

VLESS+REALITY requires TCP/443 in the supported configuration, and Hysteria2 uses UDP/443.

Stop or reconfigure the conflicting service before installing.

---

## Installation stopped halfway through

The installer is designed to be repair-safe.

First run:

```bash
sudo vpn doctor
```

If the installation did not get far enough to install `vpn`, inspect:

```bash
systemctl status sing-box
systemctl status vpn-subscription
journalctl -u sing-box -u vpn-subscription --no-pager -n 100
```

After fixing the underlying network/DNS/firewall issue, run the installation command again.

---

# Backup

Create a backup before major changes:

```bash
sudo vpn backup
```

Follow the path printed by the command and store the backup securely.

It contains sensitive VPN configuration.

---

# Uninstall

A normal installation includes a complete offline uninstaller.

Run:

```bash
sudo /opt/vpn1/bin/vpn1-uninstall --yes
```

This removes the files, credentials, services and firewall changes created by the VPN installer.

If `/opt/vpn1` is missing or damaged, use the online recovery fallback:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh \
  | sudo bash -s -- --yes
```

The uninstall process cannot remove rules from external cloud firewalls such as AWS Security Groups.

Remove those separately if you no longer need them.

---

# Security

## Protect subscription URLs

A subscription URL is a credential.

Do not:

* post it in GitHub issues
* publish screenshots containing it
* commit it to a repository
* send it through public chats
* include it in logs when opening a bug report

If a subscription URL leaks:

```bash
sudo vpn user rotate-token USER_ID --qr
```

If you need to stop an already-imported profile immediately:

```bash
sudo vpn user disable USER_ID
```

---

## What this VPN does not provide

This project does not claim to provide:

* Tor-style anonymity
* protection from a compromised VPS
* protection from every censorship system
* guaranteed access from every country or ISP
* protection if a user's VPN credentials are leaked
* automatic protection against the VPS IP itself being blocked

The VPS provider still controls the server infrastructure.

Use a provider you trust.

---

# Supported configuration

The supported v1.0 target is intentionally narrow:

```text
Server:       AlmaLinux 9 x86-64
Topology:     One VPS
Users:        Up to 10 trusted users
Primary:      VLESS + REALITY / TCP 443
Secondary:    Hysteria2 / UDP 443
Data plane:   sing-box
Clients:      Hiddify / compatible sing-box clients
Domain:       Custom domain recommended
```

Keeping the supported configuration narrow makes installation and troubleshooting easier to reproduce.

---

# Advanced documentation

More detailed information is available under `docs/`:

* [supported v1.0 product boundary](docs/SUPPORTED_PRODUCT.md)
* [AlmaLinux deployment runbook](docs/ALMALINUX_DEPLOYMENT.md)
* [architecture](docs/ARCHITECTURE.md)
* [security model](docs/SECURITY_MODEL.md) and [threat model](docs/THREAT_MODEL.md)
* [privacy model](docs/PRIVACY_MODEL.md)
* [client setup](docs/clients/README.md)
* [recovery](docs/RECOVERY.md)
* [protocol troubleshooting](docs/TELEGRAM_TROUBLESHOOTING.md)
* [AWS/network reachability testing](docs/AWS_REACHABILITY_TEST.md)
* [implementation status](docs/IMPLEMENTATION_STATUS.md) and
  [device acceptance status](docs/DEVICE_ACCEPTANCE_TESTS.md)

---

# License

Apache License 2.0.

See [LICENSE](LICENSE).

---

# Contributing

Bug reports and tested fixes are welcome.

When opening an issue, include:

* server OS
* VPS provider
* client operating system
* Hiddify/client version
* whether REALITY works
* whether Hysteria2 works
* output of `sudo vpn doctor`
* relevant service errors

Never include:

* subscription URLs
* VLESS UUIDs
* Hysteria2 passwords
* REALITY private keys
* other VPN credentials
