<div align="center">

# singbox-vpn

**Your own VPN server, without building your own VPN stack.**

Deploy on a VPS → scan the QR → connect with Hiddify.

<br>

`VLESS + REALITY` &nbsp; `Hysteria2` &nbsp; `sing-box`

<br>

![Platform](https://img.shields.io/badge/server-Linux-222?style=flat-square)
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

No web control panel is required.

### Clients: one contract, two tiers

The server owns endpoints and credentials; the client owns everything
about the device (DNS, TUN, MTU, IP family, kill switch). That split is
written down once, as a versioned contract, and every client-facing
output is generated from it.

| Tier | Client | Gets |
|---|---|---|
| **Primary** | [singbox-client](https://github.com/David610/singbox-client) — first-party, separate repo | `GET /v1/provision/{token}` — the versioned provisioning contract |
| **Fallback** | Hiddify and other sing-box-compatible importers | `GET /sub/{token}` — share links or native sing-box JSON, unchanged |

Both are rendered from the same endpoint model, so they cannot disagree
about a user's credentials. The contract, its versioning rules, and the
cross-repo test fixtures are in
**[docs/PROVISIONING_CONTRACT.md](docs/PROVISIONING_CONTRACT.md)**.
Only device-verified behaviour is claimed for the fallback tier — see
[docs/CLIENT_COMPATIBILITY.md](docs/CLIENT_COMPATIBILITY.md).

> Built for small groups of users. Release status: the supported path has
> an owner-reported smoke pass on a real AlmaLinux 9 VPS with Hiddify on an
> iPhone. See [Device acceptance tests](docs/DEVICE_ACCEPTANCE_TESTS.md).

## Supported servers

| Distribution | Tier |
|---|---|
| AlmaLinux 9 x86_64 | **Supported** |
| Rocky Linux 9, RHEL 9, CentOS Stream 9 | Recognized / best-effort |
| Ubuntu 22.04 / 24.04 LTS, Debian 12 / 13 | Recognized / best-effort |
| Amazon Linux 2023 | CI-tested |

Full matrix and evidence: [docs/SUPPORTED_PRODUCT.md](docs/SUPPORTED_PRODUCT.md).

## Requirements

- A supported VPS (see above), root or sudo access, public IPv4, ~1 GB RAM.
- A domain or subdomain pointing to the VPS.
- [Hiddify](https://hiddify.com).

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/David610/singbox-vpn/main/install.sh \
  | sudo bash -s -- \
    --domain vpn.example.com \
    --reality-handshake-server www.cloudflare.com
```

DNS, provider firewall setup (AWS/Cloudflare), distribution-specific notes,
and every install flag are documented in
**[docs/INSTALLATION.md](docs/INSTALLATION.md)**.

## Connect with Hiddify

1. Open **New Profile** and scan the printed QR code (or paste the
   subscription URL).
2. Select **REALITY** or **Hysteria2** and connect.
3. Check your public IP changed to the VPS IP.

Native YouTube app fails on iOS while Safari works fine? See
[docs/clients/HIDDIFY_IOS.md](docs/clients/HIDDIFY_IOS.md).

## Commands

```bash
sudo vpn user create --name alice --qr   # create a user + QR
sudo vpn user list                       # list users
sudo vpn status                          # server status
sudo vpn doctor                          # diagnostics
sudo vpn backup                          # backup
sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes  # complete offline uninstall
```

Full command reference, troubleshooting, updating, and credential rotation:
**[docs/INSTALLATION.md](docs/INSTALLATION.md)**.

## Security

This project does not guarantee Tor-style anonymity, protection from a
compromised VPS, access from every country/network, or protection after
credentials leak. See [docs/SECURITY_MODEL.md](docs/SECURITY_MODEL.md).

## Documentation

- [Installation & operations](docs/INSTALLATION.md) — DNS, firewall,
  distribution notes, troubleshooting, updating, uninstall
- [Supported product boundary](docs/SUPPORTED_PRODUCT.md) — authoritative
  OS/scope matrix
- [Provisioning contract](docs/PROVISIONING_CONTRACT.md) — the versioned
  client/server contract and its schema
- [Client setup](docs/clients/README.md)
- [Device acceptance status](docs/DEVICE_ACCEPTANCE_TESTS.md)
- [Security model](docs/SECURITY_MODEL.md)
- [Release and supply-chain security](docs/SUPPLY_CHAIN_SECURITY.md)
- [Recovery](docs/RECOVERY.md)

## License

Apache License 2.0. See [LICENSE](LICENSE).
