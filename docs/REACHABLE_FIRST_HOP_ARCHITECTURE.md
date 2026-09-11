# Reachable first-hop architecture — Phase 2

**Status:** local Stage 1 complete; production first-hop selection and real-network validation deliberately deferred.  
**Date:** 2026-09-11  
**Primary client:** [Tamara](https://github.com/David610/tamara)  
**Evidence boundary:** `VERIFIED_UNIT`, `VERIFIED_LOCAL_CORE`, and `SIMULATED_ALLOWLIST` only. No real second VPS, Russian ISP, destination allowlist, SNI filter, UDP restriction, or relay-provider outage is claimed here.

The detailed local evidence matrix is in [`PHASE2_LOCAL_ACCEPTANCE.md`](PHASE2_LOCAL_ACCEPTANCE.md).

## 1. Problem and fundamental limit

Phase 1 handles reachable endpoint failure: Tamara can probe several concrete outbounds and move between them without restarting Core. That cannot solve a stricter network which refuses packets to arbitrary foreign destination addresses before any VPN handshake begins.

Phase 2 therefore models an **authorized reachable first hop**:

```text
NORMAL
Tamara -> foreign exit

RESTRICTED / alternative path
Tamara -> authorized first hop -> foreign exit -> Internet
```

The design does not assume that a Russian VPS, cloud provider, CDN, TURN server, domain, SNI value, or shared address is allowlisted. Reachability is an empirical property for the later real-testing stage.

If the restricted network permits **no destination that is both reachable and legitimately capable of relaying onward**, no VPN protocol can manufacture a missing path. REALITY, Hysteria2, different SNI values, or more foreign VPSs help only after packets can reach their first hop.

The target is therefore **multiple independent, authorized reachability paths**, not an “unblockable” protocol.

## 2. Architecture boundary

The product keeps exactly one VPN/protocol engine:

```text
singbox-vpn provisioning document
        |
        v
Tamara EndpointCatalog + access-path metadata
        |
        v
EndpointSelector / ResilienceCoordinator
        |
        v
concrete Core outbound tag
        |
        v
Hiddify Core -> sing-box
```

Tamara does not parse or construct relay protocols in Dart. Credential-bearing transport and first-hop configuration stays inside the opaque `singbox_config`. The server remains a small self-hosted product, not a remote fleet-management plane.

## 3. Core-native chaining is locally verified

sing-box outbound `detour` is the chosen chaining mechanism. The important compatibility question was whether an authenticated first-hop outbound and an exit outbound using `detour` survive **Tamara's normal pinned Hiddify config-build path**, rather than only working in an artificial raw-config test.

That is now locally verified.

Tamara's dedicated Phase-2 acceptance workflow builds the checksum-pinned Hiddify Core, runs its normal builder, and verifies two independently authenticated loopback first hops:

```text
Direct:      Core -> E1                         denied by fixture
Path R1:     Core -> R1(auth) -> E1 -> sink    succeeds
Path R2:     Core -> R2(auth) -> E1 -> sink    succeeds
```

The builder preserves each `detour` relationship and the synthetic first-hop credentials. Hidden relay outbounds are not exposed as user-selectable exits. The runtime reaches the same local E1 socket through either R1 or R2 while a direct Core connection to E1 is refused.

This is `VERIFIED_LOCAL_CORE` + `SIMULATED_ALLOWLIST`. It proves configuration/runtime mechanics only. It does **not** prove that a particular real first-hop transport or address remains reachable on a restricted network.

## 4. Required properties of a production first hop

A viable production first hop must be:

1. actually reachable on the intended target networks, demonstrated rather than assumed;
2. authorized to relay the traffic — operated by the user/Tamara or supplied under compatible terms;
3. authenticated and never an open proxy;
4. replaceable, because one address/provider may become unreachable;
5. bounded in blast radius with scoped/per-user credentials where practical;
6. compatible with Hiddify Core/sing-box without a second client engine;
7. privacy-minimal, with no central browsing-history requirement;
8. observable at aggregate service-health level without destination telemetry;
9. safely revocable/rotatable without putting secrets in `EndpointCatalog`;
10. explicit about capabilities such as TCP/UDP.

The production first-hop technology/provider is intentionally **not selected by local tests**. That decision depends on later evidence about reachability, provider policy, throughput, latency, UDP needs, and abuse operations.

## 5. Candidate architectures

### A. Operator-controlled reachable ingress

```text
restricted client -> R1 -> E1 -> Internet
```

This provides the cleanest ownership, privacy and abuse boundary. A domestic or nearby VPS is not automatically reachable under an allowlist, so location alone is not evidence.

### B. Contracted/self-operated TURN-style relay

Potentially useful only when that TURN destination is reachable and the service explicitly permits the traffic. TURN does not create reachability by itself.

### C. Operator-controlled HTTPS/reverse-proxy ingress

A legitimate operator-controlled 443-facing ingress may be a strong TCP-first candidate if its application protocol explicitly supports the forwarding required. Generic HTTPS reverse proxying does not automatically provide arbitrary UDP semantics.

### D. Unauthorized CDN/fronting/piggybacking

**NO-GO as a production dependency.** Do not depend on unrelated allowlisted infrastructure, domain-fronting loopholes, or shared addresses that are not authorized for arbitrary relay traffic.

### E. Core-native `detour`

This is the implementation mechanism, not the reachability source. Its preservation through the pinned normal Hiddify builder is now `VERIFIED_LOCAL_CORE`.

### F. Multiple independent ingress classes

The robust later end-state can include more than one legitimate first-hop failure domain if real evidence justifies the operational cost.

## 6. Recommended MVP shape

The architectural MVP remains:

**one operator-controlled or explicitly contracted authenticated first hop, represented as a Core-native outbound; the existing foreign exit dials through it with `detour`; direct and relay-pathed concrete routes coexist.**

Example route set:

```text
E1 / direct
E1 / relay-R1
E1 / relay-R2
E2 / direct
E2 / relay-R1
```

The local lab uses authenticated SOCKS5 solely as a disposable mechanism to prove Core/build/selection semantics. It does **not** choose SOCKS5 as the production restricted-network transport.

## 7. Additive schema-v1 access-path model

The server implements optional non-secret `access_paths` metadata while preserving `schema_version: 1`:

```json
{
  "schema_version": 1,
  "access_paths": [
    {
      "id": "relay-r1",
      "kind": "relay",
      "failure_domain": "relay:r1",
      "region": "operator-declared-region",
      "provider": "operator-declared-provider",
      "capabilities": ["tcp"]
    }
  ],
  "endpoints": [
    {
      "id": "e1-direct",
      "tag": "Europe 1 — direct",
      "path": "direct"
    },
    {
      "id": "e1-r1",
      "tag": "Europe 1 — relay 1",
      "path": "relay-r1"
    }
  ],
  "singbox_config": {
    "outbounds": ["...credential-bearing first-hop and exit outbounds..."]
  }
}
```

`access_paths` contains metadata only. It must never contain a password, token, UUID, private key, TLS secret, raw proxy URI, or transport configuration. Deployment parsing rejects credential-shaped and unknown keys rather than silently accepting them.

The endpoint `path` field remains opaque. When a document opts into `access_paths`, a non-direct path must resolve to a declared id. Older schema-v1 documents without `access_paths` retain their previous opaque-path compatibility.

Tamara now mirrors this boundary: it can ingest/persist the non-secret path metadata while leaving the credential-bearing `singbox_config` opaque.

## 8. Route candidates and conservative failure attribution

A chained route has two dimensions of shared fate:

```text
RouteCandidate {
  exitEndpointId
  outboundTag
  accessPathId
  exitFailureDomain
  pathFailureDomain
}
```

Examples:

```text
E1 / direct       exit-domain=E1   path-domain=direct
E1 / relay-R1     exit-domain=E1   path-domain=R1
E1 / relay-R2     exit-domain=E1   path-domain=R2
E2 / relay-R1     exit-domain=E2   path-domain=R1
```

If Core reports only that `E1 via R1` failed, Tamara knows only that this **route candidate** failed. It does not know whether the cause was R1, E1, the R1->E1 link, DNS, TLS, the access network, or something else. A single chained-route failure therefore must not poison every route sharing R1 or E1.

Local selector/coordinator tests now cover this conservative rule, independent exits, R1->R2 eligibility, anti-failback, Manual semantics, stale generation rejection, and credential-rotation invalidation.

User-facing language remains “route unavailable” rather than inventing a censorship/DPI/relay diagnosis.

## 9. Credential architecture

A production relay must never be an unauthenticated public forwarder.

Required properties:

- per-user or narrowly scoped relay credentials where the selected transport supports them;
- independent revocation/rotation;
- no universal credential shared across every user;
- no server private key in the provisioning envelope;
- no relay secret in `EndpointCatalog` or `access_paths`;
- no secret in logs, diagnostics, labels, or `toString()`;
- a stolen client profile should compromise only that user's/scoped access, not relay administration;
- operator tooling should not require automatic root access across a relay fleet.

The local Core harness uses synthetic test credentials to prove the builder/runtime preserves authenticated detours. Production credential issuance is intentionally deferred until the real-testing stage selects the actual first-hop technology.

## 10. Privacy and component visibility

| Component | Can technically observe | Should not centrally retain by default |
|---|---|---|
| restricted access network | client source, first-hop IP/port, timing/volume; possibly TLS/SNI metadata | outside product control |
| first-hop relay | client source, foreign exit destination, timing/volume, relay auth identity | browsing domains, DNS history, packet payloads |
| foreign exit | tunnel account, egress destinations/DNS depending on resolver, timing/volume | long-term per-user browsing history |
| provisioning/control plane | account, declared topology, credential lifecycle | destination domains/IPs, DNS history, packet contents |
| Tamara | local route health/current route | persistent censorship labels or browsing telemetry |

The intended relay-to-exit design keeps the inner exit transport cryptographically protected through the first hop. Infrastructure monitoring should prefer aggregate service health.

## 11. Abuse and operational controls

For a real Internet-reachable first hop, require authentication, per-user revocation, bounded rate/connection limits, provider terms compatible with VPN/relay traffic, service-security logs rather than browsing logs, an incident procedure, and forwarding restrictions appropriate to the selected transport.

The server remains declarative/static first. It does not automatically SSH into remote relays, discover providers/ASNs, synchronize secrets across a fleet, or become a remote root controller.

## 12. Local Stage-1 acceptance — complete

The eight locally testable cases are now covered. The exact evidence mapping lives in [`PHASE2_LOCAL_ACCEPTANCE.md`](PHASE2_LOCAL_ACCEPTANCE.md):

1. direct E1 unavailable, R1 route succeeds;
2. R1-pathed route failure does not invent an E1/shared-component diagnosis;
3. independent R2 path to E1 succeeds;
4. exhausted E1 routes do not poison E2;
5. recovery does not immediately flap away from a healthy incumbent;
6. Manual relay-pathed route failure never silently falls back;
7. stale result after network/catalog generation change has no side effect;
8. credential-only provisioning replacement invalidates old route health through a non-secret generation while secrets stay out of metadata.

The local harness is deliberately loopback-only and modifies no firewall, host routes, DNS, adapter, TUN, or external infrastructure. It is labeled `SIMULATED_ALLOWLIST`, never `RUSSIA_VERIFIED`.

## 13. Real-world acceptance — later stage

No local result promotes any row below to PASS:

| Acceptance item | Current evidence | Required later evidence |
|---|---|---|
| production first-hop transport/provider | **UNSELECTED BY DESIGN** | target-network reachability + provider-policy + throughput/latency evidence |
| second real VPS | **UNVERIFIED** | independent host deployment |
| second provider / ASN | **UNVERIFIED** | provider/ASN-confirmed deployment and induced failure |
| REALITY through real first hop | **UNVERIFIED** | chosen ingress + real foreign exit + client acceptance |
| Hysteria2/UDP through real first hop | **UNVERIFIED** | selected ingress with required UDP semantics |
| Russian mobile network | **UNVERIFIED** | multiple operators/regions/devices |
| Russian fixed broadband | **UNVERIFIED** | multiple ISPs/regions |
| destination-IP allowlist behavior | **UNVERIFIED** | controlled target-network test |
| SNI/DPI filtering | **UNVERIFIED** | target-network measurement |
| UDP restriction | **UNVERIFIED** | target-network measurement |
| relay blocking/recovery | **UNVERIFIED** | real first-hop outage/recovery |
| DNS/leak behavior on physical devices | **UNVERIFIED** | Windows/Android/iOS acceptance |
| long-duration stability/performance | **UNVERIFIED** | multi-day soak and resource/latency/throughput evidence |

These are evidence-stage items, not unfinished local implementation work.

## 14. Rollout stages

**Stage 0 — complete locally:** direct multi-endpoint resilience/failover.  
**Stage 1 — complete locally:** additive non-secret access-path metadata, Tamara ingestion, route-policy semantics, authenticated normal-builder Core detours, and `SIMULATED_ALLOWLIST` acceptance.  
**Stage 2 — later real testing:** select and operate one authorized ingress only after target-network evidence justifies it.  
**Stage 3 — later diversity:** add a second independent real ingress failure domain if measurements justify the cost.  
**Stage 4 — later automation:** only after static real operations demonstrate a need for additional control-plane tooling.

No stage is promoted to real-network support based on unit tests or local simulation.

## 15. Explicit non-goals

- no claim of being unblockable, undetectable, Russia-proof, or whitelist-proof;
- no unauthorized domain fronting or piggybacking;
- no new VPN engine in Tamara;
- no protocol implementation in Dart;
- no fleet-wide root-access manager for the MVP;
- no central browsing/DNS telemetry;
- no automatic attribution of a failed route to DPI, censorship, relay, exit, country, or provider without evidence;
- no assumption that a domestic VPS is allowlisted.

## 16. Decision

**Architecture decision:** retain Core-native authenticated first-hop `detour` to the existing foreign exit, represented as a concrete `(exit, path)` route candidate, with direct and relay-pathed candidates coexisting. Keep all first-hop secrets inside protected Core configuration and only non-secret path identity/metadata in the additive provisioning catalog.

**Local plan status:** complete. The pinned normal Hiddify builder/runtime, Tamara metadata boundary, route policy, Manual behavior, stale-operation safety, rotation invalidation, and loopback `SIMULATED_ALLOWLIST` mechanics are covered.

**Next stage:** real evidence — choose the actual authorized first-hop technology/provider only after measuring what is genuinely reachable and operationally acceptable on the intended networks.
