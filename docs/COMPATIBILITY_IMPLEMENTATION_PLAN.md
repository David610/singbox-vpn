# COMPATIBILITY_IMPLEMENTATION_PLAN.md

> **Historical planning snapshot.** The "current" and "missing" sections
> below describe the native-only repository before the Hiddify/sing-box
> compatibility stack was implemented. They are retained as design history,
> not as current feature or support claims. See
> `docs/SUPPORTED_PRODUCT.md` and `docs/IMPLEMENTATION_STATUS.md` for the
> current state.

## 1. Current architecture

A Rust workspace implementing a *native* adaptive censorship-resistant
client/relay system (at the time this plan was written; that stack and
its `docs/ARCHITECTURE.md`/`PLAN.md` have since been removed from
`main` — see `docs/SUPPORTED_PRODUCT.md` and the
`archive/native-adaptive-stack-2026` branch. `TASKS.md` remains, with
its own historical-document notice):

- `crates/{common,crypto,config}` — signed-bundle trust chain (offline
  root → release key → bundle key, ADR-0008).
- `crates/{transport-api,network-state,failure-classifier,policy}` — the
  transport-agnostic adaptive engine (scoring, quarantine, fallback).
- `crates/transport-native` — two real transports: `direct-tls` (TLS 1.3
  over TCP, rustls) and `noise-quic` (QUIC, quinn).
- `crates/rendezvous-client`, `services/rendezvous` — signed, expiring,
  partial relay-subset issuance.
- `services/relay-agent` — ingress/egress forwarding, combined or split.
- `apps/client-daemon` — wires the engine + transports into a local
  SOCKS5-ish loopback proxy.
- `apps/cli`, `apps/keytool` — diagnostics and offline key-ceremony tooling.
- `tests/`, `fuzz/` — e2e, failure-independence, property, and fuzz tests.

Everything above is a **custom wire protocol** requiring the native
`client-daemon` on the client device. There is no path today for a
consumer client (Hiddify, sing-box, v2rayNG) to connect to this system.

## 2. What already works

A real local vertical slice: client → ingress relay → egress relay → test
HTTP service, over either of two independently-failing transports, with
signed config, adaptive scoring/fallback, and a tested failure
classifier. This is real infrastructure, not a prototype to throw away.

## 3. What is missing

- No protocol a stock consumer client understands (VLESS, Hysteria2,
  etc).
- No server-side data plane that speaks those protocols (sing-box/Xray
  not present).
- No user/credential model for third-party clients (native system has no
  concept of "a UUID + password a random Android user pastes in").
- No subscription delivery mechanism (HTTP endpoint returning a client
  config) for third-party clients.
- No AlmaLinux production deployment tooling (installer, systemd units,
  firewalld rules, SELinux-aware operation) — only a local dev slice
  exists (`deploy/local/`).
- No compatibility-specific health/scoring integration in the control
  plane.

## 4. Proposed architecture

Add a second, parallel client-compatibility stack beside the existing
native one. Nothing native is removed or rewritten:

```
                 CONTROL PLANE (Rust, this repo)
        crates/config, crates/compat-config, apps/admin
                          │
        ┌─────────────────┴──────────────────┐
        │ native trust chain (unchanged)      │ compat user/subscription store
        │ root→release→bundle keys            │ (crates/compat-config: users.json,
        │ signed RelayBundle                  │  REALITY keys, sing-box config render)
        └─────────────────┬────────────────────┘
                          │
        ┌─────────────────┴──────────────────┐
        │ native client-daemon                │ Hiddify / sing-box / v2rayNG
        │ direct-tls / noise-quic              │ VLESS+REALITY (TCP/443)
        │                                       │ Hysteria2 (UDP/443)
        └─────────────────┬────────────────────┘
                          │
        ┌─────────────────┴──────────────────┐
        │ relay-agent (Rust, unchanged)        │ sing-box (external, supervised)
        └─────────────────┬────────────────────┴──────────┐
                          ▼                                 ▼
                        Internet                          Internet
```

`services/subscription` is the new bridge: it reads the same
`crates/compat-config` user store the `apps/admin` CLI writes, and serves
per-user subscription documents over HTTPS (via a reverse proxy — no
custom TLS termination is built).

## 5. Component boundaries

New:
- `crates/compat-config` — `CompatTransport`, `CompatEndpoint`,
  `CompatUser`, subscription-token hashing, VLESS/Hysteria2 URI
  rendering, sing-box JSON rendering, `CompatibilityBackend` trait +
  `SingBoxBackend` impl (validate via `sing-box check`, atomic apply).
- `apps/admin` (`vpn-admin`) — user lifecycle CLI over
  `crates/compat-config`'s store, plus `init`/`render-config`.
- `services/subscription` — axum HTTP service (loopback only), `/sub/{token}`.
- `deploy/almalinux/` — installer, systemd units, firewalld rules,
  templates, health check.

Unchanged (per spec §4, hard constraint): `transport-api`, `policy`,
`failure-classifier`, `rendezvous-client`/`services/rendezvous`, `config`
bundle verification, `transport-native`, `client-daemon`.

## 6. Security boundaries

- Compatibility credentials (VLESS UUID, Hysteria2 password, REALITY
  keys, subscription tokens) never enter `config::EndpointDescriptor` or
  the signed `RelayBundle` — spec §5 hard requirement. They live in
  `crates/compat-config` types and a separate on-disk store
  (`/etc/vpn/compat/users/users.json`).
  bundle-signing trust chain is a *config-integrity* mechanism, not a
  per-user-credential mechanism, and mixing the two would let a
  compromised rendezvous process leak third-party-client credentials it
  has no reason to hold.
