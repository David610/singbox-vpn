# Platform v2 — target architecture

**Status:** architecture freeze for branch `feat/platform-v2`, 2026-09-15.
Everything here is **target** unless [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md) marks it as implemented, and it carries an evidence label there. Current facts are in [CURRENT_STATE.md](CURRENT_STATE.md).

## 1. The question the platform answers

> Which `{route, transport, configuration}` is most likely to give this user a working, fast and stable tunnel **on this network right now**, and what do we do when it stops working?

The product is not a protocol. It is the combination of:

1. several independent, upstream-maintained transports;
2. several independent nodes (providers, ASNs, regions);
3. a deterministic, explainable route engine running **on the client**;
4. bounded, anti-flapping automatic recovery;
5. operator tooling that can add, drain and replace nodes safely;
6. measurements that prove (or disprove) each claim.

## 2. Components and trust boundaries

```text
                      ┌───────────────────────────── control plane (optional) ─┐
                      │  route directory · entitlements · node registry        │
                      │  provider drivers · replacement orchestrator · audit    │
                      └───────▲──────────────────────────────▲─────────────────┘
          config/route metadata│ (HTTPS, signed)              │ node status (signed, aggregate)
                               │                              │
┌──────────────── Tamara ──────┴────┐                ┌────────┴──────── node N ───────────┐
│ route engine (score/failover)     │                │ node agent (health, register)       │
│ engines: sing-box core │ AWG      │── VPN data ───►│ data planes: sing-box │ amneziawg-go │──► Internet
│ TUN · DNS · kill switch · IPv6    │                │ vpn-admin · subscription (self-host)│
└───────────────────────────────────┘                └─────────────────────────────────────┘
```

**Hard rule: VPN traffic never touches the control plane.** The control plane sees route metadata, entitlements, node health and (optionally, opt-in) aggregate route-health telemetry. It never sees destinations, DNS or payloads, and it holds no key that can decrypt user tunnels. See §9.

### 2.1 Deployment modes

| Mode | Control plane | Route directory source | Who provisions nodes |
|---|---|---|---|
| **Self-hosted, 1 node** (today) | none: the node's own `subscription` service | `/v1` or `/v2/provision/{token}` on the node | operator runs `install.sh` |
| **Self-hosted, N nodes** | none required: one node is the *provisioning authority* holding the fleet catalog | `/v2/provision/{token}` on the authority node | operator (`install.sh`), or optionally `vpn-fleet` with a provider driver |
| **Managed** | a separate Tamara control-plane service | the signed route directory (`tamara-next` managed contract) plus per-route authorization | the control plane through provider drivers |

Managed mode is never required. The self-hosted path must keep working with zero outbound dependencies beyond the node itself.

## 3. Transport abstraction

### 3.1 Why

Today transport knowledge is scattered across six files. Adding AmneziaWG to that pattern would multiply the scatter, and AWG cannot live inside sing-box at all: upstream sing-box, including the 1.14.0 used by Tamara, has no AmneziaWG implementation. The platform therefore treats a transport as a **replaceable data-plane module** with its own engine.

### 3.2 Model (`crates/platform-core::transport`)

```text
TransportKind      = VlessReality | Hysteria2 | AmneziaWg | Other(String)
DataPlaneEngine    = SingBox | AmneziaWg
TransportCapabilities {
    engine, l4 (Tcp|Udp), carries_udp_payload, detour_capable (can be a relay exit),
    per_user_credentials, supports_rotation, supports_live_revocation,
    fallback_client_formats (share link / sing-box JSON / awg-quick INI),
    requires_kernel_module (false for AWG userspace), active_probe_resistance (declared, not measured)
}
trait TransportProvider {
    kind(), capabilities()
    validate(&NodeTransportConfig) -> Result
    issue_credentials(user) -> Credential          // generate server-side where the transport needs it
    revoke_credentials(user) / rotate_credentials(user)
    render_server_fragment(active users) -> ServerArtifact   // sing-box inbound JSON or awg setconf text
    render_client_endpoint(user) -> v2 endpoint params         // public parameters + this user's credential
    health_probes() -> [ProbeSpec]                              // which L1–L6 probes apply
    install_plan() -> InstallPlan                               // pinned artifacts, units, ports, sysctls
}
```

- Protocol-specific code lives **only** in the provider implementation: `compat-config` providers for REALITY and Hysteria2, and `compat-config::amneziawg` for AWG. Orchestration, route catalog, scoring and failover see only `TransportKind` and `TransportCapabilities`.
- `install_plan()` is declarative data. The shell installer consumes it (pinned version, checksum or commit, systemd unit, firewall port), so installation stays in the existing transactional shell pipeline instead of moving into Rust.
- A future transport (e.g. a TCP/443 HTTP/2 or HTTP/3 carrier) is one new provider plus one new `TransportKind` value. The route engine needs no change.

