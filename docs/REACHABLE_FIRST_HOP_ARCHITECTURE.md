# Reachable first-hop architecture — Phase 2 design

**Status:** design only; no production relay has been implemented or verified.  
**Date:** 2026-09-10  
**Primary client:** [Tamara](https://github.com/David610/tamara)  
**Evidence boundary:** local/source feasibility only. No real second VPS, Russian ISP, destination allowlist, SNI filter, UDP restriction, or relay-provider outage has been tested.

## 1. Problem and threat model

Phase 1 solves a different problem: when the client can reach several foreign VPN endpoints, Tamara can test them and move between concrete outbounds without restarting Core. That does **not** solve a network that refuses packets to arbitrary foreign destination addresses before any VPN handshake can happen.

This phase considers a stricter network in which one or more of the following may hold:

- arbitrary foreign destination IPs are unreachable while a smaller set of destinations remains reachable;
- reachability can differ by mobile operator, fixed ISP, region, time, address family, or transport;
- the network may additionally inspect SNI/TLS metadata;
- UDP may be unavailable while TCP remains usable;
- an allowed address today may disappear tomorrow;
- an operator may deliberately isolate permitted services so unrelated traffic cannot piggyback on them.

The design does **not** assume that any Russian VPS, cloud provider, CDN, TURN server, domain name, SNI value, or shared address is allowlisted. Reachability is an empirical property that must later be tested on the target networks.

### Fundamental limit

If the restricted network permits **no reachable destination that is legitimately capable of relaying traffic onward**, no VPN protocol can manufacture a route that the network does not provide. REALITY, Hysteria2, TLS camouflage, a different SNI, or a hundred additional foreign VPSs only help after packets can reach their first hop.

The Phase-2 goal is therefore **multiple independent, authorized reachability paths**, not an "unblockable" protocol.

## 2. Existing architecture to preserve

The product keeps one VPN engine:

```text
Tamara
  -> provisioning envelope + EndpointCatalog
  -> EndpointSelector / resilience coordinator
  -> Hiddify Core
  -> sing-box
```

The server remains a small self-hosted product, not a fleet-management platform. Static/operator-declared infrastructure comes first.

Current v1 already gives us useful forward-compatible primitives:

- endpoint `path` is `direct` today, represented by `PathType::Direct | Other(String)`;
- endpoint `failure_domain`, `region`, `provider`, and `asn` are operator metadata;
- Tamara treats `path` and `transport` as opaque labels rather than reimplementing protocol parsing;
- the credential-bearing, Core-consumable `singbox_config` is rendered from the same endpoint set as the envelope.

## 3. Verified Core feasibility

A local build experiment against the pinned Hiddify Core/sing-box stack established the architectural prerequisite for this phase: a sing-box outbound can use another outbound as its `detour`, and Hiddify Core's config builder preserved that chain.

Conceptually:

```text
client/Core
   |
   +-- exit E1 (VLESS+REALITY)
           |
           +-- dial via relay R1 outbound
```

The relay is therefore a first-hop outbound and the foreign exit remains the inner/ultimate VPN exit. A route such as `E1 via R1` can remain a concrete selectable tag. Phase 2 does **not** require another VPN engine.

The feasibility experiment used a relay transport only to prove `detour` preservation. It is **not** evidence that that transport is suitable for a restricted real network.

## 4. Required properties of a viable first hop

A production first hop must be:

1. **actually reachable** on the target restricted network, demonstrated rather than assumed;
2. **authorized to relay** the traffic — operated by Tamara/the user or supplied under terms that permit this use;
3. **authenticated** and never an open proxy;
4. **replaceable**, because any single address/provider may become unreachable;
5. **bounded in blast radius**, using scoped/per-user credentials where practical;
6. **compatible with the existing Core**, preferably through native outbound chaining;
7. **privacy-minimal**, requiring no destination browsing history in a central control plane;
8. **operationally observable** at aggregate service-health level without per-user browsing telemetry;
9. **safe to revoke/rotate** without placing secrets in `EndpointCatalog`;
10. **explicit about capabilities**, especially TCP/UDP, so the client does not promise a path the relay cannot carry.

## 5. Candidate architectures

### A. Operator-controlled reachable ingress -> foreign exit

Tamara or the user operates an authenticated relay on infrastructure that later testing shows remains reachable. The foreign exit outbound dials through that first hop.

```text
restricted client -> R1 -> E1 -> Internet
```

This is the cleanest ownership and abuse boundary. Its weakness is fundamental: operating a domestic VPS does not make it reachable under an allowlist. The project must first prove that a legitimate ingress class can remain reachable.

### B. TURN-style relay -> foreign exit

A TURN-like service can provide a reachable relay when the network permits the TURN service and the service contract permits this traffic. It can be useful where WebRTC infrastructure is deliberately reachable, but TURN is not a magic bypass: the TURN IP itself must be reachable, authentication/rate limits matter, and arbitrary VPN throughput may conflict with provider limits or terms.

This is a candidate only with infrastructure we operate or contract for that purpose.

### C. Legitimate HTTPS / reverse-proxy ingress

An operator-controlled HTTPS-facing ingress can carry a deliberately supported relay protocol or tunnel to the foreign exit. It can blend operationally with normal HTTPS infrastructure without pretending to be an unrelated third party.

This option is attractive for TCP-only reachability and common port 443, but a generic reverse proxy cannot automatically carry arbitrary UDP semantics. The application protocol and provider must explicitly support the required forwarding.

### D. CDN/provider fronting or piggybacking

Using unrelated allowlisted third-party infrastructure, domain-fronting loopholes, or shared addresses that are not authorized for arbitrary relay traffic is a **NO-GO production dependency**. It is fragile, can violate provider terms, can expose other tenants, and may be specifically countered by dedicated-address isolation.

A contracted provider feature that explicitly permits the relay use can be evaluated under C or B. "It happens to work through someone else's allowed IP" is not an architecture.

### E. sing-box/Hiddify-native chaining

This is the implementation mechanism, not a reachability source by itself. `detour` lets an exit outbound use an authenticated first-hop outbound while Tamara still selects a single concrete route tag. It minimizes custom protocol code and keeps all transport semantics inside Core.

### F. Multiple independent ingress classes

The robust end-state is not one magic R1. It is two or more legitimately reachable first-hop failure domains when operationally justified, for example an operator-controlled HTTPS-like ingress and a separately operated relay class. Tamara can then treat `(exit, path)` combinations as separate route candidates.

This adds operational cost, so it is a second step after one relay path has been proven on real networks.

## 6. Comparison matrix

Scores are architectural expectations, **not real-network evidence**.

| Candidate | Reachability under strict destination allowlist | TCP | UDP | Ownership/control | Privacy boundary | Operational burden | Core fit | Sudden-break risk | Production position |
|---|---|---:|---:|---|---|---|---|---|---|
| A. Operator-controlled reachable ingress | Only if its destination is demonstrably permitted | strong | transport-dependent | **high** | clear/controllable | medium | **strong** via `detour` | medium/high | **Recommended MVP if reachability can be proven** |
| B. TURN-style contracted relay | Only if TURN destination is permitted | yes | often yes | medium/high if operated/contracted | clear if self-operated; provider metadata otherwise | medium/high | needs a Core-compatible client/outbound integration | high | Secondary candidate |
| C. Operator-controlled HTTPS/reverse-proxy ingress | Only if ingress destination is permitted | **strong** | limited unless explicitly supported | **high** | clear/controllable | medium | strong if represented by a supported outbound/forwarder | medium/high | Strong TCP-first candidate |
| D. Unauthorized CDN/fronting/piggyback | Uncertain and deliberately fragile | varies | poor/varies | **low** | opaque third party | deceptively high over time | custom/provider-specific | **very high** | **Reject** |
| E. Core-native `detour` chaining | Does not create reachability itself | depends on first hop | depends on first hop | n/a | preserves existing engine boundary | **low incremental** | **verified feasible locally** | inherits first hop | **Use as implementation mechanism** |
| F. Multiple independent ingress classes | Best chance if at least one class remains permitted | mixed | mixed | high when all are authorized | controllable | **high** | strong with route catalog | lower correlated risk | Post-MVP hardening |

## 7. Recommended MVP

### Recommendation

**One operator-controlled, authenticated first-hop ingress, represented as a Core-native outbound, with the existing foreign exit outbound dialed through it using sing-box `detour`; keep direct and relay paths side by side in Tamara.**

The ingress technology should be selected only after a real-network reachability trial. For the local lab, a disposable SOCKS/Shadowsocks-style relay is sufficient to prove mechanics; that does not choose the production transport.

Example candidate set:

```text
E1 / direct
E1 / relay-R1
E2 / direct
E2 / relay-R1
```

Once one authorized first-hop class is proven, add a second independently operated ingress failure domain only if the marginal reliability justifies the cost.

### Why this MVP

- preserves Tamara -> Hiddify Core -> sing-box;
- uses a Core capability already shown to preserve outbound chaining;
- keeps credentials and protocol parsing out of Dart;
- has a clear operator/security/abuse owner;
- does not depend on exploiting unrelated infrastructure;
- can be simulated locally before any real VPS exists;
- lets direct paths remain the normal low-latency choice when available.

### What it does not solve

It does not make an arbitrary relay destination allowlisted, hide the fact that the client contacts that relay, guarantee UDP, guarantee nationwide Russian reachability, or protect against a network that removes every usable authorized relay destination.

### Go/no-go prerequisite

Do **not** build a production relay service until a legitimate ingress location/provider has been tested from the intended networks and shown to be reachable often enough to justify productization.

## 8. Additive contract model

The existing endpoint `path` field is deliberately opaque. Keep that property. Do not put protocol configuration or credentials into the Dart catalog.

The smallest useful additive v1 extension is an optional top-level metadata list that describes path identities without secrets:

```json
{
  "schema_version": 1,
  "access_paths": [
    {
      "id": "relay-r1",
      "kind": "relay",
      "failure_domain": "relay:r1",
      "region": "example-region",
      "provider": "operator-declared",
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
    "outbounds": ["...credential-bearing relay and exit outbounds..."]
  }
}
```

`access_paths` is metadata only. It must contain no password, token, UUID, private key, TLS secret, or raw proxy URI. The actual authenticated relay outbound stays inside the protected/opaque `singbox_config` just like transport credentials do today.

A v1 client that ignores `access_paths` still sees endpoint tags and the opaque `path` string. A relay-aware Tamara can use the metadata for shared-fate reasoning. This can therefore remain an additive `schema_version: 1` extension provided validation confirms old consumers ignore the new field as intended.

### Proposed Rust types (design only)

```rust
pub struct AccessPath {
    pub id: String,
    pub kind: AccessPathKind,          // Direct/Relay/Other
    pub failure_domain: Option<String>,
    pub region: Option<String>,
    pub provider: Option<String>,
    pub capabilities: Vec<String>,     // e.g. tcp, udp
}
```

`Endpoint.path` remains the reference. Do not add `relay_password`, `relay_uri`, or equivalent fields.

## 9. Route candidates and failure attribution

Phase 1 mostly reasons about endpoint shared fate. A chained route introduces another independent dimension:

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

### Conservative rule

If Core only reports that `E1 via R1` failed, Tamara knows the **route candidate** failed. It does **not** know whether R1, E1, the link between them, DNS, TLS, or the network caused it. Do not mark every route sharing R1 or E1 failed merely from that observation.

A shared failure domain may be penalized only when there is evidence that actually identifies the shared component, or when all relevant route candidates independently fail and the policy explicitly treats that aggregate as an availability fact rather than a censorship diagnosis.

User-facing language remains "route unavailable", never "relay blocked by Russia" unless a future measurement can genuinely prove that cause.

## 10. Credential architecture

A relay must never be an unauthenticated public forwarder.

Required properties:

- per-user or narrowly scoped relay credentials where the chosen transport supports them;
- independent revocation and rotation;
- no one universal credential across all users;
- no server private key in a provisioning envelope;
- no relay secret in `EndpointCatalog`/`access_paths` metadata;
- no secret in logs, diagnostics, display labels, or `toString()`;
- a stolen client profile should compromise only that user's/scoped access, not relay administration;
- operator tooling may associate user -> relay credential, but should not require automatic root access to every relay.

The server-side peer credential pattern from Phase 1 is the precedent: operator-supplied remote credentials are associated per user without turning this deployment into a remote fleet controller.

## 11. Privacy and component visibility

The architecture cannot make every component blind, so the design states what each can technically observe.

| Component | Can observe | Should not centrally retain by default |
|---|---|---|
| Restricted access network | client source, first-hop destination IP/port, timing/volume; possibly SNI/TLS metadata | n/a — outside product control |
| First-hop relay | client source IP, foreign exit destination, timing/volume; relay authentication identity | browsing domains, DNS queries, user payload contents |
| Foreign VPN exit | tunnel account, egress destinations/DNS depending on configured resolver, timing/volume | long-term per-user browsing history |
| Provisioning/control plane | account, declared topology, credential lifecycle | destination domains/IPs, DNS history, packet contents |
| Tamara client | local route health and current route | persistent censorship labels or browsing telemetry |

The relay-to-exit design should keep the inner exit transport cryptographically protected end-to-end through the relay, so the first hop forwards an encrypted session rather than terminating the user's final VPN security boundary.

Infrastructure monitoring should prefer aggregate service health. No Phase-2 requirement justifies central logging of a user's browsing destinations.

## 12. Abuse controls

An Internet-reachable relay creates a real abuse surface even for a small trusted group. Minimum controls:

- authentication required before forwarding;
- deny open-recursion/open-proxy behavior;
- per-user revocation;
- connection/rate limits sized for the product rather than unlimited anonymous use;
- bounded logs focused on service security, not browsing history;
- provider terms explicitly compatible with relay/VPN traffic;
- documented incident procedure for credential theft and abuse complaints;
- the relay should forward only the intended route semantics where the selected transport permits that restriction.

## 13. Operational model

Phase-2 MVP remains operator-declared:

```text
deployment.toml
  local endpoints
  peer endpoints
  declared access paths / relay metadata

users.json
  per-user endpoint credentials
  per-user relay credentials/references
```

The provisioning service renders one coherent document/config for a user. It does **not** automatically provision remote VPSs, log into relays as root, discover providers/ASNs, or synchronize secrets across a fleet.

A later control plane can be considered only after static relay operations are proven too costly.

## 14. Local simulation before real infrastructure

The next implementation phase can prove mechanics entirely on one machine:

```text
Tamara/Core
   |
   | simulated policy: exit E1 not directly reachable
   v
relay R1  ------------>  exit E1  ------------> local HTTP sink
```

Required `SIMULATED_ALLOWLIST` cases:

1. direct E1 denied, R1 reachable -> `E1 via R1` succeeds;
2. R1 unavailable -> relay route fails without blaming E1;
3. R2 reachable -> `E1 via R2` succeeds;
4. E1 unavailable -> all routes to E1 fail, but routes to E2 remain candidates;
5. current relay recovers while another route is healthy -> no immediate flap;
6. manual relay-pathed route fails -> no silent switch when Manual mode is active;
7. stale result after network/catalog generation change -> no side effect;
8. relay credentials rotate -> old route health is invalidated without placing the secret in metadata.

The lab may use loopback relays and firewall/process-level refusal. It must be labeled `SIMULATED_ALLOWLIST`, never `RUSSIA_VERIFIED`.

## 15. Real-world acceptance matrix

No row below is currently PASS.

| Acceptance item | Current evidence | Required later evidence |
|---|---|---|
| second real VPS | **UNVERIFIED** | independent host deployment |
| second provider / ASN | **UNVERIFIED** | provider/ASN-confirmed deployment and failure test |
| REALITY remote failover | **UNVERIFIED** | real endpoints + client acceptance |
| Hysteria2 remote failover | **UNVERIFIED** | real UDP endpoint + client acceptance |
| Russian mobile network | **UNVERIFIED** | multiple operators/regions/devices |
| Russian fixed broadband | **UNVERIFIED** | multiple ISPs/regions |
| destination-IP allowlist behavior | **UNVERIFIED** | controlled test where direct exit is unreachable but authorized relay is reachable |
| SNI filtering | **UNVERIFIED** | target-network measurement |
| UDP restriction | **UNVERIFIED** | target-network measurement |
| relay reachability | **UNVERIFIED** | each candidate ingress class tested directly |
| relay blocking/recovery | **UNVERIFIED** | induced/observed failure and failover |
| long-duration stability | **UNVERIFIED** | multi-day soak with bounded retries and resource monitoring |

## 16. Cost and complexity

The cheapest credible MVP is one additional small relay host/service plus the existing foreign exit, because Core-native chaining avoids a second client engine. Cost is dominated by relay bandwidth and provider egress, not control-plane compute.

Do not publish a fixed monthly price before a provider/region is selected: bandwidth pricing differs by orders of magnitude and is the deciding variable. The design should therefore measure GB/user/month and relay egress before choosing a provider.

A second independent relay class increases reliability but also doubles credential rotation, monitoring, abuse handling, and provider dependencies. Add it after the first path has demonstrated real value.

## 17. Rollout stages

**Stage 0 — current:** Phase-1 direct multi-endpoint failover only.  
**Stage 1 — local mechanics:** additive path metadata + Core detour fixtures + `SIMULATED_ALLOWLIST` harness.  
**Stage 2 — one authorized real ingress:** manually operated relay, small trusted cohort, explicit evidence ledger.  
**Stage 3 — diversity:** second independent ingress failure domain if measurements justify it.  
**Stage 4 — automation:** only then consider operational tooling beyond static declaration.

No stage is promoted based only on unit tests or local simulation.

## 18. Explicit non-goals

- no claim of being unblockable, undetectable, Russia-proof, or whitelist-proof;
- no unauthorized domain fronting or piggybacking on unrelated allowed services;
- no new VPN engine in Tamara;
- no protocol proliferation merely to increase the feature count;
- no multi-node root-access fleet manager in the MVP;
- no central browsing/DNS telemetry;
- no automatic attribution of a failed route to DPI/censorship/relay/exit without evidence;
- no assumption that a domestic VPS is allowlisted.

## 19. Unknowns that require evidence

1. Which legitimately operated/contracted ingress classes remain reachable on the target networks?
2. Is TCP-only relay capability enough for the intended user experience, or is UDP carriage required at the first hop?
3. What sustained bandwidth and latency are acceptable on mobile?
4. Does the target network key primarily on destination IP, SNI, protocol fingerprint, or combinations?
5. How often do allowed destinations change, and how quickly must path metadata rotate?
6. Can a relay provider support the expected VPN traffic under its terms and abuse process?
7. Does a real chained route preserve acceptable DNS/leak behavior on Windows, Android and iOS?

## 20. Go / no-go criteria

### Build the Phase-2 MVP only if

- at least one authorized ingress class is demonstrably reachable on a meaningful sample of intended restricted networks;
- its provider permits the traffic;
- Core can carry the required transport through it without a second VPN engine;
- per-user/scoped auth and revocation are practical;
- latency/bandwidth are acceptable;
- privacy review finds no need for browsing-history telemetry.

### Stop or redesign if

- all legitimate relay destinations are filtered like the foreign exits;
- the only working path depends on unauthorized third-party piggybacking;
- acceptable throughput requires an open/unbounded relay;
- preserving reachability requires disabling TLS verification or exposing credentials;
- a second client VPN engine becomes necessary merely to express the path;
- operational cost/abuse burden exceeds the reliability value.

## Decision

**RECOMMENDED MVP:** operator-controlled authenticated reachable ingress + Core-native `detour` to the existing foreign exit, represented as an additional concrete `(exit, path)` route candidate. Keep direct routes and relay routes together; keep the relay's non-secret identity in additive provisioning metadata and all relay credentials inside protected Core configuration.

**WHAT CAN BE TESTED NOW:** config rendering, path metadata, selection semantics, credential isolation, and a loopback `SIMULATED_ALLOWLIST` harness.

**WHAT REQUIRES REAL INFRASTRUCTURE:** whether any first hop is actually reachable under the intended restrictions, cross-provider/ASN diversity, real bandwidth/latency, mobile behavior, and censorship durability.

This design is intentionally a reachability architecture, not a promise that a particular network will permit it.
