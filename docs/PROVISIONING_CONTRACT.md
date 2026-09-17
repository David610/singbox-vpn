# Provisioning contract (schema_version 1)

The versioned, first-party contract between this server (`singbox-vpn`)
and its primary client, **Tamara**
(<https://github.com/David610/tamara>).

Authoritative definition: the Rust model in
`crates/provisioning-contract/src/lib.rs`. This document explains it;
where the two disagree, the code wins and this document is the bug.
Published examples live in
[`fixtures/singbox-client-contract/`](../fixtures/singbox-client-contract/README.md)
— a directory name kept from when `singbox-client` was the intended
primary client. It is deliberately **not** renamed: the name is
historical, but renaming it would touch every fixture consumer for no
runtime or test-architecture benefit.

## Why this exists

Before this contract, the client/server relationship was implicit: the
server emitted a sing-box configuration or a list of share links, and
each consumer inferred the rest. Nothing named a version, nothing said
which transports a deployment actually had, and each output format
shaped credentials in its own code. This contract makes all three
explicit and gives them one owner.

## Client tiers

| Tier | Client | What it consumes |
|---|---|---|
| **PRIMARY, first-party** | **Tamara** | `GET /v1/provision/{token}` — the contract described here, including the embedded `singbox_config`. |
| **FALLBACK, third-party** | Hiddify, v2rayNG, NekoBox, raw sing-box | The legacy `GET /sub/{token}` routes: `?format=hiddify`/`?format=uri` share links, or `?format=singbox` native sing-box JSON. |

Tamara consumes this contract by parsing the **envelope only** — endpoint
ids, tags, hosts, ports, failure domains and operator labels — and
handing the embedded `singbox_config` to its pinned Hiddify Core
verbatim. It deliberately does not reimplement VLESS/REALITY/Hysteria2
parsing in Dart; the pinned Core/ray2sing remains authoritative for
protocol syntax. That constraint is why the config is embedded rather
than described field by field.

Both tiers are served from the same endpoint model, so they can never
disagree about a user's credentials. What differs is representation, and
what each side can be *claimed* to do — see
`docs/CLIENT_COMPATIBILITY.md` and `docs/DEVICE_ACCEPTANCE_TESTS.md` for
what has actually been verified on a device, versus what is only a
documented assumption. Nothing in this document is verified network
behaviour; contract tests prove document shape and nothing more.

### Historical note: `singbox-client` (superseded)

`singbox-client` (<https://github.com/David610/singbox-client>) was this
contract's originally-intended primary client and is **no longer part of
this product**. It is not modified, extended, or treated as a source of
truth for client behaviour. The note is kept rather than deleted because
it explains why the FALLBACK routes exist and must keep working:

> Verified against `singbox-client` main as of `d04a8b4`: that client
> never parsed `/v1/provision/{token}`. Its subscription importer fetched
> a URL and parsed it as a raw sing-box config (`{"outbounds": [...]}`),
> referencing none of `schema_version`, `capabilities`, or `endpoints`.
> The URL that worked with it was `/sub/{token}?format=singbox`.

Those legacy routes remain fully supported for third-party clients (see
**Backward compatibility** below). Nothing about them changed when
Tamara became the primary client.

## The API surface

```
GET /v1/provision/{token}                      # the contract, schema v1
GET /v1/provision/{token}?schema_version=1     # same, version asserted
GET /v1/provision/{token}?diagnostic=tcp-only  # experimental, opt-in only
GET /v1/provision/{token}?diagnostic=vision-off
```

The schema version is in the **path**, so `/v1/provision/{token}` is the
URL a client stores and a future `schema_version = 2` becomes
`/v2/provision/{token}` without renegotiating anything about v1.
`?schema_version=N` is optional and exists so a client can assert the
version it expects rather than discover a mismatch later.

`{token}` is the same bearer credential as the legacy `/sub/{token}`
routes: same rate limiting, same `no-store` caching headers, same
generic 404 for an unknown, disabled, or expired token (no user
enumeration).

## Document shape

```json
{
  "schema_version": 1,
  "server": {
    "product": "singbox-vpn",
    "version": "0.1.2",
    "minimum_client_version": "0.1.0"
  },
  "capabilities": ["vless-reality", "hysteria2"],
  "experimental_capabilities": ["diag-tcp-only"],
  "endpoints": [
    {
      "id": "reality-1",
      "tag": "Reality",
      "host": "vpn.example.com",
      "port": 443,
      "server_name": "www.example-decoy.com",
      "transport": "vless-reality",
      "uuid": "00000000-0000-4000-8000-000000000001",
      "flow": "xtls-rprx-vision",
      "reality": {
        "public_key": "FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEfake",
        "short_id": "0a1b2c3d",
        "fingerprint": "chrome"
      }
    },
    {
      "id": "hysteria2-1",
      "tag": "Hysteria2",
      "host": "vpn.example.com",
      "port": 443,
      "server_name": "vpn.example.com",
      "transport": "hysteria2",
      "password": "fake-hysteria2-password-not-a-real-secret",
      "obfs": { "type": "salamander", "password": "fake-salamander-obfs-password" }
    },
    {
      "id": "eu2-reality",
      "tag": "Europe 2",
      "host": "vpn2.example.net",
      "port": 8443,
      "server_name": "www.example-decoy-two.org",
      "transport": "vless-reality",
      "uuid": "00000000-0000-4000-8000-000000000002",
      "flow": "xtls-rprx-vision",
      "reality": { "public_key": "BAKE...bake", "short_id": "9f8e7d6c", "fingerprint": "chrome" },
      "failure_domain": "eu2",
      "region": "nl",
      "provider": "provider-b",
      "path": "direct"
    }
  ],
  "singbox_config": {
    "outbounds": [
      { "type": "vless", "tag": "Reality", "server": "vpn.example.com", "...": "..." },
      { "type": "hysteria2", "tag": "Hysteria2", "server": "vpn.example.com", "...": "..." },
      { "type": "vless", "tag": "Europe 2", "server": "vpn2.example.net", "...": "..." },
      { "type": "urltest", "tag": "auto", "outbounds": ["Reality", "Hysteria2", "Europe 2"] },
      { "type": "selector", "tag": "select",
        "outbounds": ["Reality", "Hysteria2", "Europe 2", "auto"], "default": "Reality" },
      { "type": "direct", "tag": "direct" }
    ],
    "route": { "final": "select" }
  }
}
```

The third endpoint above is an operator-declared **peer** on a second,
independently-run server: different host, different key, different
per-user credential, and its own `failure_domain`. The first two carry no
`failure_domain` because they share this deployment's `public_host` and
the client derives it. The complete, un-elided form of this document is
pinned as
[`fixtures/singbox-client-contract/10-peer-endpoint-with-embedded-config.json`](../fixtures/singbox-client-contract/10-peer-endpoint-with-embedded-config.json).

* `capabilities` — the transports this deployment can serve **right
  now**, derived from real configuration. A transport that is not
  configured appears in neither `capabilities` nor `endpoints`; that
  absence is how a client learns "REALITY yes, Hysteria2 no". The
  advertised set and the endpoint set are cross-checked in both
  directions during validation, so a capability can never be advertised
  without an endpoint, nor an endpoint served without its capability.
* `experimental_capabilities` — diagnostic profiles available to this
  user. A separate field, always `diag-`-prefixed, never part of
  production negotiation, never a default. A client must not select one
  on its own.
* `endpoints[].transport` discriminates the transport-specific fields.
  `flow` is present only when a flow is requested; the `diag-vision-off`
  profile omits it entirely rather than sending an empty value. `obfs`
  is present only when Salamander obfuscation is configured.
* `endpoints[].host`/`port`/credentials are **per-endpoint**, not
  deployment-global — nothing in the schema requires every endpoint to
  share a host. Beyond its own two listeners (`standard_endpoints`, one
  VPS — see `docs/SUPPORTED_PRODUCT.md`), this server now also emits
  endpoints an operator declared via `[[peer_endpoints]]`, each with its
  own host, key and per-user credential. See **Peer endpoints** below,
  `docs/ADR/0009-declarative-peer-endpoints.md`, and fixtures `09` and
  `10`. Peer support is implemented and tested against fixtures and
  loopback only — no real second VPS exists to verify it against.
* `access_paths` — optional non-secret first-hop metadata. Omitted when empty; when present, non-direct `endpoints[].path` values must resolve to an id in this list. It contains no relay credentials or proxy configuration.
* `singbox_config` — the Core-consumable config for exactly the endpoint
  set above, rendered from the same model in the same request. See **The
  additive `schema_version` 1 extension**.

## What the contract never contains

Enforced by the type system (no field can hold these) and re-checked by
an audit of the serialized document during validation, so a future field
cannot reintroduce one silently:

* the server's REALITY **private** key, or any TLS private key
* any server filesystem path, or PEM-encoded material
* `insecure` / any certificate-verification opt-out
* client-owned policy: DNS, MTU, TUN, `auto_route`/`strict_route`,
  kill switch, IPv4/IPv6 family preference, mobile lifecycle

The last group is a boundary, not an oversight. The server has no way to
observe or enforce any of it, so expressing an opinion about it would be
a claim it cannot keep — see `docs/CLIENT_PROTOCOL_BEHAVIOR.md`.

## The additive `schema_version` 1 extension

`schema_version` remains **1**. There is no "v1.1": no such value exists
on the wire, and naming one in prose would imply a version a client could
negotiate. Everything below is optional, absent by default, and skipped
during serialization when unset — a deployment that configures none of it
emits a document byte-identical to what it emitted before these fields
existed.

### Optional endpoint metadata

| Field | Meaning |
|---|---|
| `failure_domain` | Operator-declared shared-fate identifier. Endpoints with the same value are expected to fail together. |
| `region`, `provider`, `asn` | Opaque operator labels. **The server never reads these for any decision** and performs no network or ASN lookup to populate them — they are exactly what the operator typed. |
| `path` | `direct` by default. When top-level `access_paths` is present, a non-direct value references an `access_paths[].id`. The metadata/reference mechanism is implemented; actual relay routing is not. |

**When `failure_domain` is absent the client derives one from the
normalised host.** That is correct for this deployment's own endpoints:
REALITY and Hysteria2 share one `public_host`, and sharing a host is
exactly the shared-fate relationship that actually holds. The server does
not derive it, because for a peer endpoint that would mean guessing about
infrastructure it does not run.

**Documented limitation:** two different DNS names resolving to the same
machine cannot be known to share a failure domain from syntax alone, and
will be treated as independent. Correcting that requires the operator to
declare `failure_domain` explicitly. Neither side resolves names or
queries ASNs to find out.

### Optional access-path metadata

`access_paths` is an optional top-level list of **non-secret first-hop metadata**.
It is omitted when empty, so existing direct-only deployments keep the same
schema-version-1 wire shape. Each entry contains only:

- `id` — stable path identity referenced by a non-direct `endpoints[].path`;
- `kind` — `direct`, `relay`, or a forward-compatible unknown value;
- optional `failure_domain`, `region`, and `provider` operator labels;
- non-empty `capabilities` such as `tcp`/`udp`.

The server also accepts the same metadata declaratively through optional
`[[access_paths]]` blocks in `deployment.toml`. These declarations do not deploy,
contact, authenticate to, or health-check a relay. Unknown keys are refused, and
credential-shaped keys (`password`, `token`, `secret`, `credential`, `key`,
`private`) fail closed. Relay credentials and protocol configuration belong only
inside protected/opaque Core configuration, never in `access_paths`.

When `access_paths` is non-empty, every non-direct endpoint path must resolve to
an entry in that same atomic document. With no `access_paths` list, the original
v1 forward-compatibility rule remains: an opaque future `path` value can still
round-trip without being reinterpreted.

**Implementation status:** this metadata and validation layer is implemented and
tested. A production relay outbound, Core `detour` chain, relay credential
lifecycle, and real-network reachability are not implemented or verified by this
contract change.

### The embedded `singbox_config`

An optional top-level field carrying exactly what
`render_singbox_client_subscription` emits, rendered from the **same**
endpoint list published in `endpoints`, in the same request.

It exists because Tamara does not parse transports in Dart and needs the
Core-consumable config as an opaque blob. Embedding it rather than
letting the client fetch it separately is what makes the pair atomic: two
requests can observe two different server states — a credential rotation
between them — and yield a catalog that misdescribes the running config.
One endpoint model produces one contract and one config, so they cannot
drift.

`validate` cross-checks the pair. These are the invariants a client
relies on when it trusts a single fetch:

* every endpoint's `tag` has a matching outbound `tag` in the config;
* the `select` group's options are exactly the endpoint tags plus `auto`;
* `route.final` names the `select` group, so selecting an endpoint
  actually changes what is routed.

`singbox_config` carries live per-user credentials and is **never
logged**, at any level.

### How the forbidden-content audit is applied

The audit still covers the **whole** document, but with a different
instrument for each half, because one instrument cannot judge both.

The envelope keeps the existing case-insensitive substring scan. That is
the right tool for a region that should contain no configuration at all.

The embedded config gets a **structural** audit instead. The reason is
concrete rather than theoretical: the rendered Hysteria2 outbound
contains `"insecure": false` — a security-**positive** assertion that
certificate verification is on — and a substring scan for `insecure`
cannot tell that from the opt-out it exists to forbid. The structural
audit:

* **allowlists** top-level keys to exactly `outbounds` and `route`, so a
  client-owned policy block nobody thought to forbid by name (`dns`,
  `inbounds`, `tun`, `log`) is rejected because it was never permitted;
* permits only `final` and `rules` inside `route`;
* rejects any key matching `private_key`/`privatekey`/`private-key` at
  any depth;
* rejects `insecure` unless its value is exactly `false`;
* rejects `-----BEGIN`, `.pem`, `/etc/`, `/var/`, `/opt/` in any string
  value.

This is a change of mechanism, not of guarantee. Positive allowlisting
plus a value-aware walk catches strictly more than the substring scan
could, because it understands position and value rather than the mere
presence of a word.

## Peer endpoints

`[[peer_endpoints]]` in `deployment.toml` lets an operator who
independently runs a second server declare it here, so users who have a
credential for it receive it in their document. See
`docs/ADR/0009-declarative-peer-endpoints.md`.

This is **not** multi-node orchestration, fleet management, remote
control, credential synchronisation, or health-checking another VPS.
This server never contacts a peer. It repeats a declaration.

```toml
[[peer_endpoints]]
id = "eu2-reality"          # must not collide with reality-1/hysteria2-1
tag = "Europe 2"            # display name AND the Core outbound tag
host = "vpn2.example.net"
port = 8443
transport = "vless_reality"
server_name = "www.example-decoy-two.org"
reality_public_key = "..."  # the peer's PUBLIC key only
reality_short_id = "..."
failure_domain = "eu2"      # required for a peer
region = "nl"               # optional, opaque
provider = "provider-b"     # optional, opaque
```

Rules with consequences:

* **The peer's REALITY private key is refused, not ignored.** It has no
  use here and never leaves the peer server. Any unknown key in the block
  is refused too: a silently-dropped `reality_public_ky` typo would
  produce an endpoint nothing can dial, and a silently-dropped private
  key would leave the operator believing this server needed one.
* **Credentials are per-user and per-endpoint** (ADR-0009 Option A), held
  in `users.json` under `peer_credentials`. A shared per-peer credential
  was rejected: it breaks per-user revocation — disabling a local user
  would not revoke their peer access — and one device compromise would
  expose a credential valid for every other user.
* **This server never generates a peer credential.** It does not
  administer the peer, so a value it invented could not authenticate
  there. The operator pastes in what the peer's own `vpn-admin` issued,
  via `vpn-admin user peer set <user> <endpoint-id> --uuid|--password`.
* **A peer a user has no credential for is absent from their document**,
  not present-and-broken — a client cannot distinguish a broken endpoint
  from a network failure.
* A credential is never coerced across transports; a `--password` given
  for a `vless-reality` peer is refused rather than reshaped.

`vpn-admin user peer list` prints endpoint ids and transports only.
Reading a credential value back out is not an operation the CLI offers.

**With zero peers configured nothing changes**: `users.json` and the
served document stay byte-identical, `endpoints_fingerprint` does not
move, and no schema version advances. No operator is pushed into
multi-VPS mode.

### What is implemented versus what is verified

Peer endpoints are **implemented and tested against fixtures and loopback
addresses only**. No peer endpoint in this repository has been dialled on
a real second server, because no second VPS exists.

**IMPLEMENTED is not REAL-VPS-VERIFIED.** Nothing here is evidence about
behaviour on a real alternate provider, a different ASN, or a censored
network. See `docs/DEVICE_ACCEPTANCE_TESTS.md` and
`docs/RUSSIA_PRODUCTION_INVESTIGATION.md`, neither of which is upgraded
by anything in this change.

## Validation

Every document is validated before it is serialized
(`ProvisioningDocument::to_json` validates first), so an invalid one is
never served. A validation failure is answered with HTTP 500 and logged:
it always means a server-side defect or a broken deployment state, never
something a client can cause. Rules include:

* `schema_version` must be one this server implements
* a `vless-reality` endpoint requires a non-empty REALITY public key,
  short id (even-length hex, ≤16 chars) and fingerprint
* a VLESS client id must be an 8-4-4-4-12 hex UUID
* a `hysteria2` endpoint requires a non-empty password; an `obfs` block
  requires type `salamander` and a non-empty password
* `flow`, when present, must be `xtls-rprx-vision`
* endpoint ids are unique; host is dialable; port is non-zero
* capabilities and endpoints agree in both directions
* experimental capabilities are `diag-`-prefixed and absent from
  `capabilities`

## Versioning rules

* **Adding** a `capabilities` value, an **optional** endpoint field, or
  an **optional top-level field**, is a compatible change and does
  **not** bump `schema_version`. (The top-level case was made explicit
  when `singbox_config` and the endpoint metadata fields were added; the
  rule previously covered only endpoint fields and said nothing about
  what an additive document-level field meant.) Clients
  must skip capability and `transport` values they do not recognise
  rather than rejecting the document — `Capability::Other` /
  `Transport::Other` exist for exactly this.
* **Removing or renaming** a field or capability value, or changing what
  an existing field means, **does** bump `schema_version`.
* A version this server does not implement is an explicit failure, never
  a guess.

## Unsupported version handling

Requesting a version this server does not implement — `/v2/provision/…`
or `?schema_version=2` — returns **HTTP 400**:

```json
{
  "error": "unsupported_schema_version",
  "requested": 2,
  "supported": [1],
  "message": "this server implements provisioning schema_version [1]; …"
}
```

Match on `error`; `message` is for humans and may change. A non-integer
`schema_version` returns `error: "invalid_schema_version"`; an unknown
`diagnostic` returns `error: "unknown_diagnostic"`. In every case the
server refuses rather than serving something adjacent to what was asked
for.

## Backward compatibility

Nothing about the legacy surface changed:

* `GET /sub/{token}` with no `format` still serves native sing-box JSON.
* `?format=hiddify` and `?format=uri` still serve the same
  newline-separated `vless://` / `hysteria2://` share links.
* `?format=singbox`, `?profile=`, and `?compat=` behave exactly as
  before.

An old client that never learns about `/v1/provision` keeps working
indefinitely. What did change is that these outputs are now *rendered
from* the contract model rather than shaping credentials themselves, and
that the removed `?format=xray` is now a plain 400.

## Removed: the Xray-labelled toggle

`?format=xray` (and `vpn-admin`'s `subscription_url_xray`) is **removed**.

It rendered the same UUID, REALITY public key, short id, SNI, host,
port, fingerprint and flow as `?format=uri` and differed in exactly one
way: the VLESS line's label carried a `(Xray)` suffix. A share-link
label is a display string in the client's own UI — it is never sent over
the wire and no client selects an engine from it. Its own doc comment
recorded the syntax question as **UNVERIFIED**.

This repository's own research answers that question against the
feature: `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.5a records that
Hiddify iOS defaults to its sing-box-based core and that "Xray-core is
opt-in only, via an explicit `core=xray`/`xvless://` selection
(hiddify-app v2.0.4+ release notes) **that this project's generated
links never set**". A `(Xray)` label therefore could not have selected
an Xray engine, and any A/B result attributed to it would have compared
a profile against itself.

Keeping a switch that appears to change client behaviour while provably
changing nothing is worse than not having it: it makes an experiment
look controlled when it is not. If a future need arises to exercise a
specific client engine, the import syntax that actually does so must be
verified against real client source or release notes first, and the
capability must be modelled in the contract rather than smuggled into a
label.

## Kept: the diagnostic profiles

`diag-tcp-only` and `diag-vision-off` are kept because, unlike the Xray
label, each provably changes the emitted profile:

* **`diag-tcp-only`** removes every UDP-carrying option from the profile
  — the Hysteria2 endpoint and capability are gone, and on the legacy
  sing-box path the VLESS outbound additionally carries
  `"network": "tcp"`. TCP-only is enforced by construction: there is no
  UDP outbound left to fall back to. See
  `docs/COMPATIBILITY_QUIC_EXPERIMENT.md`.
* **`diag-vision-off`** omits the XTLS Vision flow. It requires a
  matching per-user server-side opt-in
  (`vpn-admin user vision-off-experiment <id>`) because sing-box's VLESS
  server rejects a flow that does not equal the configured per-user
  flow, and it is more fingerprintable to DPI than production. See
  `docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md` §9.5.

Both are diagnostics, never defaults, and both are advertised only in
`experimental_capabilities`.

## Adding a transport later

The transport enum and capability list are deliberately open: an
unrecognised value round-trips as `Other(String)` rather than failing a
parse, so adding one is a compatible change. **No new protocol is being
added.** Production transports are exactly VLESS+REALITY and
Hysteria2 (+ Salamander where configured) — see
`docs/SUPPORTED_PRODUCT.md`.

## What the contract tests do and do not prove

The suites in `crates/provisioning-contract/src/lib.rs`,
`crates/compat-config/src/contract.rs`,
`crates/compat-config/tests/contract_fixtures.rs` and
`services/subscription/src/lib.rs` prove the **shape and content of a
generated document**: fields present and absent, validation rules,
JSON round-trips, capability derivation, error responses.

They prove nothing about real-world network behaviour. In particular
they say nothing about whether any transport works from any specific
network, Russian networks included — see
`docs/RUSSIA_PRODUCTION_INVESTIGATION.md`, whose findings remain
UNVERIFIED and are not upgraded by anything here. Real-device and
real-network status lives in `docs/DEVICE_ACCEPTANCE_TESTS.md`.
