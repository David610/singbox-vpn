# DEVICE_ACCEPTANCE_TESTS.md

Automated CI cannot validate a real Hiddify/v2rayNG import on a real
iOS/Android/MagicOS/Linux/Windows/macOS device against a real VPS — this
document is the explicit manual test matrix for that, per the honesty
rule already established in `docs/CLIENT_COMPATIBILITY.md`: a cell only
ever changes to PASS after a dated, filled-in entry exists below, never
from spec conformance or code review alone.

## Matrix

| Platform | Client | VLESS+REALITY | Hysteria2 | Subscription refresh | Network switch |
|---|---|---|---|---|---|
| iOS | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| Android | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| HONOR MagicOS | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| Android | v2rayNG | not yet tested | N/A (unsupported/not guaranteed — see `docs/clients/V2RAYNG_ANDROID.md`) | not yet tested | not yet tested |
| Linux | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| Windows | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |
| macOS | Hiddify | not yet tested | not yet tested | not yet tested | not yet tested |

## What each column means

- **VLESS+REALITY** / **Hysteria2**: the client successfully connects
  through that transport specifically (switch to it manually if the
  client auto-selects the other one first) and traffic actually egresses
  through the VPS (e.g. `curl ifconfig.me` shows the VPS's IP).
- **Subscription refresh**: after `vpn-admin user rotate-token` or
  `vpn-admin user rotate-vless`/`rotate-hysteria`, re-importing/
  refreshing the subscription in the client picks up the new
  credentials and the old ones stop working.
- **Network switch**: the connection survives (or promptly reconnects
  after) switching from Wi-Fi to mobile data and back, and after a
  screen-off idle period (mobile platforms).

## How to actually run this

Prerequisites:

1. A real AlmaLinux 9 (or Rocky Linux 9) VPS with a public IP and two
   DNS names pointed at it (`vpn.example.com`, `sub.example.com` — see
   `docs/ALMALINUX_DEPLOYMENT.md`).
2. `sudo ./deploy/almalinux/install.sh` run there, completing without
   error (a failed install must not have printed "Install complete" —
   see `docs/PRODUCTION_HARDENING_PLAN.md` #22).
3. `sudo vpn-admin doctor` (or `vpn doctor`) on the VPS reporting no
   `[FAIL]` lines.
4. `sudo vpn-admin user create --name test --qr` to get a subscription
   QR code / URL.
5. The device under test, on a real network, with the relevant client
   installed per `docs/clients/`.

For each matrix row:

```
Date:
Platform:
Client + version:
Device model / OS version:
VPS region / provider:
sing-box version (from `vpn-admin version` on the VPS):

VLESS+REALITY:          PASS/FAIL
Hysteria2:               PASS/FAIL
Subscription refresh:    PASS/FAIL
Network switch:          PASS/FAIL

Steps to disable/revoke and prove it took effect:
  1. `vpn-admin user disable test` on the VPS.
  2. Client attempts to reconnect/use the existing session — confirm it
     is rejected (REALITY/Hysteria2 handshake fails, or the client shows
     a connection error) within a reasonable time.
  3. `vpn-admin user enable test`, `vpn-admin user rotate-token test`.
  4. Re-import the new subscription URL on the client and confirm it
     connects again.
Revocation actually took effect: PASS/FAIL

Notes:
```

Paste the filled-in block above as a new dated entry directly below this
line once a real test is run, and update the corresponding matrix cell.

## Entries

_No entries yet — this document defines the procedure, not a result. Do
not mark any matrix cell PASS without a corresponding entry here._
