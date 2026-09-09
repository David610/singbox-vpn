# Multi-endpoint resilience spine — design

**Date:** 2026-09-09
**Repos in scope:** `David610/singbox-vpn` (server), `David610/tamara` (client)
**Explicitly out of scope:** `David610/singbox-client` — not modified, not
extended, not used as a source of truth for client behaviour.

## What this pass delivers, and what it does not

This is the **spine**: the server can describe independently-hosted
endpoints with per-user credentials and failure-domain metadata; Tamara
can ingest, cache and represent them; and a pure selector can decide
which endpoint should be used.

**The app does not fail over at the end of this pass.** Wiring the
selector to `ProxyRepository.selectProxy` — along with the Automatic/Manual
UI and the Windows failure-injection harness — is the next phase. Nothing
in this document should be read as claiming otherwise.

### Evidence boundary

There is no second VPS, no alternate provider, no censored network, and
only a Windows development machine. Therefore:

* **IMPLEMENTED ≠ REAL-VPS-VERIFIED.** Peer endpoints are implemented and
  tested against fixtures and loopback addresses. No peer endpoint in this
  pass has been dialled on a real second server.
* **LOCAL FAILOVER LOGIC VERIFIED ≠ CENSORSHIP RESISTANCE VERIFIED.** The
  selector's decisions are proven by unit tests over synthetic
  observations. That says nothing about DPI, TSPU, Russia, Iran, China,
  mobile carriers, or real endpoint blocking.

---

## Part 1 — Server: additive `schema_version` 1 extension

### Terminology

These changes keep `schema_version = 1`. There is no "v1.1" — no such
value exists on the wire, and inventing one in prose would imply a
negotiable version that clients could ask for. The correct description is
an **additive `schema_version` 1 extension**.

`docs/PROVISIONING_CONTRACT.md`'s versioning rule currently says adding a
`capabilities` value or an **optional endpoint field** is compatible. That
rule is extended explicitly to also cover **optional top-level fields**,
because this change adds one (`singbox_config`) and the existing wording
does not cover it. Clients already must skip unrecognised values.

### 1.1 New optional endpoint metadata

`provisioning_contract::Endpoint` gains, all optional and all
`skip_serializing_if = "Option::is_none"` so an unconfigured deployment's
document is byte-identical to what it emits today:

| Field | Type | Meaning |
|---|---|---|
| `failure_domain` | `Option<String>` | Operator-declared shared-fate identifier |
| `region` | `Option<String>` | Opaque operator label |
| `provider` | `Option<String>` | Opaque operator label |
| `asn` | `Option<String>` | Opaque operator label |
| `path` | `Option<PathType>` | `direct`, or `Other(String)` for forward compatibility |

The three label fields are **opaque to the server**. Nothing on the server
reads them for any decision; they exist to be displayed and to inform
client-side diversity heuristics later. No network or ASN discovery is
performed — the server does not look anything up.

`path` models the prompt's direct/relay distinction. Only `direct` is
produced. Relay is reserved and **not implemented**; `Other(String)`
exists so a future value does not break a v1 parser.

### 1.2 Derived failure domains, and their limit

When `failure_domain` is absent, the **client** derives one from the
normalised endpoint host. This makes today's single-VPS deployment behave
correctly with no operator action: REALITY and Hysteria2 on one
`public_host` land in one domain automatically, which is exactly the
shared-fate relationship that actually holds.

An operator-supplied `failure_domain` always overrides the derived value.

**Documented limitation:** two different DNS names that resolve to the
same machine cannot be known to share a failure domain from syntax alone.
Syntactic derivation will treat them as independent. Correcting that
requires the operator to declare `failure_domain` explicitly. The client
will not resolve names or query ASNs to find out — that would be network
discovery, which is out of scope and would leak query patterns.

### 1.3 Embedded `singbox_config`

The document gains an optional top-level `singbox_config: Option<Value>`
carrying exactly what `render_singbox_client_subscription` already emits.