### 3.3 Engines per transport

| Transport | Server engine | Client engine (Tamara target) | Fallback clients |
|---|---|---|---|
| VLESS+REALITY/TCP | sing-box (pinned upstream) | sing-box core | Hiddify, v2rayNG, raw sing-box |
| Hysteria2/UDP (+Salamander) | sing-box | sing-box core | Hiddify, raw sing-box |
| AmneziaWG 3.x/UDP | `amneziawg-go` userspace (default) or the `amneziawg` kernel module (optional) + `awg` tools | `amneziawg-go` (Windows/Linux/Android), `amneziawg-apple` (iOS/macOS) | AmneziaVPN / AmneziaWG apps via the `awg-quick` INI |

A client runs **one engine at a time** owning the TUN device. A same-engine switch (REALITY ↔ Hysteria2) can reuse Core outbounds. A cross-engine switch (AWG ↔ sing-box) is a full tunnel rebuild and is measured as such (§6.4).

## 4. Route model (`crates/platform-core::route`, contract `/v2`)

```text
Node      { node_id, provider, region, country, asn, failure_domain, role(exit|relay),
            lifecycle(provisioning|active|draining|retired), cost_hint }
Endpoint  { endpoint_id, node_id, transport, host, port, public_params, capabilities }
Route     { route_id, hops:[endpoint_id] (1 = direct, 2 = relayed), kind(direct|relayed),
            operator_priority, label }
Credential{ user × endpoint → transport credential }   // never in metadata
```

Rules (validated fail closed in `RouteCatalog::validate`):

- Unique `node_id`, `endpoint_id` and `route_id`. Every hop references an existing endpoint, and every endpoint an existing node.
- A relayed route has exactly two hops. The first hop's transport must be `detour_capable` into the exit's transport (today: REALITY → REALITY only). Hops sit on different nodes.
- A route's failure domains are the union of its hops' node failure domains. Routes never inherit health across failure domains except through explicit evidence (§6.3).
- **Credentials are per (user, endpoint).** Two endpoints on different nodes never share a credential, and the validator rejects identical credential material across nodes. The only sharing allowed is the existing `credential_ref` alias for "same exit, different path", because the exit really is the same.
- Nodes in `draining` stay servable to existing users but get a scoring penalty and are never selected for new sessions. `retired` nodes are never served.
- Metadata (provider, region, country, asn) is operator-declared or supplied by a provider driver. The server never infers it from IP lookups.

### 4.1 Contract versioning

- `/v1/provision/{token}` stays **byte-identical**. It never contains AWG, nodes or routes, so old Tamara builds and fallback clients keep working.
- `/v2/provision/{token}` carries `schema_version: 2`: nodes, endpoints, routes, per-user credentials inline per endpoint, `singbox_config` for the Core-renderable subset, and **no** client policy (DNS/TUN/MTU). A v1-only client fetching `/v2` fails explicitly (Tamara checks `schema_version != 1`). A v2 server asked for `?schema_version=3` answers HTTP 400 `unsupported_schema_version`, `supported: [1, 2]`.
- AWG fallback export: `/sub/{token}?format=amneziawg` returns one `awg-quick` INI for the user's AWG endpoint (HTTP 404 `transport_not_enabled` when there is none).

## 5. AmneziaWG integration (server)

- **Pinned upstream sources:**
  - `amneziawg-go` `v3.1.20260828` (`b5928ef…`), built with `go mod verify` in an isolated module cache using a pinned Go toolchain.
  - `amneziawg-tools` `v3.1.20260812` (`ee0f0a9…`), built from source.
  - Pins live in `deploy/lib/versions.env`.
- **Parameters** follow the upstream v3.1 README and parser (`device/uapi.go`, `src/config.c`):
  - `Jc`/`Jmin`/`Jmax` junk packets: client-side, `Jmin ≤ Jmax`, kept below the path MTU.
  - `S1`–`S4` paddings (u16): must match on both ends.
  - `H1`–`H4` header ranges (`a` or `a-b`, u32): must match, must not overlap each other, and are generated clear of the standard WireGuard type values 1–4.
  - `I1`–`I5` custom signature packets (`<b 0x..>`, `<r n>`, `<rd n>`, `<rc n>`, `<t>`): client-side.
  - `HeaderProtectionKey` (AWG 3+, 32-byte key): must match, and requires every `S1`–`S4 ≥ 12`.
  - `ContentPaddingAddition` (AWG 3+): a range, specified on both sides.
