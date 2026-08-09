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
- [`docs/IMPLEMENTATION_AUDIT.md`](docs/IMPLEMENTATION_AUDIT.md) — what existed vs. what this session added (QR onboarding, `vpn status`/`doctor`/`backup`/`restore`, client docs)
- [`docs/DEVICE_ACCEPTANCE_TESTS.md`](docs/DEVICE_ACCEPTANCE_TESTS.md) — the real-device test matrix (all cells honestly "not yet tested" until someone runs it on a real VPS + device)

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

### Quick Install

On a fresh, supported VPS (root/sudo access, public IPv4):

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh | sudo bash
```

No git clone, no manual environment variables required. This downloads
the vpn1 source (a prebuilt release if one has been published, source
otherwise), detects your OS/architecture, installs dependencies,
auto-detects your server's public IP, issues a real TLS certificate for
it automatically (via [sslip.io](https://sslip.io) + certbot — no domain
needed), stands up VLESS+REALITY and Hysteria2 behind `sing-box`,
configures the firewall, enables everything under systemd, creates a
`default` user, and prints a ready-to-import subscription URL (with a
terminal QR code, if `qrencode` is installed).

Prefer your own domain instead of the auto-assigned one? Set
`PUBLIC_HOST` first (point its DNS `A`/`AAAA` record at the server
before running):

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh \
  | sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=sub.example.com bash
```

Pin a specific release instead of the latest:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh \
  | sudo bash -s -- --version v1.2.3
```

Running the same command again is safe — it repairs/upgrades an existing
install in place without regenerating keys, duplicating firewall rules,
or destroying existing users.

### After installation

```bash
sudo vpn status              # runtime health at a glance
sudo vpn doctor               # numbered [OK]/[WARN]/[FAIL] diagnostics
sudo vpn user create --name NAME --qr
sudo vpn user list
```

`vpn` is an ergonomic alias for `vpn-admin` — both names run the same
binary. Other day-2 commands: `vpn version`, `vpn backup`/`vpn restore`,
`vpn user enable/disable/rotate-token/rotate-vless/rotate-hysteria/remove/qr`,
`deploy/almalinux/update.sh` (safe update with automatic rollback on
failed health check), `deploy/almalinux/uninstall.sh` (`--purge-state`
to also remove keys/users, `--purge-firewall` to close the ports again).

### Connect

Install [Hiddify](https://hiddify.com) (Android, iOS, Linux, Windows,
macOS) or, for Android specifically, v2rayNG also works for the VLESS
endpoint. Add a profile and either scan the printed QR code or paste the
subscription URL. See `docs/clients/README.md` for per-platform guides
(iOS, Android, HONOR MagicOS, Linux) and `docs/HIDDIFY_ANDROID.md`.

### Advanced / manual deployment

The one-liner above wraps `deploy/almalinux/install.sh` (which, despite
the directory name, supports the RHEL family — AlmaLinux, Rocky Linux,
RHEL — and the Debian family — Ubuntu, Debian; see `deploy/lib/os.sh`).
You can run it directly for full control over every install stage,
including a fully manual TLS setup:

```bash
sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=sub.example.com \
  ./deploy/almalinux/install.sh
```

See `docs/ALMALINUX_DEPLOYMENT.md` for what each of the 17 install
stages does and how to intervene manually at any of them.

See `docs/IMPLEMENTATION_AUDIT.md` for exactly what's implemented vs.
still needs a real VPS to verify, and `docs/DEVICE_ACCEPTANCE_TESTS.md`
for the manual client-import test matrix.

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

For the Hiddify-compatible server stack specifically (`install.sh` /
`deploy/almalinux/*`), see `docs/FINAL_PRODUCTION_AUDIT.md` for a detailed,
code-verified list of what was found and fixed, and
`docs/PRODUCTION_ACCEPTANCE_REPORT.md` for the current pass/fail summary,
what has and has not been verified on a real VPS/real client device, and
an honest answer to whether it can be called production-ready today
(currently: no — see that document for exactly why).
