# 02 - Baseline and reproduction

## What could and could not be reproduced

The three customer errors came from an earlier deployment that no longer existed
on the VPS (it was a bare OS image on arrival), and no iPhone was available.
Therefore:

- Issues A and B (Hiddify iOS) could **not** be reproduced; they were analysed from
  Hiddify's source and by running Hiddify's own core against the real server output.
- Issue C (Shadowrocket / INCY import) could **not** be reproduced on a device.
  The server-side and format causes that could be proven were proven.
- Server-side defects found while building the baseline were reproduced on a real
  host and are listed below with exact steps.

## Baseline (`test-results/vps-baseline-2026-10-15.txt`)

Pristine Ubuntu 24.04.5; listening sockets: sshd only; ufw inactive; no sing-box,
no `/etc/vpn`; IPv4 and IPv6 pings out OK; public egress 91.244.71.165 and
`2a12:bec4:19a5:9aa::`. The installer dry-run passed all preflight checks
(`dryrun-stable-v1.3.0.txt`).

Fresh stable `v1.3.0`: `vpn doctor --protocol --require-protocol` PASS, release
identity recorded, 3 executables + running `vpn-admin` match the recorded hashes;
tarball SHA-256 equals `SHA256SUMS` and the VPS binaries equal the tarball's.

## Reproductions

Notation: S = server verified, C = code verified, U = user reported.

### R1 - vpn-web Hysteria2 links cannot connect (S)

1. Fetch the node's real Hysteria2 parameters from its own subscription.
2. On the VPS, run a sing-box 1.14.1 client against `127.0.0.1:443` with four variants.

| Variant | Result |
|---|---|
| decoy SNI (`www.cloudflare.com`), no obfs (what vpn-web served) | timeout |
| real SNI, no obfs | timeout (Salamander mismatch) |
| decoy SNI, with obfs | `x509: certificate is valid for <node host>, not www.cloudflare.com` |
| real SNI, with obfs | HTTP 204 |

Cause in code (C): `functions/lib/subscription-renderers.js` used `route.tlsServerName`
for both protocols and `functions/sub/[token].js` never read `node_transport_secrets`.
Hiddify's real parser accepts the broken link (`hiddify-core-parse-matrix.json`, case H),
so the import "succeeds" and nothing connects.

### R2 - user operations drop every customer's connections (S)

Harness: `churn_test.py` style run; a server-paced 25 s drip stream (1 byte/0.5 s) is held
open through the tunnel while one operation runs on a *different* user.
(An earlier version used `curl --limit-rate`, which does not pace the server: the local
socket buffer already held the data, so the stream "survived". That result was discarded.)

| Operation on another user | sing-box restarted | in-flight streams survived |
|---|---|---|
| create | yes | 0/2 |
| rotate-token | no | 2/2 |
| disable | yes | 0/2 |
| enable | yes | 0/2 |
| set-expiry | no | 2/2 |
| rotate-vless | yes | 0/2 |
| rotate-hysteria | yes | 0/2 |
| renew-native (first call mints a grant) | yes | 0/2 |
| `doctor --protocol` | no | 2/2 |
| remove | yes | 0/2 |

Separately: `renew-native` on an existing grant, `enable` on an already-enabled user and
`render-config` with no change do **not** restart (the fingerprint short-circuit works).
A bare `systemctl restart sing-box` mid-stream breaks the stream 3/3 (curl rc 18);
new connections work again 1.8-3.4 s later.

### R3 - start-limit outage (S)

The unit has `StartLimitBurst=8` per 300 s and every authorization change is a restart.
After the 8th start, the next restart is refused (`start-limit-hit`); `vpn user ...`
fails with "requested change did NOT take effect ... restore ALSO failed". Timeline from
the journal: last successful start 12:24:47, refused at 12:24:50, recovered by
`vpn-service-watchdog` at 12:28:25 = **3 min 35 s** with no service.

### R4 - Hysteria2 caps lock out every standard client (S)

With the documented opt-in `[hysteria2] up_mbps/down_mbps` set, v1.3.0 renders
`ignore_client_bandwidth: true`. A client that declares no bandwidth (all Arcana
subscriptions) gets `authentication failed, status code: 404`; a client declaring
a rate still works. `sing-box check` and the installer's REALITY-only self-test still
pass (`hy2-caps-v1.3.0.txt`). Fixed on `main` by 9a07503; absent from v1.3.0 and v1.3.1-rc.1.

### R5 - tunnel users can reach the node's own services (S)

On a default install whose `public_host` is a DNS name, `deployment.toml` records no
own address, so the C-16 own-address reject is not rendered. Through the REALITY tunnel,
`https://91.244.71.165:8443/` returned HTTP 404 (nginx). `vpn doctor` warns about it.
Setting `public_ipv4`/`public_ipv6` and re-rendering makes the same request fail (`000`).

### R6 - dev-channel "repair" does not install the requested ref (S)

Running the documented dev-channel installer over an existing installation printed
"reusing the existing root-controlled persistent source tree at /opt/singbox-vpn" and
rebuilt the *old* tree; the installed binaries lacked the new code and
`install-state.json` recorded `singbox_vpn_version: "main"`. A clean install was needed
to deploy a branch.

### R7 - UDP/443 is not usable from the test PC (environment)

A hand-built QUIC Version-Negotiation probe is answered by cloudflare-quic.com, google.com
and quic.nginx.org when sent from the VPS and by none when sent from the test PC. UDP
4433/5001 from the PC reach the VPS; UDP 443 and 53 to the VPS do not (tcpdump on the VPS,
no UFW blocks logged). Hysteria2 against the real server could not be tested from this
network; it was verified over the VPS loopback only.

## Upgrade, rollback, reboot, uninstall (S)

| Step | Result |
|---|---|
| v1.3.0 -> v1.3.1-rc.1 (official updater) | `UPDATE COMPLETE`; same credentials; byte-identical subscription sizes; REALITY client OK |
| injected failure (stopped reconcile timer, `update.sh --repair`) | health check failed, `PREVIOUS RELEASE RESTORED AND VERIFIED`; service stayed active |
| reboot | SSH back ~21 s; sing-box started 17 s after the command; REALITY client first success 0.73 s; doctor PASS |
| `uninstall.sh --yes` | exit 0; host equals baseline (only sshd, ufw inactive again, no units/binaries/users/packages/dirs; only `/etc/letsencrypt/live/README` left) |
| clean dev-channel install of the candidate | success; doctor protocol self-test PASS |

Not done: interrupted-deployment recovery and disk/memory/CPU pressure injection.
