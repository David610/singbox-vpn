# 01 - System inventory (2026-10-15)

Secrets redacted. Evidence files are in `test-results/`. Evidence classes follow
`CLAUDE.md`: code verified, server verified, browser verified, native device
verified, user reported, unverified hypothesis.

## Repositories

| Repo | Role | State at start | Used here |
|---|---|---|---|
| `David610/singbox-vpn` | server, subscriptions, provisioning, installer | `main` 56193f9; stable `v1.3.0` (2026-10-13); pre-release `v1.3.1-rc.1` | branches below |
| `David610/vpn-web` | portal, control plane, Cloudflare `/sub/` endpoint | `main` 55277ed | PR #83 branch |
| `David610/tamara-next` | current first-party client | `main` ccffa5d; open PR #47 (iOS PacketTunnel) | read-only |
| `David610/tamara` | archived legacy client | archived | not used |
| `David610/singbox-client` | historical, superseded | - | not used |

The three working checkouts in `D:\David610` were clean and on old topic
branches (`fix/c16-relay-target-isolation`, `preserve/dirty-2026-10-07`,
`fix/dynamic-revision-shape-guard`); they were not modified. All work happened in
separate worktrees under `D:\David610\.worktrees\m2026-10-15\`.

Branches produced (all pushed, none merged):

| Branch | Base | Head at end |
|---|---|---|
| `singbox-vpn:fix/shadowrocket-base64-subscription-link` | `main` 56193f9 | see `07_CHANGELOG_AND_REGRESSIONS.md` |
| `singbox-vpn:audit/2026-10-15-primary-client-contract` (PR #156) | `main` | e9e1d05 |
| `vpn-web:audit/2026-10-15-client-import-compat` (PR #83) | `main` 55277ed | 4970a82 |

## Two independent subscription implementations

They must not be confused; a fix in one does not repair the other.

| | singbox-vpn `services/subscription` | vpn-web `functions/sub/[token].js` |
|---|---|---|
| Host | the node itself, `https://<host>:8443/sub/<token>` (token 27 chars) | Cloudflare Pages, `https://<site>/sub/<token>` (token 43 chars) |
| Formats | `singbox` (default), `uri`/`hiddify` (identical), opt-in `encoding=base64`, `compat=...` | `hiddify`, `shadowrocket`, `incy`, `links`, `singbox`, `xray`; `encoding=base64` only for `shadowrocket` |
| Source of node data | the node's own key files | `nodes` table (one row per transport) + `node_transport_secrets` |
| Distinguish on a phone | port 8443, 27-char token | account site host, 43-char token |

## Deployed VPS (disposable)

- Address 91.244.71.165 / `2a12:bec4:19a5:9aa::` (IPv6 works, no AAAA record). rDNS `customer.evolus-ix.com`.
- Domain `vpn.у-тамары.com` = `vpn.xn----8sba1cvaif4e.com`, A record only, verified at 1.1.1.1 and 8.8.8.8. (A different, hyphen-less name `vpn.утамары.com` points at another host.)
- Ubuntu 24.04.5, kernel 6.8.0-45, 2 vCPU (AMD EPYC 9654), 3.8 GB RAM, 61 GB disk.
- On arrival: pristine image, only sshd listening, ufw inactive, no VPN software (`test-results/vps-baseline-2026-10-15.txt`). The original deployment that produced the reported errors no longer existed.
- sshd allowed password login for root; a dedicated key was added for the session and the host key was pinned in a separate known_hosts file.

Installs performed, in order:

1. Stable `v1.3.0` via the official installer (checksum + cosign attestation verified). Installed binaries match the release tarball (SHA-256).
2. Official `update.sh` to `v1.3.1-rc.1`, injected failure + rollback, reboot.
3. Official uninstall (`uninstall.sh --yes`), verified equal to the pristine baseline.
4. Fresh dev-channel install of branch `fix/shadowrocket-base64-subscription-link` at head `c1c0a7f` (see `08_RELEASE_ACCEPTANCE.md`).

Software on the candidate: sing-box 1.14.1 (pinned), nginx + certbot (Let's Encrypt, 89 days left), ufw (22/tcp, 443/tcp, 443/udp, 8443/tcp).
Services: `sing-box`, `vpn-subscription` (loopback :9100 behind nginx :8443), `vpn-egress-isolation`, `vpn-expiry-reconcile.timer`, `vpn-service-watchdog.timer`, `certbot.timer`.

## Protocol and data flow

```
client app --HTTPS--> nginx:8443 --> vpn-subscription:9100 --> share links / sing-box JSON
client app --TCP 443 REALITY(VLESS, Vision)--> sing-box --> direct egress
client app --UDP 443 Hysteria2 (Salamander)--> sing-box --> direct egress
```

REALITY decoy `www.cloudflare.com`; Hysteria2 certificate name = the node hostname.
The two names differ, which is exactly what the vpn-web renderer mixed up.

## Client software versions used

| Client | Version | Where |
|---|---|---|
| sing-box (reference client) | 1.14.1 (SagerNet release, digest checked) | Windows 11 |
| Hiddify core (built from source) | `dbf3ffb` (current main, 2026-10-10) and `v4.0.0` | Windows 11, CLI |
| Hiddify app source read | `83911ed` (2026-10-14) | read only |
| Google Chrome + Playwright | project probe `youtube_shorts_probe.py` | Windows 11, iPhone *emulation* |
| Hiddify iOS 4.0.0 dev, Shadowrocket, INCY | - | **not available to the engineer** |

Latest published Hiddify app: v4.1.1 (2026-03-05); the user's screenshot showed
"4.0.0 dev", i.e. an older client than the current server output was designed for.
