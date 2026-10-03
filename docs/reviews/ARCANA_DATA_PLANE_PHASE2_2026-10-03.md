# Arcana data-plane phase 2 handoff — 2026-10-03

## Baseline and scope

Work began from `810cef5c9cb956f7cb0f43919f53fc5e4451ae1c` on the
`codex/arcana-data-plane-phase2` branch. The checkout was the repository's
platform-provided `work` branch and had one pre-existing untracked file,
`fuzz/Cargo.lock`; this work does not add or modify that file. No node was
deployed and no customer data was accessed.

The existing v2 external-authorization snapshot design was retained. It fetches
`schema=2`, validates the explicit schema and monotonically increasing snapshot
revision, persists desired/applied/acknowledged revisions separately, applies
the exact desired set, verifies live state, and acknowledges only the applied
revision. A same-revision content change and a stale revision fail closed.
Revision zero and an empty set are authoritative. Existing executable tests
cover empty removal, A -> A+B -> B -> restart, non-resurrection of revoked A,
native-user preservation, collision rejection, apply-before-ACK, failed ACK,
and exact retry after restart.

## Claim capability and lease

Before this change `Job` contained only `id`, `job_type`, and `payload`, while
completion/failure bodies contained no claim capability. Reports were retried
forever by a persistent queue keyed only by job id.

The claim response must now contain non-null strings `claim_token` and
`lease_expires_at` (RFC3339). Missing/malformed fields reject the claim. The
token is scoped to that one claim attempt and is included in every `/complete`
or `/fail` JSON body. It is redacted from `Job` debug formatting and never
logged. The agent checks the lease before executing work and checks it again
before enqueueing success. The report queue checks it before every delivery.

The queue persists the token only because crash-safe completion delivery needs
the original attempt capability; without it, a restarted old process could
report against a newly reclaimed job. The file already contains sensitive job
results and remains mode `0600`. A delivered, expired, terminal, or exhausted
entry is removed, removing the token as well.

Report semantics are:

* any 2xx is accepted (including an idempotent duplicate acknowledged by the
  server);
* 401/403 is terminal unauthorized and 404/409/410 is terminal
  gone/stale/cancelled; none is retried;
* timeout, connection error, 429, and 5xx are transient, retried independently
  with exponential backoff capped at 30 seconds and a maximum of eight total
  attempts;
* an expired or malformed lease is discarded locally, and successful work is
  never reported after its claim deadline;
* after restart, the old persisted report retains the old token. Server-side
  token comparison rejects it if another agent has reclaimed the job.

This does **not** enable vpn-web's `REQUIRE_CLAIM_TOKEN` setting. Fleet rollout
and that switch remain control-plane responsibilities.

## Authorization snapshot proof

The current implementation, rather than a replacement architecture, remains in
use. Relevant tests prove:

1. exact desired-state transitions A -> A+B -> B and B-only convergence after
   serialization/restart;
2. omission and empty snapshots remove external credentials while preserving
   native/operator/probe users;
3. revoked A cannot become active again;
4. credential-id and UUID ownership collisions are rejected;
5. lower revisions and same-revision equivocation are rejected;
6. apply failure sends no ACK, and restart/retry ACKs the exact persisted
   applied revision only.

Unsupported schema versions, legacy fields in v2, unknown protocols, identity
fields, invalid routes/secrets, excessive lifetimes, overlaps, and excess
rotation generations remain fail-closed.

## Per-credential counter feasibility

**Conclusion: not technically trustworthy with the currently shipped runtime.**

Official sing-box 1.14.1 in this repository has the Clash API but not the
`with_v2ray_api` build tag. Clash `/connections` exposes node totals and live
flow records, but it neither identifies the authenticated inbound user nor
retains closed connections. An inbound tag is shared by all credentials.
Consequently Arcana cannot currently produce an authenticated byte count for
one pseudonymous external credential without inventing attribution. Dividing
node totals, attributing by source/destination, or summing polling snapshots
would be incorrect and/or privacy-invasive; none is implemented.

The technically credible future boundary is an authenticated-runtime counter
source keyed by the already opaque `credential_id` (for example a deliberately
built and supported sing-box V2Ray StatsService integration). Choosing a custom
sing-box build, supply-chain/update support, and gRPC client are product and
runtime decisions. Until then per-credential usage is explicitly unavailable;
the existing opt-in node aggregate endpoint remains separate and must not be
presented as client usage.

## Proposed vpn-web counter ingest contract (not yet integrated)

vpn-web does not expose a verified per-credential ingest endpoint in this
checkout, so no production URL is asserted. A future endpoint should reuse the
existing node bearer authentication and node binding. It should accept only:

```json
{
  "schema_version": 1,
  "node_id": "node_opaque_id",
  "report_sequence": 184,
  "generated_at": "2026-10-03T12:34:56Z",
  "counters": [
    {
      "credential_id": "cred_pseudonymous_generation_id",
      "counter_epoch": "runtime_boot_or_reset_nonce",
      "rx_bytes_total": 123,
      "tx_bytes_total": 456,
      "connection_count_total": 7,
      "observed_at": "2026-10-03T12:34:55Z"
    }
  ]
}
```

Contract requirements:

* authenticate with the existing node credential; reject a body `node_id` that
  differs from the authenticated node (or derive it server-side);
* cap the encoded body, counter count, string lengths, timestamp skew, and
  integer values; reject unknown fields and validate `credential_id` against
  the opaque credential-id grammar;
* permit only non-negative unsigned cumulative counters. A new unpredictable
  epoch explicitly represents runtime reset/restart. Rotation generations use
  different credential IDs and must never be merged on-node;
* use `(authenticated_node_id, report_sequence)` as the idempotency/replay key.
  Equal-sequence equal-content retries return success; equal-sequence changed
  content is rejected; lower sequences are stale; increments are transactionally
  compared within `(node, credential, epoch)` so out-of-order values never
  subtract usage;
* reject credentials not assigned to that node. For a revoked credential,
  accept at most a bounded final cumulative observation whose `observed_at` is
  no later than revocation; reject later activity;
* allow no arbitrary metadata, principal/account/Link/customer identity,
  domain, DNS, URL, source/destination address, packet content, or browsing
  fields. vpn-web alone resolves credential -> external client -> Link ->
  account.

The server should return an acknowledgement containing the accepted
`report_sequence` and a stable payload digest, or explicit stale/conflict
errors. Retrying the identical report is therefore harmless. The node must not
implement a Link database or parallel authentication scheme.

## Privacy review

Normal logs contain job id/type, response status, attempt count, and bounded
aggregate node counters only. They do not contain claim tokens, claim/report
bodies, VLESS UUIDs, Hysteria passwords, configuration/subscription URLs,
subscription bearer tokens, credential ciphertext, or traffic destinations.
The claim queue is necessary encrypted-at-rest-independent local secret state
with filesystem mode `0600`; operators should protect the host state directory
as already required for provisioning URLs.

## Validation and remaining work

Repository gates and focused tests are recorded in the final task report. Unit
and mocked-HTTP tests are not real-host proof. Remaining blockers are: deploy
the claim-capable agent fleet before control-plane enforcement; validate lease
duration versus worst-case job duration; perform disposable-host reclaim/crash
testing; decide whether to own a custom statistics-enabled sing-box build; and
implement/review vpn-web counter ingest before any per-credential reporting is
enabled.
