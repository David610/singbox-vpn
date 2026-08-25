# Server production baseline

Status as of 2026-08-22. This is the canonical evidence summary for the
server boundary; `docs/SUPPORTED_PRODUCT.md` remains the support-tier contract.

## Production path — VERIFIED

```text
verified bootstrap / transactional installer
  -> vpn-admin (user and credential state; candidate render/check/apply)
  -> upstream sing-box 1.13.19 (TCP/443 VLESS+REALITY, UDP/443 Hysteria2)
  -> vpn-subscription (127.0.0.1:9100)
  -> nginx (HTTPS subscription endpoint, default TCP/8443)
```

The installer and release workflow build only `admin` and `subscription`.
Their local dependency closure is `common` plus `compat-config`. Root-level
`cargo build` now has the same production boundary through Cargo
`default-members`. `cargo test --workspace` remains deliberately broader and
continues testing the experimental code; this is not evidence that it ships.

## Component classification

| Component | Status | Installed/started | Decision |
|---|---|---|---|
| `apps/admin` | PRODUCTION | `/usr/local/bin/vpn-admin` | KEEP |
| `services/subscription` | PRODUCTION | `vpn-subscription.service` | KEEP |
| `crates/common`, `crates/compat-config` | PRODUCTION | linked into the two binaries | KEEP |
| upstream `sing-box` | PRODUCTION DATA PLANE | `sing-box.service` | KEEP PINNED |
| nginx and deployment scripts/units | PRODUCTION ORCHESTRATION | yes | KEEP |
| `apps/client-daemon` | EXPERIMENTAL | no; local dev slice only | ISOLATE |
| `crates/transport-native`, `policy`, `failure-classifier`, `network-state`, `transport-api`, `rendezvous-client`, `telemetry` | EXPERIMENTAL | no | ISOLATE |
| `services/rendezvous`, `services/relay-agent` | EXPERIMENTAL | no; local dev slice only | ISOLATE |
| `services/test-service`, workspace `tests` crate | TEST-ONLY | no | KEEP TEST-ONLY |
| `apps/cli`, `apps/keytool`, `crates/config`, `crates/crypto` | EXPERIMENTAL native-stack tooling | no | ISOLATE |

Neither supported VPN transport, `vpn-admin`, the subscription service,
Hiddify, nor the release archive depends on the native adaptive stack.
There is insufficient cross-repository evidence here to claim what a future
`singbox-client` implements; that must be established by the versioned contract
phase. Source is retained for explicit development rather than deleted, but it
is excluded from the default build.

## Server/client responsibility boundary — CODE-VERIFIED

### Current implementation

The server owns the endpoint host and port, available transport type, VLESS
UUID, Hysteria2 password, REALITY public connection parameters, TLS/SNI
connection parameters, profile/endpoint labels, and subscription bearer-token
authentication. The native JSON subscription additionally emits a manual
selector, its default hint, a sing-box `urltest` option, and `route.final`
pointing to that selector. URI subscriptions emit only the individual
transport links; they have no selector or route semantics.

The renderer deliberately emits **no** DNS block, inbound/TUN configuration,
MTU, IP-family policy, per-application rules, or `route.rules`. Consequently
the client application and client OS own TUN lifecycle, DNS policy, IPv4/IPv6
routing, MTU, kill-switch semantics, per-app routing, network handover, and the
actual selection/failover behavior. The server's selector/default fields are
hints inside a generated document, not proof that an importing client retains
or follows them. This boundary is protected by renderer regression tests; real
client behavior remains DEVICE-UNVERIFIED.

### Future contract target

A future first-party mobile contract may formalize more of the client-owned
behavior, but no such contract ships here. Future architecture must not be
described as current implementation. This repository makes no claim about the
separate `singbox-client` repository.

## Credentials and revocation — VERIFIED

* Subscription tokens are CSPRNG bearer credentials, displayed once, stored
  only as SHA-256 hashes, compared in constant time, and invalidated by token
  rotation, disable, expiry, or removal. Rotation does not revoke an already
  downloaded tunnel profile.
* VLESS UUIDs and per-user Hysteria2 passwords are independent tunnel
  credentials. Disable/removal renders active users only, validates a candidate
  with pinned sing-box, activates/reloads it, and only then publishes the new
  user store. Thus both downloaded credentials stop authenticating.
* Raw token recovery is intentionally impossible. QR/URL recovery rotates the
  token. Backups contain token hashes but do contain live tunnel credentials,
  REALITY private material, and TLS-related state; a backup is a master
  credential bundle and must remain mode 0600.

## Control/data plane — CODE READY, EXTERNAL DEPLOYMENT NOT MIGRATED

`public_host` identifies VPN endpoints; `subscription_host` independently
identifies nginx TLS and generated subscription URLs, defaulting to the public
host for compatibility. Separate names can therefore be configured today, but
the standard installer still puts both planes on one VPS/IP. Blocking that IP
still removes both VPN and refresh availability. A separate VPS/static or
serverless publisher needs a future deployment design; token revocation and
fresh rendering argue against an unmanaged long-lived CDN cache.

nginx certificate identity and REALITY camouflage are independent TLS systems.
The subscription certificate provides no REALITY camouflage benefit.

## Known limitations and deferred work

* **KNOWN ISSUE:** GitHub-hosted checksums detect corruption but do not
  independently authenticate a publisher that can replace artifact and digest.
  New releases also require GitHub artifact provenance; its remaining trust
  assumptions and the explicit historical-release override are documented in
  `docs/SUPPLY_CHAIN_SECURITY.md`.
* **KNOWN ISSUE:** `curl | sudo bash` initially trusts HTTPS and GitHub account
  security. Mutable dev source now requires the two explicit opt-ins
  `SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1`.
* **NOT VERIFIED:** no disposable VPS or dated Russia real-device run was
  available for this phase. The canonical current evidence ledger is
  `docs/DEVICE_ACCEPTANCE_TESTS.md`; server probes are server facts, never
  proof of app behavior on a Russian mobile network.
* **DEFERRED:** shared `singbox-vpn`/`singbox-client` schema and compatibility
  matrix, external subscription deployment, transport changes, failover, and
  mobile/device acceptance.
