# singbox-client contract fixtures

Stable JSON examples of the versioned provisioning contract that
`singbox-vpn` serves to `singbox-client`
(<https://github.com/David610/singbox-client>).

**Every credential in this directory is fake.** The UUIDs, REALITY public
key, short IDs and passwords are deliberately obvious placeholders and
are not accepted by any deployment. They exist to be committed to a
public repository and consumed by another project's CI. A test in this
repository
(`crates/compat-config/tests/contract_fixtures.rs::no_fixture_contains_anything_that_could_be_a_real_secret`)
fails if anything here starts to look like real key material, a server
path, or a certificate-verification opt-out.

The schema itself is documented in
[`docs/PROVISIONING_CONTRACT.md`](../../docs/PROVISIONING_CONTRACT.md);
its authoritative definition is the Rust model in
`crates/provisioning-contract`.

## The files

| File | What it represents |
| --- | --- |
| `01-reality-only.json` | A deployment where Hysteria2 is **not** configured: `capabilities` lists only `vless-reality`, and there is no Hysteria2 endpoint. This is the "a transport is unavailable" case. |
| `02-hysteria2-only.json` | The mirror case: Hysteria2 configured, no REALITY endpoint. Note it advertises no experimental capabilities — `diag-tcp-only` needs a REALITY endpoint to be meaningful. |
| `03-both-transports.json` | The normal production document: both transports available. |
| `04-hysteria2-salamander-obfs.json` | Both transports, with Hysteria2 Salamander obfuscation configured (`obfs` present). Compare against `03` to see that `obfs` is absent, not null, when obfuscation is off. |
| `05-diagnostic-tcp-only.json` | The `diag-tcp-only` **experimental** profile (`?diagnostic=tcp-only`): the Hysteria2 endpoint and capability are gone entirely. |
| `06-diagnostic-vision-off.json` | The `diag-vision-off` **experimental** profile (`?diagnostic=vision-off`): the REALITY endpoint has **no `flow` field at all**. |
| `07-error-unsupported-schema-version.json` | The HTTP 400 body a client gets when it requests a schema version this server does not implement. |
| `08-invalid-missing-short-id.json` | A document that **must be rejected**: a `vless-reality` endpoint whose `reality.short_id` is empty. See "Expected errors" below. |

`server.version` is written as `0.0.0-fixture` in every file. The real
server sends its actual release version there; the placeholder keeps a
routine version bump from breaking either repository's CI. Treat the
field as "a non-empty version string", never as a value to match.

## What `schema_version` means

`schema_version` is an integer naming the *semantics* of the document,
not the server release. It is `1` today.

* **Adding** a value to `capabilities`, or an **optional** endpoint
  field, is a compatible change and does **not** bump `schema_version`.
  A client must therefore **skip capability values and endpoint
  `transport` values it does not recognise** rather than failing the
  whole document.
* **Removing or renaming** a field or capability value, or changing what
  an existing field means, **does** bump `schema_version`.
* A client that requires a version the server cannot serve must get an
  explicit error (see `07-...json`), never a silently different document.

## How to fetch the contract

```
GET https://<subscription-host>:<port>/v1/provision/<token>
GET https://<subscription-host>:<port>/v1/provision/<token>?schema_version=1
```

The path carries the version, so `/v1/provision/...` is the URL a client
stores. `?schema_version=N` is optional: send it to assert the version
you expect and get an explicit error instead of a surprise. The token is
the same bearer credential used by the legacy `/sub/<token>` routes.

Requesting a version the server does not implement — either
`/v2/provision/<token>` or `?schema_version=2` — returns **HTTP 400**
with the body in `07-error-unsupported-schema-version.json`. Match on
`error == "unsupported_schema_version"`; the `message` string is for
humans and may change.

## How `singbox-client` CI is expected to use these files

1. Vendor this directory (git submodule, sparse checkout, or a CI step
   that fetches it from `singbox-vpn@main`). Do not hand-copy: the point
   is that both repositories test the same bytes.
2. For each of `01`–`06`: parse with the client's real parser and assert
   the parse **succeeds**, then assert on structure —
   * `schema_version == 1`,
   * `server.product == "singbox-vpn"`,
   * the exact `capabilities` set,
   * one endpoint per capability, with the transport-specific fields
     present (`uuid` + `reality.{public_key,short_id,fingerprint}` for
     `vless-reality`; `password` for `hysteria2`).
3. For `01`: assert the client reports Hysteria2 **unavailable** and
   still produces a working REALITY-only configuration — a missing
   transport must never make the whole profile unusable.
4. For `04`: assert the Salamander obfuscation password is applied; for
   `03`: assert no obfuscation is applied.
5. For `05`/`06`: assert the client treats them as **experimental** —
   it must never select a `diag-*` capability on its own, and must not
   present these profiles as normal options. `06` in particular must not
   invent a `flow` value: an absent `flow` means "no flow".
6. For `07`: assert the client surfaces an explicit
   unsupported-version error to the user rather than retrying, falling
   back to a legacy format, or guessing a version.
7. For `08`: assert the client **rejects** the document. A client that
   accepts it would silently produce a REALITY configuration that cannot
   complete a handshake.
8. Compare on parsed structure, not raw bytes. Key order and whitespace
   are not part of the contract.

## Expected errors

| Fixture | Expected outcome |
| --- | --- |
| `07-error-unsupported-schema-version.json` | Not a document at all — an error body. `error` is the stable discriminator. |
| `08-invalid-missing-short-id.json` | Validation failure: transport `vless-reality` requires a non-empty `reality.short_id`. This repository's parser reports `MissingRealityParameter { endpoint_id: "reality-1", missing: "short_id" }`. A client need not match that wording, only reject the document and say which endpoint and field were at fault. |

## Regenerating

Fixtures `01`–`07` are generated by the real server code path, never by
hand:

```
UPDATE_CONTRACT_FIXTURES=1 cargo test -p compat-config --test contract_fixtures
```

Review the diff, and mirror any change in `singbox-client` before
releasing it. `08-invalid-missing-short-id.json` is hand-written on
purpose: it is an example of what the server must never produce.
