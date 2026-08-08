# THREAT_MODEL.md

## Assets

- User connectivity / ability to reach the open Internet.
- Confidentiality of tunneled traffic contents and destinations.
- Client's local key material (session keys, cached bundles).
- Rendezvous/control-plane signing keys.
- Relay fleet availability and diversity (AS/geography/provider).
- Client software integrity (no arbitrary remote code execution).

## Adversaries (ranked by capability relevant to this system)

1. **Passive traffic observer** (ISP/state-level DPI, no injection) — sees
   packet sizes/timing/TLS or QUIC fingerprints on the wire.
2. **Active network interferer** — resets TCP, drops UDP, forges DNS,
   blocks IP ranges/ASNs, time-limited blocking windows.
3. **Active endpoint prober** — connects to suspected relay IPs and sends
   crafted/replayed traffic to distinguish them from decoy services.
4. **Malicious or compromised relay operator** — sees relayed ciphertext
   and connection metadata (timing, volume) for sessions it forwards.
5. **Compromised rendezvous service** — could try to enumerate the full
   fleet, target specific clients with bad bundles, or deny service.
6. **Compromised telemetry/measurement endpoint** — could try to
   re-identify users from event streams or poison scoring.
7. **Compromised transport module / supply-chain attacker** — could try to
   ship malicious code through the update channel (only relevant once a
   pluggable transport runtime exists — see ADR-0003).
8. **Sybil clients** — many fake clients reporting good/bad telemetry to
   bias endpoint scoring or exhaust an endpoint's failure budget.
9. **Denial-of-service actor** — floods rendezvous/ingress to exhaust
   resources.

## What this system protects against

- A single blocked/fingerprinted transport does not remove connectivity —
  independent transport families are selected adaptively (§DECISION_ENGINE).
- A single blocked/failed endpoint does not remove a transport family, and
  does not get globally delisted from one client's report (§policy crate
  quarantine: local-only until corroborated).
- Full relay-fleet enumeration through one client request is not possible —
  rendezvous issues small, signed, expiring subsets (§RENDEZVOUS_DESIGN.md).
- Tampered or expired configuration/relay bundles are rejected client-side
  (ed25519 signature + expiry + schema version checks, §config crate).
- Unauthenticated probes to ingress get a response that does not trivially
  distinguish "real relay" from "closed port" beyond what the transport's
  own standard protocol already reveals (TLS: normal TLS alert behavior;
  QUIC: normal QUIC version-negotiation/stateless-reset behavior). We do
  not invent custom probing-resistance tricks — see ADR-0002.
- Telemetry is schema-limited to coarse, non-identifying fields (see
  `docs/TELEMETRY_DICTIONARY.md`); it cannot carry destination URLs or
  payloads because the wire schema has no such field.

## What this system explicitly does NOT protect against (documented, not hidden)

- **Full Internet shutdowns**: if no external route exists, no VPN can
  manufacture one. The client must classify "no external route exists" as
  a distinct state from "circumvention failed" (§FAILURE_CLASSIFICATION.md)
  and say so in status output rather than implying it can always get
  through.
- **A fully global passive+active adversary that can correlate traffic
  timing across every relay and every client** (traffic-confirmation /
  end-to-end correlation attacks) — out of scope for a circumvention tool;
  this is a Tor-class research problem, not something bolted on here.
- **A malicious relay operator learning connection metadata for sessions it
  forwards** — using a single relay hop always trusts that operator with
  timing/volume. Two-hop (ingress≠egress, different operators) reduces but
  does not eliminate this; it is a configurable tradeoff (ADR-0006), not a
  guarantee.
- **Compromise of the client host itself** (malware, OS-level surveillance)
  is out of scope.
- **WASM transport sandbox escape** cannot be assessed because the sandbox
  is not implemented in this session (ADR-0003) — no pluggable third-party
  transport code executes today, so this risk does not yet exist in the
  running system, but it is not "solved" either.

## Non-goals (explicitly, per spec §43)

This software contains no credential theft, malware persistence, exploit
delivery, unauthorized third-party access, botnet functionality, DDoS
tooling, vulnerability exploitation, malicious scanning, stealth
installation, or destructive behavior of any kind.

## Abuse cases considered

- **Endpoint exhaustion attack**: an attacker deliberately fails many
  connections to a healthy relay hoping clients globally blacklist it.
  Mitigation: `policy` crate scoring is per-client-local first; only a
  documented, not-yet-built cross-client aggregation step (control plane,
  deferred) could promote local quarantine to global policy, and that step
  requires multiple independent reporters, not one (see `policy` module
  docs and ADR-0006 for the aggregation contract this leaves for later).
- **Replay of a captured rendezvous response**: mitigated by bundle
  `issued_at`/`expires_at` and short validity window enforced client-side;
  the rendezvous server also binds bundles to a client-presented nonce it
  is not required to trust indefinitely (see `services/rendezvous`).
- **Relay enumeration via repeated rendezvous requests**: mitigated by
  returning a bounded random subset per request rather than deterministic
  paging; full-fleet-by-many-requests is a rate-limiting / operational
  concern flagged in DEPLOYMENT.md, not fully solved by protocol alone.
- **Sybil telemetry poisoning**: telemetry has no effect on this session's
  implemented client behavior (no aggregation service exists yet — see
  Phase 7 in TASKS.md), so poisoning risk against *this codebase* is
  currently zero; documented so the future aggregation service is built
  with Sybil resistance as a requirement from day one, not bolted on.

## Adversarial self-review (§54 of the spec)

Performed against the implementation as it stands at the end of this
session; see the "Security self-review" appendix at the bottom of this
document (added by the final review pass) for concrete findings and fixes
applied.

### Security self-review (final pass)

- **As a DPI engineer**: the two transports use standard TLS 1.3 and
  standard QUIC libraries (rustls, quinn) rather than a custom wire format,
  so there is no bespoke framing pattern to fingerprint beyond what
  fingerprinting TLS/QUIC in general already achieves. Risk: both are
  still "generic TLS/QUIC to a possibly-unusual host", which is a weaker
  disguise than a browser-shaped transport (Family A in the original spec
  would ideally mimic real HTTP traffic shape); documented as future work
  rather than claimed as solved.
- **As an active censor**: probing an ingress port gets ordinary TLS/QUIC
  server behavior, not a custom banner — reduces trivial active-probe
  signatures. The client's fallback timing is jittered (`policy` crate)
  specifically so many clients don't all retry transport B at the same
  fixed offset after A fails, which would itself be a network-visible
  pattern.
- **As a malicious relay operator**: egress sees plaintext destination
  host:port (it must, to dial out) and plaintext bytes after that point
  when the ultimate destination isn't itself using TLS — this is inherent
  to being an egress relay and is documented, not hidden.
- **As a supply-chain attacker**: there is no pluggable/downloaded
  transport code execution path in this codebase yet (Phase 5 deferred),
  so this attack surface does not currently exist; config bundles fetched
  from rendezvous are signature-checked before any field is trusted.
- **As a privacy researcher**: telemetry schema was checked field-by-field
  against `TELEMETRY_DICTIONARY.md` to confirm no destination, payload, or
  long-lived identifier field exists in the wire type.
- **As an SRE**: single points of failure identified — the offline root
  signing key (by design, rarely used); the rendezvous service (mitigated
  by client-side emergency-bundle caching, see RENDEZVOUS_DESIGN.md).
