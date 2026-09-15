# Platform v2 — privacy model

**Authoritative code:** `crates/platform-core/src/telemetry.rs`. The test `privacy_model_documents_every_field` fails if this document omits any exported or forbidden field.

## 1. Principles

1. **Decide locally.** Route scoring, failover and failure classification run on the client (or on a probe agent) from local observations. The platform needs no central data to pick a route.
2. **Off by default.** `TelemetryConsent::Off` is the default. `LocalOnly` builds reports for the user's own diagnostics. Only `UploadAggregate` allows export, and it must be an explicit opt-in. `tamara-next` today has no telemetry at all (`docs/architecture/privacy.md`), and this model does not change that silently.
3. **Describe routes, not people.** A report says "route X on transport Y during hour H: N attempts, M successes, these failure classes, these latency buckets". It never says what the user did.
4. **Allowlist, not blocklist.** A serialized report is valid only if every key is in the field table below, no forbidden key appears at any depth, and no string value is shaped like a URL, e-mail, IP address, UUID, hex or base64 secret, or PEM block.
5. **Coarsen before storing.** Hour-resolution timestamps; bucketed latency, loss, throughput and recovery values; no raw samples.

## 2. What is never collected

No component of the platform, whether client, node agent, probe agent or control plane, may centrally collect:

- browsing history, destination IPs or domains, URLs, SNI of user traffic;
- DNS queries or responses;
- packet contents or application payloads;
- client public IP addresses;
- local network identifiers (SSID, BSSID, gateway MAC, the route engine's `network_id`);
- credentials, private keys, preshared keys, subscription tokens, account identifiers.

Forbidden keys, rejected at any depth by `validate_report_json`: `network_id`, `client_ip`, `ip`, `address`, `host`, `destination`, `domain`, `sni`, `server_name`, `url`, `dns`, `query`, `uuid`, `password`, `private_key`, `preshared_key`, `token`, `secret`, `credential`, `email`, `account`, `device_id`, `mac`, `ssid`, `bssid`, `imei`, `payload`.

## 3. Route-health report (`route-health/1`)

| Field | Purpose | Retention (days) | Sensitivity | Required | Locally computed |
|---|---|---:|---|---|---|
| `schema` | schema version for parsing | 90 | low | yes | yes |
| `policy_version` | which route-engine rules the client used | 90 | low | yes | yes |
| `client_version` | correlate regressions with releases | 90 | low | yes | yes |
| `platform` | platform-specific failures (TUN, sleep, handover) | 90 | low | yes | yes |
| `network_type` | separate Wi-Fi, cellular and wired behaviour | 90 | low | yes | yes |
| `access_asn` | which access networks break which routes; **separate opt-in**, coarse | 30 | medium | no | no (requires lookup; see §4) |
| `country` | regional reachability, user- or operator-declared, never geolocated | 30 | medium | no | yes |
| `route_id` | which operator route the aggregate describes | 90 | low | yes | yes |
| `transport` | transport-level reliability | 90 | low | yes | yes |
| `route_kind` | direct versus relayed reliability | 90 | low | yes | yes |
| `hour_bucket` | time-of-day and incident windows (hour resolution) | 90 | low | yes | yes |
| `attempts` | denominator for success rate | 90 | low | yes | yes |
| `successes` | success rate (L4 usable) | 90 | low | yes | yes |
| `failures` | failure classes and counts (typed, no free text) | 90 | low | yes | yes |
| `highest_layer` | how far attempts got (L1–L6) | 90 | low | no | yes |
| `handshake_ms` | handshake latency bucket | 90 | low | no | yes |
| `rtt_ms` | RTT bucket | 90 | low | no | yes |
| `jitter_ms` | jitter bucket | 90 | low | no | yes |
| `loss_permille` | packet-loss bucket | 90 | low | no | yes |
| `throughput_kbps` | sustained-transfer bucket | 90 | low | no | yes |
| `recovery_ms` | automatic recovery time bucket | 90 | low | no | yes |

Retention values are the maximum a collector may keep. A collector does not exist yet (see IMPLEMENTATION_STATUS).

### Re-identification risk

For a small self-hosted fleet, `route_id` + `hour_bucket` + `access_asn` + `country` can narrow a report to one household. Mitigations:

- `access_asn` and `country` require their own opt-in, separate from upload consent.
- A collector must drop reports for any (route, hour) cell with fewer than *k* = 5 distinct submitting installations before storing aggregates (collector requirement, not yet implemented).
- Self-hosted deployments do not upload anywhere by default. There is no default endpoint.

## 4. Network metadata policy

- The route engine keys local history by an opaque `network_id` computed on the device (e.g. a salted hash of gateway identity). It is never exported.
- `access_asn`, when a user opts in, must come from a local database or from the client's own resolver-free method. It must not come from a third-party IP-lookup API call that would reveal the user's IP to that third party. Until such a method exists in the client, the field stays unset.

## 5. Server-side logging

| Component | Logged | Not logged |
|---|---|---|
| sing-box (server) | fatal errors only (`server_log_options`: level `fatal`) | connections, destinations, user identities |
| amneziawg-go / `awg` | error-level daemon messages to journald (`LOG_LEVEL=error`, upstream default). Wireguard-go-derived error lines can name a peer by an abbreviated **public** key. | verbose handshake logs (`LOG_LEVEL=verbose` is never set), destinations, private keys |
| `vpn-subscription` | request status class and rate-limit events, no token | tokens, rendered documents, credentials |
| `vpn-admin` | operator command results | credentials (onboarding output can be suppressed with `SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1`) |
| probe agent | `ProbeResult` JSONL: target route id, layers, timings, failure class | payload contents, credentials, resolver answers |
| replacement orchestrator | append-only audit JSONL: actor, node id, states, reason class, evidence counts | provider tokens, credentials, IPs of clients |

`deploy/lib/check-no-secret-logging.sh` (CI) and the redaction tests in `telemetry.rs` guard this.

## 6. What each party can observe

Unchanged from TARGET_ARCHITECTURE §9. The data-plane nodes can observe what an exit inherently observes. The control plane observes route metadata and opt-in aggregates, never traffic.

## 7. Evidence

The redaction and allowlist behaviour is **CODE-VERIFIED** (`cargo test -p platform-core telemetry`). There is no telemetry collector and no client integration, so the end-to-end privacy of an upload path is **UNVERIFIED**, because no such path exists.
