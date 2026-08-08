# Adaptive Censorship-Resistant Networking Platform

> The protocol is not the product. The adaptive connection system is the product.

An encrypted-connectivity platform that assumes any single transport,
endpoint, or relay can eventually be blocked or fingerprinted, and adapts
by scoring and switching between independent transport families instead of
depending on one protocol staying usable forever.

Start here:

- [`PLAN.md`](PLAN.md) — what this session actually built vs. deferred, and why
- [`TASKS.md`](TASKS.md) — live status of every workstream
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — system design
- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) — adversaries, what's protected, what isn't
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) — running the local dev slice

## Quickstart (local, loopback only)

```bash
cargo build --workspace
./deploy/local/run-dev-slice.sh
curl --socks5-hostname 127.0.0.1:1080 http://127.0.0.1:8081/
```

This boots a test HTTP service, a combined ingress/egress relay
(direct-tls + noise-quic), a rendezvous service issuing signed relay
bundles, and a client daemon exposing a local SOCKS5 proxy — all on
loopback with freshly generated dev-only keys. See `docs/DEPLOYMENT.md`
for the split ingress/egress topology and other variations.

## Workspace layout

See `docs/ARCHITECTURE.md#workspace-layout`.

## Status

This is a working local vertical slice with two independent, real
transport families, signed/verified configuration, adaptive
transport/endpoint selection with failure attribution, and a tested
failure-classification state machine — not a production-ready public
deployment. See `TASKS.md` for exactly what's real vs. deferred, and the
final engineering report in the session that produced this repository for
an honest gaps/next-steps list.
