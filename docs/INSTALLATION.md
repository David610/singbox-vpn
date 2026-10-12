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

**Bootstrap prerequisites** — `bash`, `curl`, and `tar` must already be on
the VPS before the `curl -fsSL ... | bash` install commands below can run
at all; the installer sets up everything else on the host (packages,
firewall, systemd units, certificates), but it cannot install the very
tool used to fetch and execute it. These three ship by default on every
supported distribution's standard cloud image, but a minimal/hardened or
custom image can omit `curl` in particular — if the install command below
fails with `curl: command not found`, install it first
(`dnf install -y curl` on AlmaLinux/RHEL-family, `apt-get install -y curl`
on Ubuntu/Debian) and re-run.

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
IPv6 is configured and working: a broken IPv6 path stalls clients and
certificate issuance, and some hosts lose IPv6 neighbour discovery after a
reboot. One `A` record is all the VPN needs; both certificates and the
subscription use that single name.

- For an internationalized domain (for example a Cyrillic one), pass the
  ASCII/punycode form to the installer (`vpn.xn--...`).
- If the zone has a `CAA` record, it must allow `letsencrypt.org`.
- Records for other purposes on the same zone (website apex, mail) are not
  used by the VPN and should be left alone.

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
  | sudo SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

### Preview without installing (`--dry-run`)

Runs the same preflight checks (OS, architecture, memory, disk, SSH port,
port conflicts on 443/tcp+udp and the subscription port, TCP/80 for ACME,
firewall backend state, certbot presence, DNS for `--domain`, and a live
TLS 1.3 probe of `--reality-handshake-server`) and prints a PASS/WARN/FAIL
report. Makes no changes to the host, regardless of whether checks pass:

```bash
... | sudo bash -s -- --dry-run \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

A WARN (e.g. the firewall backend isn't active yet, or no `--domain` was
given) does not block installation — `--dry-run` still ends with `READY TO
INSTALL` — but is shown so nothing about what a real run would do is
hidden. A FAIL (e.g. a required port is already in use, or `--domain`
doesn't resolve to this host) ends with `NOT READY` and a non-zero exit
code. Note this only ever reports on your host's own firewall; it cannot
see or configure your cloud/VPS provider's separate network firewall (see
[Provider firewall](#provider-firewall) above).

### Custom SSH port

The installer never activates the firewall before your real SSH port is
positively identified and explicitly allowed. Pass it if auto-detection is
inconclusive:

```bash
... | sudo bash -s -- --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com \
    --ssh-port 2222
```

### Node role and identity

Every installation has a stable `node_id` and a `role`:

- `--role exit` (default) — an ordinary VPN exit; identical to every earlier
  installation.
- `--role relay` — a first hop that forwards authenticated traffic only to
  exits declared in `deployment.toml`, and nothing until one is paired. See
  [ALMALINUX_DEPLOYMENT.md](ALMALINUX_DEPLOYMENT.md#relay-nodes-two-hop-routes).
- `--node-id ID` — 1–63 of `A-Z a-z 0-9 . _ -`, starting with a letter or
  digit. Default: the first DNS label of the public hostname with unsupported
  characters replaced by `-` (for example `de1.example.com` → `de1`).

Both are written once. A repair or update keeps them and refuses a
conflicting flag; deployments installed before roles existed are migrated to
`role = "exit"` with the default node id.

### Profile name shown in Hiddify

Every successful subscription response carries a `profile-title` header so
Hiddify and other importers name the profile **Arcana VPN** for every user.
Without it they invent a name from the URL (the punycode host or the token).
To use a different name, set it in `/etc/vpn/deployment.toml` and restart the
subscription service:

```toml
[subscription]
profile_title = "My VPN"   # 1-64 characters, no control characters
```

```bash
systemctl restart vpn-subscription
```

Clients show the new name after their next profile update. A name that is not
plain ASCII is sent base64-encoded (`base64:...`), which Hiddify decodes.

### Custom subscription port

```bash
... | sudo bash -s -- --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com \
    --subscription-port 8444