**Why embed rather than have the client fetch separately.** Tamara
deliberately does not reimplement transport parsing in Dart; the pinned
Core/ray2sing is authoritative for protocol syntax. So Tamara needs the
Core-consumable config as an opaque blob. Fetching it separately from the
catalog would allow the two to observe different server states — a
credential rotation between the two requests yields a catalog that
misdescribes the running config. One request, rendered from one endpoint
model, cannot drift.

This is the single-source-of-truth rule (§5G of the brief): **one endpoint
model → one rendered contract + one embedded Core config.** There is never
a second independently generated config.

### 1.4 Cross-validation between catalog and embedded config

Validation enforces, and negative tests deliberately construct violations
of, each of:

| Invariant | Check |
|---|---|
| A | `singbox_config` is rendered from exactly the document's endpoint set |
| B | Every catalog endpoint's `tag` has a matching Core outbound `tag` |
| C | The `select` group's options are exactly the endpoint tags plus `auto` |
| D | `route.final` names the selector group (`select`) |

These are what make the atomicity guarantee real rather than asserted: the
server structurally cannot serve a catalog that misdescribes the config
shipped beside it.

### 1.5 Forbidden-content audit — a discovered constraint

`FORBIDDEN_CONTENT` is a **case-insensitive substring scan over the whole
serialized document**, with needles including `insecure`, `"dns"`,
`"tun"`, `"inbounds"`, `"ipv4"`, `"ipv6"`, `auto_route`, `private_key`.

Embedding the rendered config breaks this, and not hypothetically: the
Hysteria2 outbound emits `"insecure": false` — a security-**positive**
assertion that certificate verification is on. A substring scan for
`insecure` cannot tell that from `"insecure": true`, so it would reject a
correct document.

**Resolution.** The whole document remains audited (§5E of the brief), but
with the appropriate instrument for each part:

* **Envelope** — every field except `singbox_config` — keeps the existing
  substring audit, unchanged. This is right for a region that should
  contain no configuration at all.
* **`singbox_config`** — a new **structural** audit that walks the JSON:
  * top-level keys are **allowlisted** to exactly `outbounds` and `route`;
    anything else (`dns`, `inbounds`, `tun`, `log`, `experimental`) is
    rejected. Positive allowlisting is strictly stronger than blocklisting
    substrings.
  * within `route`, only `final` and `rules` are permitted.
  * any object key matching `private_key` / `privatekey` / `private-key`
    (case-insensitive) is rejected wherever it appears.
  * a key named `insecure` is rejected unless its value is exactly
    `false`.
  * any string value containing `-----BEGIN`, `.pem`, `/etc/`, `/var/`,
    `/opt/` is rejected.

This is a change of **mechanism, not of guarantee** — the structural audit
catches strictly more than the substring scan could, because it
understands position and value rather than mere presence of a word.

`singbox_config` contains live per-user credentials and is **never
logged**, at any level.

### 1.6 Runtime declarative peer endpoints

ADR-0009 specified this shape and deferred implementation. It is
implemented here, at its smallest safe scope.

This is **not** multi-node orchestration, fleet management, remote
control, credential synchronisation, or health-checking another VPS. It is
one thing: an operator who independently runs a second server can tell
this server *"for this user, also advertise that already-existing
endpoint."*

New optional `deployment.toml` section, following the existing section
conventions:

```toml
[[peer_endpoints]]
id = "eu2-reality"
tag = "Europe 2"
host = "vpn2.example.net"
port = 8443
transport = "vless-reality"
server_name = "www.example-decoy-two.org"
reality_public_key = "..."
reality_short_id = "..."
reality_fingerprint = "chrome"     # optional, defaults to chrome
failure_domain = "eu2"
region = "nl"                       # optional
provider = "provider-b"             # optional
asn = "operator-label"              # optional
path = "direct"                     # optional, defaults to direct
```

Validation at load:

* `id` must not collide with a locally generated id (`reality-1`,
  `hysteria2-1`) nor with another peer id.
* the REALITY **public** key and short id are shape-validated with the
  existing `credentials::validate_reality_public_key_shape`.
* **any key resembling a private key is rejected outright.** The peer's
  REALITY private key has no use on this server and must never be typed
  into its config. This is enforced, not merely documented.
* `host` must be dialable, `port` non-zero.

