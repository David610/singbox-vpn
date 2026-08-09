# Adaptive Censorship-Resistant Networking Platform

> The protocol is not the product. The adaptive connection system is the product.

An encrypted-connectivity platform that assumes any single transport,
endpoint, or relay can eventually be blocked or fingerprinted, and adapts
by scoring and switching between independent transport families instead of
depending on one protocol staying usable forever.

This repository ships **two distinct client modes** — do not treat them
as the same client:

1. **Native adaptive client** — the original `client-daemon` +
   `transport-native` (direct-tls, noise-quic) stack, driven by the
   `policy`/`failure-classifier` adaptive scoring engine. Requires
   running this project's own Rust daemon on the client device.
2. **Hiddify-compatible deployment** — a VLESS+REALITY (TCP/443) and
   Hysteria2 (UDP/443) data plane, served by an external, unmodified
   `sing-box` process, with a Rust control plane (`vpn-admin`,
   `services/subscription`) for user management and subscription
   delivery. No custom client software is required — Hiddify,
   sing-box-compatible clients, and (for VLESS) v2rayNG connect
   directly. See `docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md`,
   `docs/HIDDIFY_ANDROID.md`, and `docs/ALMALINUX_DEPLOYMENT.md`.

Adaptive transport *selection* works differently in each mode — the
native client uses this repo's own `policy` scoring engine; Hiddify/
sing-box clients use sing-box's own `urltest` selector plus
server-reported endpoint health. Neither claims to control the other —
see `docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md` §"adaptive behavior" for
the exact boundary.

Start here:

- [`PLAN.md`](PLAN.md) — what this session actually built vs. deferred, and why
- [`TASKS.md`](TASKS.md) — live status of every workstream
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — system design
- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) — adversaries, what's protected, what isn't
- [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) — running the local dev slice (native mode)
- [`docs/ALMALINUX_DEPLOYMENT.md`](docs/ALMALINUX_DEPLOYMENT.md) — production deployment (Hiddify-compatible mode)
- [`docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md`](docs/COMPATIBILITY_IMPLEMENTATION_PLAN.md) — how the two modes fit together
- [`docs/PRODUCTION_HARDENING_PLAN.md`](docs/PRODUCTION_HARDENING_PLAN.md) — issue-by-issue security/operability hardening pass for the Hiddify-compatible deployment (permissions, credential revocation, TLS, rollback, CI validation), with an honest implemented-vs-verified status per item

## Quickstart: native mode (local, loopback only)

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

## Quickstart: Hiddify-compatible mode (production)

```bash
sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=sub.example.com \
  ./deploy/almalinux/install.sh
sudo vpn-admin --config /etc/vpn/deployment.toml user create --name test
```

Prints a subscription URL the user pastes directly into Hiddify — see
`docs/HIDDIFY_ANDROID.md`.

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