```

Open the matching TCP port in your provider firewall too.

A successful installation prints the first user's subscription URL and QR
code. See the top-level `README.md` for the short Hiddify connection steps.

## YouTube Shorts and your exit's IP

YouTube refuses Shorts video to some datacenter IPs (the player says "OK",
then shows "Video unavailable"; long videos still play). This is decided by the
exit's IP address, so no protocol or client setting can fix it. A fresh
install is a plain single exit and has no workaround: test Shorts through it
first (`docs/YOUTUBE_FIX_INVARIANTS.md` documents the browser probe,
`deploy/lib/youtube_shorts_probe.py`).

If your exit's IP is refused and you operate a second node whose IP YouTube
accepts, the exit can send only the Google/YouTube domain set through a
dedicated static user on that node. On the **exit**:

```bash
printf '%s\n' "$HAIRPIN_UUID" | sudo vpn-admin google-egress-hairpin set \
  --relay-host NODE_B_IP --relay-port 443 \
  --relay-server-name NODE_B_REALITY_SERVER_NAME \
  --relay-reality-public-key NODE_B_REALITY_PUBLIC_KEY \
  --relay-reality-short-id NODE_B_SHORT_ID \
  --uuid-stdin
sudo vpn doctor --protocol --require-protocol
```

Use a dedicated user on the other node for `HAIRPIN_UUID`, never a real
client's credential. `set` validates the candidate config with `sing-box
check`, applies it, and rolls back on failure. To remove it and return to a
plain single exit:

```bash
sudo vpn-admin google-egress-hairpin clear
```

The rendered rules, ordering and the things that must never change (no
WARP, fail closed, IPv6 reject) are in
[YOUTUBE_FIX_INVARIANTS.md](YOUTUBE_FIX_INVARIANTS.md).

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
added (tracked via `/var/lib/singbox-vpn/firewall-owned.env`), and restores
firewalld/ufw to whatever enabled/disabled state they were in before
installation if singbox-vpn was the one that activated them.

Re-apply firewall rules manually if needed:

```bash
sudo ./deploy/almalinux/firewall.sh       # RHEL family (firewalld)
sudo ./deploy/almalinux/firewall-ufw.sh   # Debian family (ufw)
sudo firewall-cmd --list-all              # or: sudo ufw status verbose
```

## TLS / Certbot

The default single-hostname install uses one certificate lineage for nginx
and a restricted copy of that certificate/key for Hysteria2. If the optional
`subscription_host` differs from `public_host`, separate names/lineages are
needed. REALITY still uses its public decoy and needs no certificate.

| | Hysteria2 TLS | Subscription HTTPS |
|---|---|---|
| Hostname (default) | `vpn.example.com` | `vpn.example.com` |
| Optional separate name | `vpn.example.com` | `sub.example.com` |
| Port | `443/udp` | `8443/tcp` |
| Consumed by | `sing-box` directly | `nginx` (reverse proxy) |

VLESS+REALITY needs **no certificate** — it dials a real site's TLS
handshake as a disguise. Required certificate names are auto-issued via
certbot's HTTP-01 challenge if no domain-specific certificate already
exists, which requires TCP/80 to be reachable from the public internet
for the few seconds each challenge takes — both at the initial install
**and** at every renewal, since certbot's renewal timer re-runs the
HTTP-01 challenge roughly every 60 days for the life of the deployment.

The installer installs a pair of certbot pre/post renewal hooks
(`/etc/letsencrypt/renewal-hooks/{pre,post}/singbox-vpn-firewall.sh`)
that reopen TCP/80 at the **host** firewall (`firewalld`/`ufw`) around
every renewal attempt and close it again afterward — no manual step or
permanently-open port needed at the host level. If your VPS sits behind
a separate cloud-provider security group (AWS/GCP/Azure/etc.), that
layer is **not** managed by singbox-vpn and needs its own permanent
inbound TCP/80 allow rule, since there's no way to know in advance
exactly when the timer will next fire — see
`docs/ALMALINUX_DEPLOYMENT.md` "Cloud provider firewalls / security
groups". Confirm renewal works end to end with:

```bash
sudo certbot renew --dry-run
```

A renewal hook (`/etc/letsencrypt/renewal-hooks/deploy/singbox-vpn-hysteria.sh`)
refreshes the Hysteria2 certificate copy, reloads and verifies `sing-box`,
and reloads nginx after every successful renewal — no manual step needed.

## Updating / repairing an installation

```bash
sudo /opt/singbox-vpn/deploy/almalinux/update.sh --latest
sudo /opt/singbox-vpn/deploy/almalinux/update.sh --version vX.Y.Z
sudo /opt/singbox-vpn/deploy/almalinux/update.sh --repair   # reconcile without changing versions
sudo vpn repair                                             # same as --repair, shorter to type
```

Updates download and checksum-verify the target release before changing
live state, validate the re-rendered configuration, restart services, run
protocol health checks, and automatically roll back the complete
transaction on failure. A no-argument update is rejected so a version
change never happens by accident.

### Updating a host that is still on v1.1.0

`update.sh` runs from the **installed** release, so it only knows the systemd
units and acceptance steps of the release you already have. Stable v1.1.0
predates the egress-isolation unit (`vpn-egress-isolation.service`) and the
reserved probe principal, so v1.1.0's own updater cannot install a later
release: it rolls back safely (found updating a real VPS to v1.2.0-rc.1) but
never succeeds. Run the **target** release's updater instead, from its
verified source archive. Replace the tag with the release you want:

```bash
TAG=v1.2.0   # the release to install
B=https://github.com/David610/singbox-vpn/releases/download/$TAG
W=/root/singbox-vpn-updater; rm -rf "$W"; install -d -m 0700 "$W/dl" "$W/src"
cd "$W/dl"
curl -fsSL -o SHA256SUMS "$B/SHA256SUMS"
curl -fsSL -o singbox-vpn-src.tar.gz "$B/singbox-vpn-src.tar.gz"
grep ' singbox-vpn-src.tar.gz$' SHA256SUMS | sha256sum -c -    # must print OK
# the archive has a top-level singbox-vpn-src/ directory: strip it so deploy/ is at $W/src
cd "$W/src" && tar xzf ../dl/singbox-vpn-src.tar.gz --strip-components=1
bash deploy/almalinux/update.sh --version "$TAG"
```

The updater then verifies the release's attestations again and runs the same
transaction (verify, switch, health check, protocol self-test, roll back on
failure). Once a host runs a release that includes this fix, the normal
`/opt/singbox-vpn/deploy/almalinux/update.sh --version vX.Y.Z` command works
for later updates. If the node's `vpn-expiry-reconcile.timer` was stopped by
hand, start it first (`systemctl start vpn-expiry-reconcile.timer`): the
post-update health check fails while it is inactive.

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
sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes
```

