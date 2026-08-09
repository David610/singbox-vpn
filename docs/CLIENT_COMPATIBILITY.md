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
  limiting exercised.