`DEPLOYMENT_SCHEMA_VERSION` stays `1`: the section is optional and absent
from every existing file, and `#[serde(default)]` reads its absence as an
empty list. An existing `deployment.toml` loads byte-identically.

### 1.7 Per-user peer credentials (ADR-0009 Option A)

Shared per-peer credentials are rejected: they break per-user revocation
(disabling a local user would not revoke their access to the peer) and
widen blast radius (one device compromise exposes a credential working for
every user).

`CompatUser` gains:

```rust
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub peer_credentials: BTreeMap<String, PeerCredential>,
```

keyed by peer endpoint id. This mirrors the established
`vision_off_experiment` pattern exactly, so `users.json` for a deployment
with no peer credentials stays **byte-identical** to what it is today, and
`USERS_SCHEMA_VERSION` stays `1`.

`PeerCredential` is a transport-shaped enum holding a `SecretString` — so
it inherits the existing guarantee that `Debug`/`Display` print
`SecretString(REDACTED)` and cannot leak through a stray `{:?}`.

Two rules with real consequences:

* **Credentials are never auto-generated for a peer.** This server does
  not control the peer and cannot mint credentials that would authenticate
  there. The operator pastes in a credential that was independently
  created on the peer server by its own `vpn-admin`. Generating one would
  produce a confidently-wrong document.
* **A peer endpoint a user has no valid credential for does not appear in
  that user's document at all.** Not present-but-broken — absent. A client
  must never be handed an endpoint it cannot authenticate to.

### 1.8 Peer credential administration

New `vpn-admin user peer` subcommands, in the existing style:

| Command | Behaviour |
|---|---|
| `user peer set <user> <endpoint-id> --uuid/--password` | Add or replace |
| `user peer rotate <user> <endpoint-id> --uuid/--password` | Replace an existing one; fails if absent |
| `user peer remove <user> <endpoint-id>` | Remove |
| `user peer list <user>` | List **endpoint ids only** |

`user peer list` and every status/list surface print **endpoint ids and
presence, never the credential value**. Reading a credential back out is
not an operation this CLI offers.

Credentials are supplied by the operator; the transport determines which
flag is valid (`--uuid` for `vless-reality`, `--password` for
`hysteria2`), and a mismatch is rejected rather than silently coerced.

### 1.9 Single-VPS behaviour is unchanged

With zero `peer_endpoints` configured, the generated provisioning document
and every `/sub` output must be **equivalent to current behaviour**.
Regression tests assert this directly, including that a document with no
peers serializes without any of the new keys present.

No operator is forced into multi-VPS mode. The feature is inert until
configured.

### 1.10 Documentation retargeting

`docs/PROVISIONING_CONTRACT.md`'s client-tier table is updated so **Tamara**
is the current first-party/primary advanced client. The existing
`singbox-client` analysis is **retained as a dated historical note**,
because it explains why the fallback `/sub` routes exist and why they must
keep working. It is not deleted.

**No fixture directory rename in this pass.** `fixtures/singbox-client-contract/`
keeps its name. Renaming would touch every fixture consumer for a purely
aesthetic gain; the historical name is instead explained in the directory
README and in the contract doc. New fixtures are added in place. A rename
would need a test-architecture justification, and there is none.

---

## Part 2 — Tamara: endpoint catalog and cache

### 2.1 The catalog model contains no credentials

```dart
class CatalogEndpoint {
  final String id;              // stable contract id
  final String tag;             // display name AND Core outbound tag
  final String host;
  final int port;
  final String transport;       // opaque label, never interpreted
  final String failureDomainId;
  final String? region, provider, asn;
  final EndpointPath path;
}
```

There is deliberately **no field able to hold a UUID, password, obfs
password, or REALITY key material.** Those exist only inside the opaque
`singbox_config` blob, which passes through Dart as a string on its way to
`configs/{id}.json`.

Credential isolation is therefore **structural**, not a matter of
care: there is no field to leak, so no future edit to a `toString`,
log line, or diagnostic bundle can leak one. `toString` is credential-safe
by construction because there is nothing unsafe in scope.

Tests serialize and debug-print catalog objects and assert that known
fixture credentials cannot appear.

