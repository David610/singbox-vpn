# Non-network hardening pass — 2026-08-23

This pass intentionally improves server engineering controls without changing
VPN packet behavior.

## In scope

- static security analysis for Rust and GitHub Actions;
- pull-request dependency vulnerability review;
- bounded Dependabot maintenance for Cargo and Actions;
- immutable-SHA enforcement for third-party Actions;
- rejection of `pull_request_target` in repository workflows;
- adversarial tests around `users.json` truncation, future schema input and a
  deterministic failed atomic temp write;
- adversarial first-party provisioning-contract boundary tests;
- threat-model alignment with the actually supported singbox-vpn server.

## Explicitly out of scope

No changes are made to:

- VLESS+REALITY transport parameters;
- Hysteria2 transport parameters;
- REALITY decoy/handshake targets;
- ports;
- MTU;
- DNS;
- IPv4/IPv6 behavior;
- UDP routing;
- congestion control / Brutal / BBR configuration;
- firewall topology;
- user credentials or deployed server state.

## Acceptance

The change is acceptable only if the existing blocking CI remains green in
addition to the new security jobs. A security-tooling improvement is not allowed
to weaken or skip an existing build, test, interop, installer, rollback or
secret-logging gate.

Real-VPS and real-device acceptance remain separate evidence and are not replaced
by this pass.

## External repository setting

At the start of this pass, GitHub reported `main` as unprotected. After merging,
configure branch protection or a repository ruleset so ordinary changes require a
pull request and the blocking CI/security checks are required. This cannot be
enforced from a committed repository file alone.
