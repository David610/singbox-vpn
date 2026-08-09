# Security Policy

## Supported versions

Security fixes are made against the latest tagged release and `main`.
There is no long-term-support branch at this time.

## Reporting a vulnerability

**TODO (operator/maintainer action required):** this repository does not
yet have a real, monitored security contact. Before this project is used
in production by anyone other than its own maintainer, replace this
section with:

- A real email address or GitHub Security Advisory link that is actively
  monitored.
- An expected response-time commitment (e.g. "acknowledged within 5
  business days").

Do not report security vulnerabilities as public GitHub issues until a
private reporting channel above is in place — use GitHub's private
"Report a vulnerability" flow under the repository's Security tab if
available, or hold the report until a contact is published here.

## Scope

In scope:

- `install.sh`, `deploy/almalinux/*`, `deploy/lib/*` (installer/deploy
  scripts)
- `crates/compat-config`, `apps/admin`, `services/subscription` (the
  Hiddify-compatible server stack's control plane)
- `crates/crypto`, `crates/config`, `crates/transport-native`,
  `services/rendezvous`, `services/relay-agent` (the native experimental
  transport stack)

Out of scope:

- The upstream `sing-box` binary itself (report to
  https://github.com/SagerNet/sing-box)
- Third-party client apps (Hiddify, v2rayNG, etc.)

## What this project treats as security-relevant

- Anything that exposes a private key, subscription token, VLESS UUID, or
  Hysteria2 password to a user/process that should not have it (see
  `docs/FINAL_PRODUCTION_AUDIT.md` for the current filesystem/logging
  permission model).
- Anything that allows a firewall/SSH lockout during install/update.
- Anything that allows installation of unverified binaries (sing-box,
  vpn1 release artifacts).
- Anything that leaves the running server and the subscription service
  serving mutually inconsistent credentials/keys.