- **Keys:** X25519 through the existing `curve25519-dalek` dependency. The server keypair is node-local, and the private key never leaves the node. Per-user client keypair and preshared key:
  - *Self-hosted:* generated server-side so an INI can be served by bearer URL; this is the same trust class as today's VLESS UUID.
  - *Managed (target):* client-generated, registering only the public key. See THREAT_MODEL.
- **Addressing:** the node-level tunnel subnet is `10.66.0.0/16` plus optional IPv6 ULA `fd66::/64` (configurable). Per-user /32 and /128 allocations are deterministic, persisted and never reused while any user holds them.
- **Runtime:** `vpn-amneziawg.service` runs `amneziawg-go -f awg0` (or uses the kernel module) → `awg setconf awg0 <rendered>` → address and MTU. The firewall and masquerade are owned by `deploy/lib/amneziawg.sh`; forwarding sysctls are reversible, like `perf-tuning.sh`.
- **Revocation** removes the peer from the rendered config and applies it with `awg syncconf` semantics, validated first, like the sing-box apply.
- **Relay:** AWG is **not** detour-capable (UDP), so it is never a relay hop and is refused in `[[access_paths]]`, like Hysteria2 today.
- **Evidence separation:**
  - CONFIG-VERIFIED: rendered text parsed and accepted by the real `awg setconf`.
  - LIVE-DATAPLANE-VERIFIED (LOCAL): a real handshake plus transfer between two `amneziawg-go` instances in network namespaces.
  - VPS-VERIFIED and NETWORK-VERIFIED need real hosts and networks.

## 6. Route engine (client-side, reference implementation in Rust)

The engine runs **locally** on the client, and decisions use local observations. The reference implementation is `crates/platform-core` (pure, no I/O) with golden-vector fixtures (`fixtures/platform-v2/route-engine/*.json`). Tamara ports it to Dart or binds to it and must pass the same vectors, following the existing cross-repository fixture pattern. The probe agent (§10) uses the Rust engine directly.

### 6.1 Health layers

| Layer | Meaning | Example probe (no user traffic) |
|---|---|---|
| L1 reachability | TCP connect / UDP send path to the endpoint | TCP SYN→ACK to the endpoint port; for UDP, only inferable at L2 |
| L2 protocol handshake | Transport-authenticated handshake | REALITY/TLS handshake; Hysteria2 QUIC auth; AWG handshake completed (`latest-handshake` advanced) |
| L3 tunnel established | Engine reports tunnel up, local TUN/route installed | engine state + TUN present |
| L4 Internet via tunnel | A fixed controlled URL is reachable through the tunnel | `GET https://<node>/generate_204` via tunnel, or a generic 204 URL |
| L5 sustained transfer | Bounded download completes above a floor | N MiB from a controlled endpoint within a deadline |
| L6 stability | Survives over a time window | ≥ K successful L4 checks over W minutes with no stall |

"Socket accepted" means L1 only. "Connected" in UI terms requires L4. A route is **healthy** only with L4 inside the freshness window, and **proven** with L5 plus L6.

### 6.2 Scoring (deterministic, explainable)

`score = Σ component_i` with every component bounded, integer-valued and recorded as a reason:

- `+ operator priority` (bounded)
- `+ recency-weighted L4 success` / `− recency-weighted failures` (by failure class)
- `− handshake time` (piecewise, measured at L2)
- `− RTT`, `− jitter`, `− loss` (buckets)
- `+ sustained transfer health` (L5 pass) / `− stall`
- `+ stability streak` (L6)
- `+ per-network history` (keyed by a *local*, non-exported network fingerprint)
- `+ protocol preference` for the network class (e.g. UDP penalised after a `UDP_UNREACHABLE` classification on this network)
- `− draining node`, `− server load` (if advertised), `− battery/resource cost` (AWG < REALITY < Hysteria2 on a CPU basis, **declared defaults pending measurement**)

The result is a `RouteDecision { chosen, ranked:[(route, score, reasons[])] }`. Reasons are machine-readable enums, and a separate formatter produces operator text (`+ recent successful tunnel`, `− moderate RTT`). No machine learning.

### 6.3 Failure classification

`FailureClass`: `LOCAL_TUN_FAILURE, DNS_FAILURE, TCP_UNREACHABLE, UDP_UNREACHABLE, HANDSHAKE_FAILURE, AUTH_FAILURE, POST_HANDSHAKE_STALL, PMTU_OR_MTU_FAILURE, SERVER_OVERLOADED, ENDPOINT_DOWN, NETWORK_TRANSITION, ROUTE_LEAK_DETECTED, IPV6_LEAK_DETECTED, CENSORSHIP_SUSPECTED, UNKNOWN`.

