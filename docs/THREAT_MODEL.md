# Threat model

This document defines the security boundary for the **supported singbox-vpn
server product**: a small self-hosted VPN using sing-box to serve VLESS+REALITY
and Hysteria2, with `vpn-admin` as the control plane and `subscription` as the
client-provisioning service.

The former native/adaptive prototype is not on `main` (it is archived on the
`archive/native-adaptive-stack-2026` branch) and must not be used to make
security or availability claims about the supported server. The provisioning
agent (`apps/provisioning-agent`) belongs to the deferred managed-fleet path:
a standalone install does not configure or run it.

## Assets to protect

Highest-value assets are:

1. REALITY private key material.
2. TLS private keys.
3. Per-user VLESS UUIDs and Hysteria2 passwords.
4. Hysteria2 obfuscation password when enabled.
5. Subscription/provisioning bearer tokens and URLs.
6. Backup archives containing deployment or user state.
7. The integrity of installed `vpn-admin`, `subscription`, sing-box, systemd
   units and generated sing-box configuration.
8. The integrity and availability of `users.json` and deployment state.

## Trust boundaries

### Trusted

- the operator with root access to the VPS;
- the VPS kernel and host platform while uncompromised;
- a client device while uncompromised;
- a pinned, verified singbox-vpn release produced by the repository release
  workflow;
- the selected upstream sing-box release after checksum verification.

### Untrusted

- the public Internet;
- local access networks and ISPs;
- censorship/DPI infrastructure;
- unauthenticated connections to public VPN ports;
- arbitrary requests to the subscription/provisioning service;
- **authenticated VPN users, for everything except their own tunnel.** A valid
  (or leaked) credential lets its holder send arbitrary traffic from inside the
  exit. It must not reach node loopback services, cloud metadata, private or
  special-purpose ranges, SMTP, or the node's own public addresses (C-16, see
  "Egress isolation" below), and must not be able to take the node's whole
  link by declaring a Hysteria2 bandwidth (without operator caps the exit tells
  every client to use BBR);
- malformed provisioning requests;
- malformed or corrupted persisted state presented after an operational
  failure;
- pull requests and third-party GitHub Actions until their exact revisions are
  reviewed and pinned.

### Bearer capability: subscription/provisioning URL

A valid subscription/provisioning URL is a bearer credential. Possession of the
raw token authorizes access to the associated client configuration. The server
persists a hash rather than the raw token, but an operator or client that leaks
the URL has leaked a credential.

Rotating only the subscription token revokes that URL. It does **not** revoke an
already imported VLESS UUID or Hysteria2 password; transport credentials have
separate rotation commands. The CLI/tests must state that blast radius
accurately.

## Attacker capabilities considered

The supported design assumes an attacker may:

- scan the VPS and send arbitrary bytes to exposed TCP/UDP ports;
- observe, delay, drop, reset, reorder or selectively block traffic between a
  client and VPS;
- know that the VPS hosts a VPN;
- obtain one user's client credentials without obtaining other users' secrets;
- send malformed HTTP requests and unsupported provisioning versions;
- cause ordinary operational faults such as interrupted writes, restarts, DNS
  failures, certificate-renewal failures or an update that fails validation;
- submit a malicious or compromised dependency/action update through normal
  development channels.

## Explicit non-goals

The server does **not** claim to protect against:

- a fully compromised VPS root account, kernel or hypervisor;
- a fully compromised client device;
- a malicious VPS provider with unrestricted memory/disk introspection;
- traffic-correlation anonymity comparable to Tor;
- hiding the VPN server's IP address;
- full Internet shutdowns where no external route exists;
- indefinite resistance to future censorship changes without protocol/client
  updates;
- compromise of upstream sing-box before the verified release artifact is
  produced.

This is a circumvention/privacy VPN, not an anonymity network.

## Release-blocking security invariants

### Secret isolation

- Raw subscription tokens are not persisted in `users.json`.
- Server-private REALITY/TLS key material never appears in client provisioning
  documents.
- Secret-bearing files retain restrictive owner/group permissions after create,
  update, migration, backup and restore.
- Diagnostics and normal logs do not print credential material.

### Egress isolation (C-16)

- Every exit config starts with `resolve` then rejects the C-16 IPv4/IPv6
  ranges, TCP/25 and the node's own declared public addresses for every
  inbound, ahead of `route.final = direct`; the reserved probe principal keeps
  only its narrow allow-list. A relay forwards only to declared exits and ends
  in `reject`.
- A host nftables table (`inet arcana_egress_isolation`, uid-scoped to
  `sing-box`) repeats the ranges as a second layer.
