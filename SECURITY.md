# Security Policy

## Supported versions

Security fixes are made against the latest tagged release and `main`.
There is no long-term-support branch at this time.

## Reporting a vulnerability

Please report security vulnerabilities privately, not as a public GitHub
issue:

- Email: mkhitaryandddd@gmail.com
- Or use GitHub's private "Report a vulnerability" flow under this
  repository's Security tab, if enabled.

We aim to acknowledge reports within 5 business days. Include enough
detail to reproduce the issue (affected file/command, expected vs.
actual behavior); for issues involving key material or credentials,
describe the exposure without pasting the actual secret value.

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
  singbox-vpn release artifacts).
- Anything that leaves the running server and the subscription service
  serving mutually inconsistent credentials/keys.