The classifier takes *observations* (which layers passed or failed, error kinds, and control observations) and returns a class plus the evidence used. Ambiguous input → `UNKNOWN`. `CENSORSHIP_SUSPECTED` requires **all** of:

1. the same endpoint was reachable from an independent vantage point or path within the window (control success);
2. the local network's generic Internet control succeeded (the network is up);
3. a transport-specific failure pattern repeated ≥ N times, e.g. handshake reset or stall after ClientHello while a control TLS handshake to a non-VPN host on the same port succeeds.

Anything less is classified by the mechanical symptom (e.g. `HANDSHAKE_FAILURE`), never as censorship.

Shared-fate penalties: a failure attributes to the **route**. It spreads to other routes sharing a node or failure domain only if the class identifies that component (`ENDPOINT_DOWN` from multiple transports on the same node, `SERVER_OVERLOADED`). This implements the conservative rule of `docs/REACHABLE_FIRST_HOP_ARCHITECTURE.md` §9.

### 6.4 Failover controller

State machine per client session: `Idle → Connecting(route) → Connected(route) → Degraded → Recovering(next) → Connected | Exhausted`.

- **Retry bounds:** per route (`max_attempts`) and per recovery episode (`max_routes`). `Exhausted` stops automatic retries until a network change, a user action or the backoff timer expires.
- **Hysteresis:** switch away from a *working* route only if `score(candidate) − score(current) ≥ switch_margin` **and** the candidate has been better for `min_dwell`.
- **Cooldown:** a route that failed gets `cooldown = base × 2^(n−1)`, capped, and ignored until it expires.
- **Anti-flapping:** at most `max_switches` per `flap_window`; beyond that, pin the best working route.
- **Failure memory:** per (network fingerprint, route), decaying.
- **Probation:** a recovered route re-enters as `Probation` and becomes eligible for automatic preference only after `probation_successes` consecutive L4 successes.
- **Network transition** resets the per-network context, cancels in-flight attempts through generation fencing (Tamara already fences stale operations) and re-plans. It never counts as a route failure.
- **Mode invariant:** a Privacy+ session never falls back to direct routes (existing Tamara invariant, kept).
- **Recovery time** is measured from `Degraded` detection to L4 success on the new route, and reported. **No "zero interruption" claim:** switching nodes changes the egress IP and breaks existing TCP flows.

## 7. Multi-node

- Each node keeps its own `vpn-admin`, state and private keys (unchanged).
- The **fleet catalog** (`fleet.toml` on the provisioning authority, or the control plane in managed mode) declares nodes and endpoints and holds per-user, per-endpoint credentials received from each node.
- Credential transfer between nodes changes from "operator pastes a UUID" to a **credential bundle**: JSON listing user id, endpoint id and credential, produced by `vpn-admin user export-credentials --for-authority`. It is written mode 0600 and never logged, and is imported on the authority. Sealed-box encryption for this bundle is target work.
- Removing a node → its endpoints and routes vanish from `/v2` on the next render. Users keep their other routes. Backup of the authority includes the fleet catalog. Backup of a node remains node-local.
- **Migration from one node:** `vpn-admin fleet init` derives a one-node catalog from the existing `deployment.toml` and `users.json` without changing either file (§ MIGRATION_PLAN).

## 8. Node agent, provisioning and replacement

### 8.1 Node agent

- Runs on each node. It reports **aggregate** health (service up, per-transport L1–L2 self-probe, CPU/RAM/conntrack, active-peer count bucket) and never per-user destinations.
- **Identity:** an Ed25519 keypair generated on the node at bootstrap. Registration presents a **one-time bootstrap token** (control plane stores only its hash, TTL ≤ 1 h), node id and public key. The control plane answers with a registration record. Later status reports are signed with the node key and have a short validity (≤ 5 min) with replay protection (monotonic counter plus timestamp window).
- **Revocation:** the control plane marks the node key revoked, and reports are rejected. **Rotation:** the node signs its new public key with the old one.
- The node agent never receives user credentials for *other* nodes.

### 8.2 Provider drivers (`crates/platform-core::provider`)

```text
trait ProviderDriver {
    create_node(NodeSpec{region, size, image, ssh_key_ref, user_data}) -> ProviderNode
    destroy_node(id), get_node(id), list_nodes()
    assign_metadata(id, labels)
    configure_firewall(id, [PortRule])
    wait_until_ready(id, deadline) -> Ready | Timeout
    replace_ip(id) -> Unsupported | NewIp
}
```

