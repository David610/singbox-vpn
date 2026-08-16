# CLIENT_COMPATIBILITY.md

Honesty rule (spec §21): a "yes" below means either an automated check
ran against the real client, or a documented manual test was performed
and its steps/date are recorded. Nothing is marked "yes" from spec
conformance alone.

**Canonical per-platform matrix**: `docs/DEVICE_ACCEPTANCE_TESTS.md` —
this file does not duplicate it; the table below is the summary. See
`docs/CLIENT_PROTOCOL_BEHAVIOR.md` for the protocol-level facts (DNS,
IPv4/IPv6, full-tunnel, UDP/TCP, failover) that apply across every
client, and see `docs/clients/` for per-platform walkthroughs.

| Client | VLESS+REALITY | Hysteria2 | Subscription import |
|---|---|---|---|
| Hiddify (iOS/Android/Linux/Windows/macOS/MagicOS) | iOS smoke connection reported; transport was not recorded, so this protocol-specific cell remains incomplete | iOS smoke connection reported; transport was not recorded, so this protocol-specific cell remains incomplete | iOS import/connect smoke pass reported; detailed refresh/revocation matrix remains incomplete |
| v2rayNG (Android, VLESS-only fallback) | not yet tested on a real device | N/A — not supported (see `docs/clients/V2RAYNG_ANDROID.md`) | not yet tested on a real device |
| sing-box (real pinned binary, real handshake) | **verified** — real handshake against a real pinned `sing-box` binary, both success (matched keys) and failure (mismatched keys) paths | **verified** — real handshake, matched/mismatched password, obfuscated, and Brutal-bandwidth variants | **verified** — real `sing-box check` against a rendered server config, in CI on every push |

The sing-box row is genuinely verified, not schema-checked-only: this
project's own `sing-box` interop tests
(`crates/compat-config/tests/reality_interop.rs`,
`reality_decoy_budget.rs`, `hysteria2_interop.rs`) download/run the real
pinned `sing-box` binary and perform actual protocol handshakes — 9
tests, all passing as of the date `docs/COMPATIBILITY_VERSIONS.md`
records. CI's `singbox-validate` job (`.github/workflows/ci.yml`) does
this on every push, and `VPN1_REQUIRE_REAL_INTEROP=1` turns any skip
into a hard failure so this can never silently degrade to schema-only
checking again. An owner-reported iPhone/Hiddify smoke test now confirms
that a real app can import/connect through a real AlmaLinux VPS. It did
not record enough detail to assign that success to one specific transport
or complete the refresh, revocation, network-switch, DNS, and IPv6 matrix.

## Why the Hiddify/v2rayNG rows aren't fully "yes" yet

Real client/device testing requires a real AlmaLinux 9 host with a
public IP and DNS name, `sudo ./deploy/almalinux/install.sh` run there,
and an actual iOS/Android/Linux/Windows/macOS device with the relevant
client installed, on a real network, pointed at the real subscription
URL. A limited owner-reported AlmaLinux/iPhone smoke pass was recorded on
2026-08-16, but its client version, selected transport, and extended matrix
were not captured. Treat `docs/DEVICE_ACCEPTANCE_TESTS.md` as the checklist
for the remaining manual acceptance pass — see
`docs/ALMALINUX_DEPLOYMENT.md` §"Testing with Hiddify" for the exact
steps — and fill in a dated entry there once run.

## What *was* validated in this session (automated, in `cargo test`)

- Rendered VLESS URI contains `security=reality`, `pbk=`, `sid=`,
  `flow=xtls-rprx-vision`, and never contains the REALITY private key or
  any substring resembling `private`.
- Rendered Hysteria2 URI contains the user's password, `sni=`, and
  optional `obfs=salamander` parameters when configured.
- Rendered sing-box client subscription JSON contains both a `vless`
  and a `hysteria2` outbound, a `urltest` group (tag `auto`), and a
  manual `selector` group (tag `select`) whose `default` is the
  REALITY endpoint's tag — `route.final` points at `select`, so a
  freshly-imported profile deterministically starts on REALITY rather
  than whatever `urltest`'s Google-latency race happened to prefer.
  `auto` remains selectable for anyone who wants sing-box's own
  automatic switching instead — see
  `docs/TELEGRAM_RESILIENCE_PLAN.md` §A for why `urltest` alone isn't
  a safe default on a censored network. Matches the field names in the
  current sing-box docs, and never contains `private_key`.
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

`docs/DEVICE_ACCEPTANCE_TESTS.md` is now the primary place to record
per-platform results (it has a fuller per-row template and the
Telegram-specific matrix) — prefer pasting entries there. The template
below is kept for the MagicOS-specific fields it adds; either location
is acceptable as long as the entry is dated and the corresponding matrix
row/table cell is updated.

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