`tag` is the join key to `OutboundGroupItem.tag`. Part 1's cross-validation
(invariant B) guarantees the two agree.

### 2.2 Parsing — envelope only

`ProvisioningDocumentParser` recognises a provisioning document by a
top-level integer `schema_version` together with an `endpoints` list.
It parses **the envelope only**:

* transport is kept as an **opaque string**; Dart never interprets it.
* unknown `transport` or `path` values round-trip rather than failing,
  honouring the contract's forward-compatibility rule.
* `failure_domain` absent → derived as `host:<normalised host>`.
* the embedded `singbox_config` is extracted as an **opaque blob** and
  never inspected.

Typed failures: `malformed`, `unsupportedSchemaVersion(int)`,
`missingConfig`.

**Dart never parses VLESS/REALITY/Hysteria2 parameters.** The provisioning
envelope is also **never handed to Core as if it were sing-box config** —
only the extracted `singbox_config` is. Tests prove both directions of
this distinction.

### 2.3 The import path is wired, not merely available

The parser is not a utility that nothing calls. It is wired into the real
remote profile fetch/update flow:

```
remote fetch
  → recognise provisioning envelope   (else: existing path, untouched)
  → validate envelope
  → extract opaque singbox_config
  → write it through the existing profile-repository config path
  → store catalog metadata alongside the profile row
  → Core consumes the generated config exactly as it already does
```

Anything that is not a recognised provisioning document — Hiddify
subscriptions, Clash YAML, direct proxy URIs, arbitrary sing-box JSON,
base64 lists — continues through its **current path, unchanged**. No
resilience metadata is manufactured for them. That boundary is asserted by
test.

### 2.4 Crash consistency across filesystem and database

**Correction to the approved design.** The earlier draft said the config
file and catalog column are "replaced in one transaction." That is not
achievable: the config is a filesystem file and the catalog is a SQLite
column, and no transaction spans both. Documenting it as atomic would have
been a false claim.

Tamara already solves this. `ProfileOperationJournal` has phases
`prepared → fileInstalled → dbCommitted → rollingBack`, keeps the
superseded config as `.previous`, and writes the operation id into a
SQLite commit-marker table **in the same transaction as the profile-row
mutation**, so startup recovery can tell whether the DB side committed.
`_commitCandidate(...)` in `ProfileRepository` is the single funnel.

**The catalog therefore does not get its own recovery system.** It is
carried on the `ProfileEntity` written by `commitDb(operationId)`, so it
lands inside the *existing* transaction, and the *existing* journal already
guarantees the file/DB pairing. No second mechanism is invented.

Required postcondition after crash and restart — the recovered stable
state is either:

* OLD CONFIG + OLD CATALOG, or
* NEW CONFIG + NEW CATALOG

and never a mixed pair as a stable resting state.

Failure injection covers each stage: downloaded, parsed/validated, temp
config written, DB/catalog update begun, config rename, operation
committed, journal cleanup.

### 2.5 Cache semantics

| Event | Result |
|---|---|
| valid update | config + catalog replaced together via the journal |
| fetch failure | both preserved |
| malformed response | neither written; typed failure; cache preserved |
| unsupported `schema_version` | neither written; typed incompatibility; cache preserved |
| catalog decrypt failure | `EncryptedTextDecryptionException` — typed failure, ciphertext never reaches a parser |
| no cache + fetch fails | typed "no cached configuration" failure |
| profile deleted | catalog dies with the row; in-memory health for that profile dropped |

A valid working profile is never erased because a refresh failed.

The catalog is stored in a new DB column using the existing
`EncryptedTextCodec` convention already applied to `populatedHeaders` and
`userOverride`. The catalog holds no secrets, but keeping the column policy
uniform costs nothing and avoids a "which columns are encrypted?" ambiguity.

The DPAPI invariant is unchanged and must stay green: a `dpapi1:`-marked
value that fails to decrypt raises a typed failure and is **never**
returned to a semantic parser as plaintext.

---

## Part 3 — The selector

A pure Dart state machine: no Riverpod, no I/O, no Core reference, no
clock of its own (time is injected). Fully unit-testable.

### 3.1 State and bounded memory

States: `unknown`, `probing`, `healthy`, `degraded`, `failed`, `cooldown`.

