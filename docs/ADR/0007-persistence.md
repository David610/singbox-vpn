# ADR-0007: Persistence — files for client, Postgres if/when a server needs it

## Context
Spec §25 says prefer simple infrastructure; Postgres only where relational
persistence is actually required; no Redis/Kafka/Elasticsearch/Kubernetes
without concrete justification.

## Decision
This session's services (`rendezvous`, `relay-agent`) are stateless per
process beyond an in-memory relay pool loaded from a config file and an
in-memory rate-limiter token bucket — neither needs a database yet at this
scale. The client persists exactly one thing to disk: the cached/emergency
signed relay bundle (a single small signed file, not a database). No
Postgres, Redis, or any other infrastructure dependency was added.

## Alternatives considered
- Postgres-backed relay pool now: rejected as premature — the relay pool
  in this session is a static config-file list; a real deployment with
  dynamic relay enrollment/health (Phase 7-ish, deferred) is exactly the
  point where Postgres becomes justified, and is recorded here as the
  trigger condition for revisiting this decision.

## Consequences
No database setup/migration tooling needed for this session's local dev
environment, keeping `deploy/local` genuinely one-command-per-service.
