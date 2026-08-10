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

Follow these steps **in order** on a fresh, supported VPS (root/sudo
access, public IPv4). Skipping straight to "run the installer" without
step 1 is fine too — you'll just get an auto-assigned hostname instead
of your own domain.

### 1. (Optional) Point your own domain at the VPS

If you want `vpn.example.com` instead of an auto-assigned
`<ip>.sslip.io` hostname, create the DNS record *before* running the
installer:

- Add an `A` record (and `AAAA` if the VPS has IPv6) for your chosen
  hostname pointing at the VPS's public IP.
- **If you're on Cloudflare (or any proxying DNS host): the record
  must be "DNS only" (grey cloud), not proxied (orange cloud).**
  VLESS+REALITY needs a real, unproxied TLS handshake on 443/tcp and
  Hysteria2 needs raw UDP/443 passthrough — an HTTP(S) proxy in front
  breaks both.
- If your domain uses non-ASCII characters (Cyrillic, etc.), use its
  ASCII/punycode form (`xn--...`) everywhere below — TLS/ACME/nginx
  all require ASCII hostnames, even though your DNS dashboard may
  display the Unicode form.
- Give DNS a minute to propagate, then confirm before installing:
  `dig +short vpn.example.com` (or `getent hosts vpn.example.com`)
  should return the VPS's IP.

Skip this step to get a zero-touch install with an auto-assigned
`sslip.io` hostname instead (no DNS setup required, real TLS cert
issued automatically either way).

### 2. Run the installer

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh \
  | sudo REALITY_HANDSHAKE_SERVER=www.cloudflare.com bash
```

`REALITY_HANDSHAKE_SERVER` is explicit because REALITY decoys are an
external protocol dependency, not a safe universal default. The installer
runs a real authenticated sing-box round trip and refuses completion if the
selected endpoint is incompatible; `www.microsoft.com` is known to exceed
the pinned sing-box/utls 8192-byte handshake budget and must not be used.
The command downloads the vpn1 source (a prebuilt release if
one has been published, source otherwise), detects your OS/architecture,
installs dependencies, auto-detects your server's public IP, issues a
real TLS certificate for it automatically (via [sslip.io](https://sslip.io)
+ certbot — no domain needed), stands up VLESS+REALITY and Hysteria2
behind `sing-box`, configures the firewall and SELinux (RHEL family),
enables everything under systemd, creates a `default` user, and prints
a ready-to-import subscription URL with a terminal QR code.

If you set up a domain in step 1, pass it explicitly instead of
auto-detecting (the installer also prompts for one interactively if
you run it without this and a real terminal is attached):

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh \
  | sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=vpn.example.com \
    REALITY_HANDSHAKE_SERVER=www.cloudflare.com bash
```

If something else on the VPS already listens on 8443 (the default
subscription-HTTPS port), relocate it with `SUBSCRIPTION_PORT`:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh \
  | sudo PUBLIC_HOST=vpn.example.com SUBSCRIPTION_HOST=vpn.example.com \
    SUBSCRIPTION_PORT=8444 REALITY_HANDSHAKE_SERVER=www.cloudflare.com bash
```

Pin a specific release instead of the latest:

```bash
curl -fsSL https://raw.githubusercontent.com/David610/vpn1/main/install.sh \
  | sudo REALITY_HANDSHAKE_SERVER=www.cloudflare.com bash -s -- --version v1.2.3
```

Running the same command again is safe and expected if it fails partway
through (transient network issues, etc.) — it repairs/upgrades an
existing install in place without regenerating keys, duplicating
firewall rules, or destroying existing users. A failed run always
prints the exact diagnostic commands to check next.

### 3. Get your subscription URL

The installer prints a subscription URL + QR code for a new `default`
user automatically — but if a user with that name already exists from
an earlier run, it skips recreating it (never silently rotates a
credential a real client might already be using). Get it yourself:

```bash
sudo vpn user list                       # find the user's ID (NOT its display name)
sudo vpn user rotate-token <user-id> --qr    # prints subscription URL + QR
```

`vpn user`/`vpn-admin` commands take the user's **ID** column from
`vpn user list` (e.g. `user_dd466bb1-...`), not its `NAME` column.

If `sudo vpn ...` reports `command not found` even though installation
succeeded, it's a `PATH` issue, not a missing binary: some
distributions' `sudo` uses a `secure_path` that excludes
`/usr/local/bin`. Use the full path instead: `sudo /usr/local/bin/vpn ...`.

### 4. Connect

Install [Hiddify](https://hiddify.com) (Android, iOS, Linux, Windows,
macOS) or, for Android specifically, v2rayNG also works for the VLESS
endpoint. Add a profile and either scan the printed QR code or paste the
subscription URL. See `docs/clients/README.md` for per-platform guides
(iOS, Android, HONOR MagicOS, Linux) and `docs/HIDDIFY_ANDROID.md`.

### 5. Day-2 operations

```bash
sudo vpn status               # runtime health at a glance
sudo vpn doctor                # numbered [OK]/[WARN]/[FAIL] diagnostics
sudo vpn user create --name NAME --qr
```

`vpn` is an ergonomic alias for `vpn-admin` — both names run the same
binary. Other day-2 commands: `vpn version`, `vpn backup`/`vpn restore`,
`vpn user enable/disable/rotate-token/rotate-vless/rotate-hysteria/remove/qr`
(all keyed by user ID), `deploy/almalinux/update.sh` (safe update with
automatic rollback on failed health check), `deploy/almalinux/uninstall.sh`
(`--purge-state` to also remove keys/users, `--purge-firewall` to close
the ports again).

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