Per-endpoint record, **all scalars**:

```
consecutiveSuccesses   (capped)
consecutiveFailures    (capped)
lastSuccessAt, lastFailureAt
lastFailureCategory
cooldownUntil
lastEstablishMs
```

"No unlimited history" holds structurally — there is no list to grow, and
no telemetry database. Health state is **in memory only** for this phase
and starts at `unknown` on every cold start.

### 3.2 Observable-only failure taxonomy

Core exposes per-outbound `urlTestDelay` (a number, or failure — **with no
reason**), nine coarse whole-Core `CoreAlert` values, `currentOutbound`,
and traffic counters. It does not report why a probe failed.

So the taxonomy is exactly what is observable:

`endpointProbeFailure`, `coreStartFailure`, `tunFailure`,
`noTrafficAfterConnect`, `unknown`.

`unknown` is first-class and expected, not a defect.

There is no `CENSORSHIP_CONFIRMED` and no `DPI_DETECTED` — **not as a
policy but as a fact about the enum**: those variants do not exist, so
they cannot be emitted. `restrictedNetworkSuspected` exists only as a
derived advisory requiring several independent domains failing at once,
and is never assigned from a single signal.

No failure cause is invented that Core does not expose. `urlTest` failing
is not evidence about DNS, TCP, TLS, REALITY, UDP, or DPI, and is never
reported as such.

### 3.3 Two-level reasoning, with domain health derived

Domain health is **derived on demand, never stored**. A domain counts as
unavailable only when *every* endpoint in it is `failed` or `cooldown`.

This makes the brief's requirement structural rather than a rule to
remember: a transport-specific failure on one endpoint cannot poison its
domain, because a healthy sibling transport keeps the domain available. A
domain becomes unavailable only when the observable states justify it.

Candidate ordering:

1. exclude endpoints in cooldown (unless all are)
2. prefer endpoints in available domains
3. within those, `healthy` > `unknown` > `degraded`
4. tie-break on `lastEstablishMs`

### 3.4 Asymmetric hysteresis

A **healthy incumbent is never displaced by a faster challenger.** It is
displaced only by becoming `degraded` or `failed`. A previously failed
endpoint recovering does **not** trigger failback while the incumbent is
healthy.

Both anti-flap requirements follow from this one rule rather than from
separate special cases.

Cooldown is exponential with a **hard cap**, and attempts per selection
round are bounded. Unbounded backoff and infinite retry are excluded by
construction, not by a guard that could be forgotten.

### 3.5 Designed for the next phase's wiring

The selector emits a decision naming the **exact Core outbound tag**:

```
EndpointSelector
   → ProxyRepository.selectProxy(selectorGroupTag, endpointTag)
   → Hiddify Core → sing-box selector
```

Failover is **not** designed around restarting Core per endpoint change.
Core already supports changing the selector's outbound without restarting
the tunnel, and that advantage is preserved deliberately.

The selector accepts a `generation` on every observation and rejects
superseded ones, mirroring `ConnectionLifecycleCoordinator`'s existing
contract so the next phase's wiring is mechanical. A network change bumps
the generation, so a stale probe result structurally cannot override a
newer selection.

The selector never calls Core itself.

---

## Testing

| Level | Covers |
|---|---|
| **UNIT** | selector state machine; catalog parser; credential-absence; cache semantics |
| **CONTRACT** | Rust: new field validation, peer assembly, cross-validation, negative mismatch tests, forbidden-content structural audit. Dart: parses the same fixture the Rust side emits |
| **CRASH-CONSISTENCY** | failure injection at each of the seven journal stages |
| **REGRESSION** | zero-peer output equivalence; ordinary subscriptions unchanged; `users.json`/`deployment.toml` load unchanged |
| **REAL_NETWORK_UNVERIFIED** | everything about real VPS, providers, ASN diversity, censored networks. Stated, never claimed |

Deferred to the next phase: `LOCAL_CORE_INTEROP`, `LOCAL_LOOPBACK_TRAFFIC`,
`SIMULATED_FAILURE`, and the PowerShell harness.

No production addresses are hard-coded in any test; fixtures and loopback
only.