Online fallback if `/opt/singbox-vpn` is missing or damaged:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/uninstall.sh \
  | sudo bash -s -- --yes
```

This removes everything singbox-vpn created — `/etc/vpn`, `/opt/singbox-vpn`,
`/var/lib/singbox-vpn`, its systemd units, the nginx vhost, the sing-box binary (if
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

## Exit literal IPv6 policy (unreleased)

See [EXIT_IPV6_POLICY.md](EXIT_IPV6_POLICY.md) for `vpn-admin egress set`,
rollback, local validation and the device evidence gate. Default plain exits
remain `allow`. `reject_google` is an explicit TLS/QUIC port-443 mechanism,
not guaranteed native YouTube IPv4 recovery. It cannot fix a refused IPv4.
No manual edits to rendered `config.json` or host IPv6 shutdown are needed.

### Newly created hostname: Failed host lookup

A router/ISP may cache NXDOMAIN from before the DNS record existed. Verify
the DNS-only A record at authoritative/public resolvers, then compare mobile
data with Wi-Fi and wait for the negative TTL or flush the router cache.
Changing the server AAAA affects client-to-VPS connectivity only. Do not
Cloudflare-proxy REALITY/Hysteria2. Keep TLS hostname validation: an IP HTTPS
URL with a hostname-only certificate is not a valid recovery path. `user
links` avoids subscription fetching, but its endpoint hostnames still need
DNS. Restoring valid DNS or using a client-supported address override with
the original verified TLS hostname is required; no general Hiddify override
is claimed. Rotate exposed historical credentials through the owner, never
copy them from old reports.

### nginx TCP/80 welcome site

Debian-family setup disables only the unmodified nginx package default-site
symlink, records ownership, and restores it on failed configuration/uninstall.
Administrator-modified or unrelated sites are retained; a doctor warning
requires an ownership-aware operator review, not a blanket listener exemption.
Standalone Certbot HTTP-01 and existing temporary firewall hooks are unchanged.
