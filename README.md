<div align="center">

# singbox-vpn

**Your own VPN server, without building your own VPN stack.**

Deploy on a VPS → import the provisioning URL in Tamara → connect.

<br>

`VLESS + REALITY` &nbsp; `Hysteria2` &nbsp; `sing-box`

<br>

![Platform](https://img.shields.io/badge/server-Linux-222?style=flat-square)
![Users](https://img.shields.io/badge/designed%20for-%E2%89%A410%20users-222?style=flat-square)
![Self-hosted](https://img.shields.io/badge/self--hosted-yes-222?style=flat-square)

</div>

### One locally managed server, optional independent peers

```text
┌────────────────────┐       ┌─────────────────┐       ┌─────────────┐
│ singbox-vpn VPS    │       │ provisioning    │       │   Tamara    │
│ + optional static │ ─────► │ URL / contract  │ ─────►│ phone / PC  │
│ peer endpoints    │       │                 │       │             │
└────────────────────┘       └─────────────────┘       └─────────────┘
```

- **VLESS + REALITY** over TCP/443
- **Hysteria2** over UDP/443
- optional operator-declared endpoints on independently managed peer servers
- automatic TLS
- user management
- backup and restore
- complete offline uninstall

No web control panel is required.

### Clients: one contract, two tiers

The server owns endpoints and credentials; the client owns everything
about the device (DNS, TUN, MTU, IP family, kill switch). That split is
written down once, as a versioned contract, and every client-facing
output is generated from it.

| Tier | Client | Gets |
|---|---|---|
| **Primary** | [Tamara](https://github.com/David610/tamara) — first-party, separate repo | `GET /v1/provision/{token}` — endpoint metadata plus the embedded Core-consumable config |
| **Fallback** | Hiddify and other sing-box-compatible importers | `GET /sub/{token}` — share links or native sing-box JSON, unchanged |

Both are rendered from the same endpoint model, so they cannot disagree
about a user's credentials. The contract, its versioning rules, and the
cross-repo test fixtures are in
**[docs/PROVISIONING_CONTRACT.md](docs/PROVISIONING_CONTRACT.md)**.
The historical `singbox-client` repository is superseded; its name remains
only in fixture paths where renaming would add churn without changing the
contract. Only device-verified behaviour is claimed for fallback clients — see
[docs/CLIENT_COMPATIBILITY.md](docs/CLIENT_COMPATIBILITY.md).

> Built for small groups of users. Static peer-endpoint provisioning is
> implemented and fixture/local tested, but no real second VPS has been used
> to verify peer failover yet. See [Provisioning contract](docs/PROVISIONING_CONTRACT.md)
> and [Device acceptance tests](docs/DEVICE_ACCEPTANCE_TESTS.md).

## Supported servers

| Distribution | Tier |
|---|---|
| AlmaLinux 9 x86_64 | **Supported** |
| Rocky Linux 9, RHEL 9, CentOS Stream 9 | Recognized / best-effort |
| Ubuntu 22.04 / 24.04 LTS, Debian 12 / 13 | Recognized / best-effort |
| Amazon Linux 2023 | CI-tested |

Full matrix and evidence: [docs/SUPPORTED_PRODUCT.md](docs/SUPPORTED_PRODUCT.md).

**Explicitly not supported:** a multi-node control plane/fleet manager,
automatic remote peer deployment or credential synchronization, custom VPN
protocols beyond the supported VLESS+REALITY/Hysteria2 data plane, and
Tor-class anonymity guarantees. Static independently managed peer endpoints
are an implemented provisioning extension; they are not a fleet manager.

## Requirements

- A supported VPS (see above), root or sudo access, public IPv4, ~1 GB RAM.
- A domain or subdomain pointing to the VPS.
- [Tamara](https://github.com/David610/tamara) for the first-party provisioning path, or a documented fallback client such as [Hiddify](https://hiddify.com).

**Bootstrap prerequisites** (must already be on the VPS *before* the
one-command install below can run at all — the installer sets up
everything else, but it cannot install its own means of being fetched
and executed):
- `bash`
- `curl`
- `tar`

These ship by default on virtually every mainstream AlmaLinux/RHEL/Ubuntu/Debian
cloud image, but a minimal/hardened or custom image can omit `curl` in
particular. If the one-liner below fails with `curl: command not found`,
install it first (e.g. `dnf install -y curl` / `apt-get install -y curl`)
and re-run.

## Install

### Guided one-command install

On a fresh supported VPS, run:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh | sudo bash
```

The installer performs preflight checks, asks only for required deployment
choices, verifies the selected stable release, and prints the client QR/URL
only after the installation health gates pass. The VPS, DNS record, and
provider/cloud firewall still have to exist before the server can be made
reachable from the Internet.

### Automated / non-interactive install

For reproducible automation, provide the required security-sensitive values
explicitly:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com \
    --non-interactive
```

### Preflight only

Check a host without making persistent changes:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com \
    --dry-run
```

DNS, provider firewall setup (AWS/Cloudflare), distribution-specific notes,
the trust boundary of `curl | sudo bash`, and every install flag are documented
in **[docs/INSTALLATION.md](docs/INSTALLATION.md)** and
**[docs/SUPPLY_CHAIN_SECURITY.md](docs/SUPPLY_CHAIN_SECURITY.md)**.

## Connect

### Tamara (primary)

Import the printed `/v1/provision/{token}` URL in Tamara. A recognized
provisioning profile consumes the endpoint catalog and embedded config; current
Tamara can keep an Automatic route or pin one concrete endpoint manually.

### Fallback clients

Hiddify and other documented sing-box-compatible clients continue to consume
`/sub/{token}` share links or sing-box JSON. See [docs/clients/README.md](docs/clients/README.md).

Native YouTube app fails on iOS while Safari works fine? See
[docs/clients/HIDDIFY_IOS.md](docs/clients/HIDDIFY_IOS.md).

## Commands

```bash
sudo vpn user create --name alice --qr   # create a user + QR
sudo vpn user list                       # list users
sudo vpn status                          # server status
sudo vpn doctor                          # diagnostics
sudo vpn backup                          # backup
sudo vpn repair                          # reconcile after drift, no version change
sudo /opt/singbox-vpn/deploy/almalinux/update.sh --latest  # update to the latest release
sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes      # complete offline uninstall
```

Full command reference, troubleshooting, updating, and credential rotation:
**[docs/INSTALLATION.md](docs/INSTALLATION.md)**.

## Security

This project does not guarantee Tor-style anonymity, protection from a
compromised VPS, access from every country/network, or protection after
credentials leak. See [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

The proposed reachable-first-hop work is design-only and is not part of the
current supported runtime. No relay/access-path schema or relay runtime is
shipped by this PR. CI and local tests can validate code/configuration mechanics;
they do not establish relay reachability or behavior on a censored network. See
[docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md](docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md).

## Documentation

- [Installation & operations](docs/INSTALLATION.md) — DNS, firewall,
  distribution notes, troubleshooting, updating, uninstall
- [Supported product boundary](docs/SUPPORTED_PRODUCT.md) — authoritative
  OS/scope matrix
- [Provisioning contract](docs/PROVISIONING_CONTRACT.md) — the versioned
  client/server contract and its schema
- [Reachable first-hop architecture](docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md) — Phase-2 design only
- [Client setup](docs/clients/README.md)
- [Device acceptance status](docs/DEVICE_ACCEPTANCE_TESTS.md)
- [Release policy](docs/RELEASE.md) — RC acceptance and stable-release gates
- [Threat model](docs/THREAT_MODEL.md)
- [Release and supply-chain security](docs/SUPPLY_CHAIN_SECURITY.md)
- [Recovery](docs/RECOVERY.md)

## License

Apache License 2.0. See [LICENSE](LICENSE).
