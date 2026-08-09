# CLIENT_COMPATIBILITY.md

Honesty rule (spec §21): a "yes" below means either an automated check
ran against the real client, or a documented manual test was performed
and its steps/date are recorded. Nothing is marked "yes" from spec
conformance alone.

| Client | VLESS+REALITY | Hysteria2 | Subscription import |
|---|---|---|---|
| Hiddify (Android) | not validated | not validated | not validated |
| v2rayNG | not validated | not validated | not validated |
| sing-box (CLI / `sing-box check`) | schema-checked* | schema-checked* | JSON schema-checked* |

`*` = the config *shape* was checked against the current sing-box
configuration reference (`docs/COMPATIBILITY_VERSIONS.md`) and is
covered by unit tests in `crates/compat-config` (`render.rs`,
`server.rs`) that assert the required fields are present with the
documented names/types. This is **not** the same as running the real
`sing-box check` binary or a real handshake — this sandboxed development
environment has no network capability to install/run sing-box, no
Android device, and no public DNS/TLS certificate to test against.

## Why nothing here is marked "yes" yet

This session built and unit-tested the rendering/serving code
end-to-end against fakes (see `docs/TEST_STRATEGY.md`-equivalent
coverage in `crates/compat-config` and `services/subscription`), and
wrote (but could not execute) the AlmaLinux installer. Actually
connecting a real Hiddify install to a real VPS running real sing-box
requires:

1. A real AlmaLinux 9 host with a public IP and DNS name.
2. `sudo ./deploy/almalinux/install.sh` run there.
3. `sing-box check` against the rendered config on that host.
4. An Android device with Hiddify installed, on a real network, pointed
   at the real subscription URL.

None of these are available in this development session. Treat this
document as the checklist for that manual acceptance pass — see
`docs/ALMALINUX_DEPLOYMENT.md` §"Testing with Hiddify" for the exact
steps — and update the table above with the date and outcome once run.

## What *was* validated in this session (automated, in `cargo test`)

- Rendered VLESS URI contains `security=reality`, `pbk=`, `sid=`,
  `flow=xtls-rprx-vision`, and never contains the REALITY private key or
  any substring resembling `private`.
- Rendered Hysteria2 URI contains the user's password, `sni=`, and
  optional `obfs=salamander` parameters when configured.
- Rendered sing-box client subscription JSON contains both a `vless`
  and a `hysteria2` outbound plus a `urltest` selector, matches the
  field names in the current sing-box docs, and never contains
  `private_key`.
- Rendered sing-box *server* config excludes disabled/expired users
  entirely (revocation takes effect) and the REALITY private key value
  appears exactly once (only where sing-box itself needs it).
- `services/subscription`: valid token → 200 with the expected body
  shape; unknown/disabled/expired token → generic 404 (no
  distinguishing signal); oversized token rejected cheaply; rate
  limiting exercised; `Cache-Control: no-store` present on success and
  error responses alike.
- CI (`.github/workflows/ci.yml` job `singbox-validate`, added in the
  production-hardening pass — see `docs/PRODUCTION_HARDENING_PLAN.md`
  #11) downloads the real pinned sing-box binary and runs `sing-box
  check` against an actual rendered server config on every push — this
  is real-binary validation, still not a real client handshake.

## Manual acceptance test template

Copy this block, fill it in, and paste the result as a new dated entry
below when a real device/VPS test is actually run. A row in the table
above only ever changes to "yes" after a completed entry like this
exists — never from spec conformance or code review alone.

```
Date:
Hiddify version:
Android version:
Device:
Network:
VPS:
sing-box version:

VLESS REALITY:        PASS/FAIL
Hysteria2:             PASS/FAIL
Subscription refresh:  PASS/FAIL
Disable user:           PASS/FAIL
Rotate token:           PASS/FAIL
Switch Wi-Fi -> mobile: PASS/FAIL

Notes:
```

### HONOR MagicOS acceptance subsection

HONOR MagicOS is a named target for this project's user base but is
**not** the same thing as "generic Android" for VPN app reliability —
MagicOS's aggressive background-process/battery management has a
documented history of killing always-on VPN services that stock
Android and other OEM skins leave running. Do not claim MagicOS
support merely because a test passed on stock/AOSP Android or a
different OEM skin. A MagicOS-specific pass requires checking, in
addition to the generic template above:

```
MagicOS version:
Device model:

Battery optimization exemption granted for Hiddify: YES/NO
"Manage all apps" / background app launch allowed for Hiddify: YES/NO
VPN permission granted and persists after reboot: YES/NO
Connects over Wi-Fi: PASS/FAIL
Connects over mobile data: PASS/FAIL
Stays connected with screen off for 10+ minutes: PASS/FAIL
Reconnects automatically after Wi-Fi -> mobile data switch: PASS/FAIL
Reconnects automatically after mobile data -> Wi-Fi switch: PASS/FAIL

Notes (any MagicOS-specific settings changed to make this work):
```

No entries exist yet for either template — this section documents the
*procedure*, not a result.