- Subscription tokens: ≥128 bits CSPRNG entropy, stored only as a salted
  hash (`argon2` or `blake3` keyed hash — see crate for the exact
  choice), compared in constant time, generic 404 on miss (no
  enumeration signal).
- REALITY private key and Hysteria2 passwords: file mode 0600, directory
  mode 0700, never logged (reuses the `crypto::Secret<T>` pattern from
  the native stack — no `Debug`/`Display` on the wrapped value).
- `services/subscription` binds loopback only; a reverse proxy (documented,
  not built by us) terminates HTTPS on 8443.

## 7. Configuration flow

```
users.json (source of truth, root-readable, 0600)
        │
        ▼ vpn-admin (or subscription service, read-only)
CompatEndpoint + CompatUser → sing-box config.json.tmp
        │
        ▼
sing-box check -c config.json.tmp
        │
   ┌────┴────┐
   │ fail    │ succeed
   ▼         ▼
keep old   atomic rename → config.json → reload sing-box
```

Backups of the previous known-good `config.json` are kept
(`config.json.bak`) before every apply.

## 8. User provisioning flow

```
admin: vpn-admin user create --name X
  → generates VLESS uuid, Hysteria2 password, subscription token
  → writes users.json (token stored as hash only)
  → renders + validates + atomically applies sing-box config
  → prints subscription URL once (not re-printable without --show-secrets)
```

## 9. Deployment topology

Single AlmaLinux 9 VPS. `sing-box` supervised by systemd, listening on
`443/tcp` (VLESS+REALITY) and `443/udp` (Hysteria2). `services/subscription`
on loopback `127.0.0.1:9100`, reverse-proxied to public `8443/tcp` by a
TLS terminator (nginx/caddy — documented as an external, well-maintained
component; not built by us per spec §27). Existing native services keep
their current loopback bindings (`rendezvous` 9000-class, unchanged).

## 10. Testing strategy

- Rust unit tests in `crates/compat-config`: token generation/validation
  (incl. constant-time comparator), redaction (`Debug` never exposes
  secrets — same style as `crates/crypto` tests), URI rendering,
  sing-box JSON rendering against fixed expected output, disabled-user
  exclusion, atomic-update logic (temp file + rename, rollback on
  validation failure) using a fake "validator" so the test suite does
  not require the real `sing-box` binary.
- `services/subscription` tests: valid token → 200 with expected body
  shape; invalid/disabled token → generic 404; rate limiting.
- Where the real `sing-box` binary is available (CI/deployment host, not
  this sandbox), `deploy/almalinux/render-config.sh` calls
  `sing-box check` before every apply — documented as an integration
  step, not faked as a unit test.
- No test depends on real third-party infrastructure.

## 11. Rollback strategy

- Config: `config.json.bak` kept; `update.sh` restores it and restarts
  sing-box if the new config fails `sing-box check` or the service fails
  to come up healthy.
- Binaries: `update.sh` keeps the previous release's Rust binaries and
  sing-box binary alongside the new ones (versioned directories) and
  symlink-swaps; a failed health check reverts the symlink.
- REALITY keys are never rotated by `update.sh`/`install.sh` automatically
  (spec §12/§30 — high client impact); only `vpn-admin key rotate-reality
  --confirm` does that, explicitly.

## 12. Exact files expected to change

New:
```
crates/compat-config/**
apps/admin/**
services/subscription/**
deploy/almalinux/**
docs/COMPATIBILITY_VERSIONS.md
docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md
docs/CLIENT_COMPATIBILITY.md
docs/HIDDIFY_ANDROID.md
docs/ALMALINUX_DEPLOYMENT.md
docs/archive/COMPATIBILITY_SECURITY_REVIEW.md
```
Modified:
```
Cargo.toml                (add new workspace members)
README.md                 (document the two modes)
TASKS.md                  (append compatibility-phase tracking)
docs/ARCHITECTURE.md       (add compatibility stack to the diagram/text --
                            this doc was later removed with the native stack;
                            see docs/SUPPORTED_PRODUCT.md)
```
Not modified: anything under `crates/{transport-api,policy,
failure-classifier,transport-native,rendezvous-client,config}`,
`services/rendezvous`, `services/relay-agent`, `apps/client-daemon`.

## 13. Risks

- **sing-box syntax drift**: config schema pinned to 1.13.x
  (`docs/COMPATIBILITY_VERSIONS.md`); `sing-box check` is the actual
  guard against drift, not memorized syntax.
- **This sandbox cannot run a real sing-box binary or bind privileged
  ports / real network interfaces** (no outbound package install
  verified, no root network capability) — implementation is written and
  unit-tested against fakes; live-server validation (`sing-box check`,
  actual VLESS/Hysteria2 handshakes, Hiddify import) requires the real
  AlmaLinux target host and is documented as a manual acceptance step,
  not claimed as automated in this session.
- **ACME/TLS for the subscription host** is a real external dependency
  (a domain + DNS); not something this session can provision. Documented
  as an install-time prerequisite.
- **Scope**: the full spec (60+ numbered sections) describes a multi-week
  production rollout. This plan prioritizes per spec §62 (working
  connectivity → security → deployment → usability → failure
  independence → maintainability → elegance → features) and documents
  every deferred item honestly rather than stubbing it.

## 14. Explicit non-goals (this phase)

- No rewrite of REALITY/VLESS/Hysteria2/TLS/QUIC cryptography — sing-box
  is external, supervised, unmodified upstream.
- No Docker/Kubernetes/Postgres/Redis/service mesh — single VPS,
  systemd, flat files.
- No live purchase/provisioning of a domain, DNS, or ACME certificate
  from this session — documented prerequisite.
- No mobile client code — Hiddify/v2rayNG are used unmodified.
- No removal or weakening of the native stack.