- The own-address rule needs `public_ipv4`/`public_ipv6` in `deployment.toml`;
  a domain-based install does not record them automatically.
- Enforced by `relay_role_policy.rs`
  (`exit_rendering_keeps_inbounds_outbounds_identical_and_adds_mandatory_c16_route`,
  `relay_destinations_cannot_name_internal_or_metadata_addresses`) and
  `c16_host_policy_contract.rs`.

### Privacy boundaries the server cannot remove

- The exit sees each user's source IP, opaque principal and destinations in
  memory; it logs nothing per connection (`log.level = fatal`).
- The exit resolves destinations through the host resolver; that resolver (the
  provider's, by default) can see every destination domain.
- With the Google/YouTube egress hairpin configured, that domain set leaves from
  the paired node, which sees those destinations.
- In two-hop mode the Direct and via-relay routes share one exit credential, so
  a single Direct use links that user's relayed sessions to their IP.

### Fail-closed state handling

- Corrupted or future-version state is rejected rather than interpreted as an
  empty/default configuration.
- User-state writes are atomic: a failure before the final rename must leave the
  previous live state intact.
- Generated sing-box configuration is validated before production activation.
- Restore/update failure must leave either the previous known-good state or an
  explicit failed/stopped state; it must not silently claim success.

### Credential lifecycle

- Disabling/removing a user revokes both supported transports.
- Rotating one transport credential changes only that transport unless the
  operator explicitly requests full credential rotation.
- Subscription-token rotation does not falsely claim to revoke already imported
  VLESS/Hysteria2 transport credentials.
- A mutation is not reported as live success until the affected runtime state
  has been reloaded/verified.

### Provisioning contract

- The server emits only server-owned connection facts: host/port, supported
  transport, and the credential material needed to authenticate.
- Client-owned DNS/TUN/MTU/IP-family/lifecycle policy is structurally excluded
  from the first-party provisioning contract.
- Unknown/unsupported schema versions fail explicitly rather than being silently
  reinterpreted.
- Client-facing serialization must not contain private keys, filesystem paths or
  certificate-verification opt-outs.

### Supply-chain integrity

- Production installs use published releases; mutable development installs
  require a separate explicit opt-in.
- sing-box and singbox-vpn release artifacts are checksum verified before use.
- Third-party GitHub Actions are pinned to full commit SHAs.
- `pull_request_target` is not used.
- The committed Rust dependency set is scanned by the blocking `cargo audit`
  job. GitHub's incremental dependency-review gate is an additional desired
  control but requires the repository's Dependency graph feature to be enabled.
- Release jobs grant write permissions only where publication/attestation
  requires them.

### Least privilege

- CI workflows default to read-only repository permissions and grant write
  scopes only to the jobs that require them.
- Runtime services use dedicated service identities and systemd hardening rather
  than running the data plane as unrestricted root.
- Secret state is not made world-readable for operator convenience.

## Automated evidence

The repository enforces these invariants through independent layers:

- Rust unit/integration tests for provisioning, credentials, persistence,
  migration, backup/restore and user lifecycle;
- adversarial persistence tests that reject truncated/corrupted state and force a
  pre-rename atomic-write failure to prove the previous state survives;
- shell regression tests under `deploy/lib/tests/` for installer/update/rollback
  behavior;
- `cargo audit`, clippy, formatting, docs and secret-logging CI gates;
- real pinned sing-box configuration/interoperability tests;
- CodeQL analysis for Rust and GitHub Actions;
- a workflow-policy regression test that rejects mutable external Action refs,
  `pull_request_target`, and unconstrained top-level workflow permissions.

GitHub dependency review is not claimed as active while Dependency graph is
disabled. Enable that repository feature before adding the official dependency
review action as a required pull-request check.

Automated tests prove code paths, not real-world censorship resistance. A real
host/device run remains required by `docs/VPS_ACCEPTANCE_TEST.md`.

## Repository-governance requirement

CI can detect unsafe workflow configuration, but repository settings are an
external trust control. `main` should be protected (or covered by an equivalent
GitHub ruleset) so ordinary changes require a pull request and blocking
CI/security checks cannot be bypassed by a direct push.

A repository file cannot enforce its own branch protection; this must be enabled
and verified in GitHub settings.

## Connectivity boundary

Security/recovery hardening must not be used as an excuse to retune working
network behavior. Changes to REALITY parameters, Hysteria2 parameters, DNS, MTU,
ports, congestion control, UDP handling or censorship-evasion modes require
separate evidence from controlled device and in-country tests.

For that reason, this threat-model/CI/state-integrity hardening deliberately
changes **no** VPN transport parameters.
