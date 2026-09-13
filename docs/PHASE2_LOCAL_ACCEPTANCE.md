# Phase 2 local acceptance ledger

**Scope:** local/configuration mechanics only.  
**Evidence labels:** `VERIFIED_UNIT`, `VERIFIED_LOCAL_CORE`, `SIMULATED_ALLOWLIST`.  
**Explicitly not evidence for:** a real VPS, a real reachable ingress, Russia, a carrier/ISP allowlist, DPI/SNI/UDP filtering, or censorship resistance.

This ledger closes the locally testable Stage-1 work from `REACHABLE_FIRST_HOP_ARCHITECTURE.md`. Production first-hop technology is intentionally not selected here: its selection depends on later real-network reachability, provider-policy, latency, bandwidth, and abuse-operability evidence.

## Implemented boundaries

The server contract now carries optional non-secret `access_paths` metadata while endpoint `path` stays an opaque reference. Tamara ingests and persists the same metadata, but the credential-bearing relay and exit configuration remains inside opaque `singbox_config`; Dart does not parse or construct relay protocol configuration.

The local Core harness in Tamara uses the checksum-pinned Hiddify Core and its normal config builder. Its loopback fixture provides two authenticated first hops, `R1` and `R2`, and one exit, `E1`. A direct Core connection to the same E1 socket is deliberately refused while both `R1 -> E1` and `R2 -> E1` are permitted. Hidden relay outbounds stay out of the user-selectable exit list. No firewall, host route, DNS, adapter, TUN, or external infrastructure is modified.

## Required local cases

| # | Required case | Local result | Evidence |
|---|---|---|---|
| 1 | direct E1 denied, R1 reachable -> `E1 via R1` succeeds | PASS | Tamara `scripts/test_phase2_local.sh` + `test/interop/phase2_local_servers.py`; real pinned builder/runtime, loopback only (`VERIFIED_LOCAL_CORE`, `SIMULATED_ALLOWLIST`) |
| 2 | R1 route unavailable -> do not infer that E1 itself failed | PASS | Tamara route-candidate selector tests keep `E1 via R2` unknown/eligible after `E1 via R1` fails (`VERIFIED_UNIT`) |
| 3 | R2 reachable -> `E1 via R2` succeeds | PASS | same pinned-Core harness proves the independent authenticated R2 detour to the same E1 socket; selector tests also move from R1 to R2 (`VERIFIED_LOCAL_CORE`, `SIMULATED_ALLOWLIST`, `VERIFIED_UNIT`) |
| 4 | all E1 routes fail -> E2 routes remain candidates | PASS | Tamara route-candidate selector matrix (`VERIFIED_UNIT`) |
| 5 | recovered relay-pathed route must not flap away from a healthy incumbent | PASS | Tamara route-candidate selector matrix; healthy incumbent remains selected even when the recovered alternative reports lower latency (`VERIFIED_UNIT`) |
| 6 | failing Manual relay route -> no silent fallback | PASS | Tamara resilience mode/coordinator test pins `E1 via R1`, makes alternatives healthy, and proves only the manual tag is probed/applied (`VERIFIED_UNIT`) |
| 7 | stale result after network/catalog generation change -> no side effect | PASS | existing Tamara resilience coordinator generation/supersession tests; the authority check is route-agnostic and therefore applies equally to relay-pathed tags (`VERIFIED_UNIT`, with the Phase-1 local-Core harness also covering catalog-generation staleness) |
| 8 | relay credential rotation -> invalidate old route health without storing secret in metadata | PASS | Tamara provisioning replacement uses a fresh non-secret catalog generation even when endpoint/path metadata is unchanged; refresh reconnect/rebuild tests prove invalidation, and catalog tests prove password/UUID/private-key/raw-config material is absent (`VERIFIED_UNIT`) |

## Security assertions retained

- `access_paths` is metadata only; server deployment parsing rejects unknown and credential-shaped keys.
- Tamara's catalog types have no field capable of storing relay credentials or raw Core configuration.
- First-hop authentication is required in the local harness; the test relay is not an open proxy.
- A failed chained route is attributed only to that concrete route candidate. It is not automatically promoted into a claim that a relay, exit, protocol, DPI system, country, or provider caused the failure.
- Core remains the only VPN/protocol engine. Normal route changes still select a concrete outbound; Tamara does not implement relay protocols in Dart.
- The fixture is contained to loopback destinations and is labeled `SIMULATED_ALLOWLIST`, never `RUSSIA_VERIFIED` or equivalent.

## Deliberately deferred to the real-testing stage

The following are not local implementation gaps and do not block this Stage-1 ledger: selection of the production first-hop transport/provider; real per-user relay credential issuance against that selected service; a real second exit/VPS/provider/ASN; real REALITY/Hysteria2 chaining through the chosen ingress; Russian mobile/fixed-network reachability; destination-IP allowlisting; SNI/DPI/UDP restrictions; relay outage/recovery on real infrastructure; physical-device DNS/leak acceptance; and long-duration performance/stability.

If later real evidence shows that no legitimate reachable destination can relay onward, this architecture has reached its fundamental limit and must not be represented as capable of creating a missing network path.