- Driver errors are typed (`RateLimited`, `QuotaExceeded`, `AuthFailed`, `NotFound`, `Unavailable`, `InvalidRequest`), never stringly. Tokens are `SecretString`, never `Debug`-printable or serialised.
- Implemented: `MockProvider` (deterministic, fault-injectable) plus a reusable contract test suite that every driver must pass.
- **No live cloud driver** is implemented yet: nothing in the repository establishes which provider comes first (the existing RU/DE hosts are manually procured). This is an explicit operator decision, not a guess (IMPLEMENTATION_STATUS §External gates).

### 8.3 Bootstrap pipeline

`create → wait_until_ready → install (pinned release, verified) → configure firewall → enable transports → health gates (doctor --protocol --require-protocol, AWG handshake self-test) → register → issue route config → Active`. Any failed step → `Failed` with the node never served, then destroy or quarantine per policy. **No node becomes active until its health gates pass.**

### 8.4 Replacement orchestrator

```text
Active ─(degradation evidence ≥ threshold over window, from ≥ M independent vantage points)→ ReplacementRequested
  → [cooldown ok? budget ok? provider available?] → Provisioning(new) → Bootstrapping → Verifying
  → CredentialsProvisioned → Published (clients receive new route) → Old: Draining(drain_period) → Retired → Destroyed
```

- **Thresholds are configurable**, and one transient failure never triggers replacement. Evidence must be classified (`ENDPOINT_DOWN`, repeated `CENSORSHIP_SUSPECTED` from probe vantage points) and sustained.
- **Guards:** per-fleet cooldown, a replacement budget (count per period and cost ceiling), and a maximum of concurrent replacements.
- **Audit log:** append-only JSONL with actor, reason, evidence summary, states and timestamps. It contains no secrets.
- **Rollback:** if the new node fails verification, it is destroyed or quarantined, the old node stays Active (not drained), and the attempt counts against the budget. If the provider is unavailable, the request stays pending with bounded retries and an operator alert, and the old node is untouched.
- **Operator visibility:** `vpn-fleet node list|replace --dry-run|drain|history`.

## 9. What each component can and cannot see

| Component | Can see | Cannot see / must not store |
|---|---|---|
| Access network / censor | client IP, node IP and port, timing and volume, TLS ClientHello (REALITY decoy SNI), QUIC/AWG packet shapes | payloads (encrypted) |
| Node (exit) | client IP, tunnel identity, egress destinations (inherent to an exit) | nothing stored per destination; sing-box logs at `fatal`; AWG has no connection log |
| Relay node | client IP, the declared exit it forwards to | inner payload (end-to-end REALITY to the exit) |
| Control plane (managed) | account, entitlement, route directory, node health aggregates, opt-in route-health telemetry | destinations, DNS, SNI of user traffic, payloads, node private keys, AWG client private keys (client-generated in managed mode) |
| Provisioning authority (self-hosted) | per-user credentials for every endpoint in its catalog | node private keys of other nodes, traffic |
| Tamara client | everything local | nothing is exported unless the user opts in to telemetry |

## 10. Probe agent (synthetic measurement)

- A binary mode `probe-agent` (on the Russian VPS, or any vantage point) reads a **probe matrix** of targets × transports and periodically runs L1–L5 probes against controlled endpoints:
  - L1: TCP connect.
  - L2: REALITY via a sing-box client, Hysteria2 via sing-box, AWG via `amneziawg-go` in a netns.
  - L4: a controlled 204 URL.
  - L5: a bounded download.
- It emits `ProbeResult` records (telemetry schema, §PRIVACY_MODEL) as JSONL, labelled with vantage evidence class `RUSSIAN-DATACENTER-NETWORK` / `SIMULATED` / `LOCAL`. It never carries user traffic.
- Russian-VPS results are **RUSSIAN-DATACENTER-NETWORK VERIFIED** at most, never residential or mobile.

## 11. Multi-hop

Unchanged mechanism (Core `detour`, fail-closed relay). It is an optional route kind scored like any other route, with a latency penalty inherent in its measurements. It is never the default and never required.

## 12. Deliberate non-goals (this architecture)

- No custom cryptography, no AWG fork, no ML scoring.
- No control plane in the data path; no browsing, DNS or SNI telemetry.
- No single mandatory VPS count (1, 2 or N all work); no mandatory multi-hop; no Cloudflare/CDN dependency.
- No claim of censorship-proof operation, zero interruption, or transport dominance.
