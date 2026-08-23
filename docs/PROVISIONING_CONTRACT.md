# Provisioning contract (schema_version 1)

The versioned, first-party contract between this server (`singbox-vpn`)
and its primary client, **`singbox-client`**
(<https://github.com/David610/singbox-client>).

Authoritative definition: the Rust model in
`crates/provisioning-contract/src/lib.rs`. This document explains it;
where the two disagree, the code wins and this document is the bug.
Published examples live in
[`fixtures/singbox-client-contract/`](../fixtures/singbox-client-contract/README.md).

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
| **PRIMARY, first-party** | `singbox-client` | `GET /v1/provision/{token}` — the contract described here. |
| **FALLBACK, third-party** | Hiddify, v2rayNG, NekoBox, raw sing-box | The legacy `GET /sub/{token}` routes: `?format=hiddify`/`?format=uri` share links, or `?format=singbox` native sing-box JSON. |

Both tiers are served from the same endpoint model, so they can never
disagree about a user's credentials. What differs is representation, and
what each side can be *claimed* to do — see
`docs/CLIENT_COMPATIBILITY.md` and `docs/DEVICE_ACCEPTANCE_TESTS.md` for
what has actually been verified on a device, versus what is only a
documented assumption. Nothing in this document is verified network
behaviour; contract tests prove document shape and nothing more.

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
    }
  ]
}
```

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

* **Adding** a `capabilities` value, or an **optional** endpoint field,
  is a compatible change and does **not** bump `schema_version`. Clients
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
