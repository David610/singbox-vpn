# Arcana phase 1 implementation assessment

Assessment performed 2026-10-01 against commit `ae5f3d8`, before implementation changes.

## Current implementation

* The production data plane is the compatibility stack: `vpn-admin`, `vpn-provisioning-agent`,
  `vpn-subscription`, and `compat-config`. VLESS+REALITY and Hysteria2 are rendered from one
  contract. The retired native/adaptive prototype is not on `main`.
* Exit and relay roles are structurally distinct. Relay routing is allowlisted to declared exit
  forwarding and otherwise rejected. The existing Core configuration expresses Privacy+ as a
  nested authenticated relay detour; it must not be replaced by server-side forwarding.
* Users currently have one VLESS UUID, one Hysteria2 secret, `enabled`, and an optional
  `expires_at`. Rendering filters inactive users at the current clock. Disable/revoke and secret
  rotation use validate, atomic rename, full sing-box restart, health verification, and rollback.
  sing-box has no supported in-process configuration reload, so a changed authentication set
  interrupts active connections. Extending only stored expiry can avoid a restart while the
  already-rendered credential remains unchanged.
* Atomic state/config writers fsync files and parent directories, preserve ownership, and apply
  restrictive modes. systemd units, watchdog/recovery, firewall ownership, C-16 egress denial,
  fatal-only sing-box logging, REALITY decoy checks, lifecycle tests, and real-binary interop tests
  are already present. The lease-pool implementation already demonstrates restart-persistent,
  bounded credentials, but it is an internal pool rather than the public two-class contract.
* Existing public subscription outputs expose physical endpoint labels. There is no complete
  logical-product capability document and no first-class A/B validity interval per credential.

## Gaps and implementation direction

Phase 1 therefore adds a node-safe authorization contract with opaque principals, explicit
closed/open validity intervals, bounded A/B overlap, deterministic active-set projection,
immediate revocation flags, and class-specific lifetime policy. Native renewal changes only
authorization expiry and preserves protocol secrets. Compatibility leases permit configurable
6-hour through 30-day lifetimes; policy selection remains a control-plane decision.

The implementation now projects validated authorization metadata into the existing `CompatUser`
store rather than introducing a second credential database. Credential-backed records use opaque
credential IDs as user IDs, opaque principals as names, `created_at`/`expires_at` as the rolling
closed/open interval, and `enabled=false` as revocation. The production role-aware renderer
reconstructs and validates the authorization set before emitting either inbound.

The logical-route and client-capability contract explicitly fails closed for Privacy+: share-link
clients cannot represent its two-hop composition. A changed active credential set still requires
the existing atomic full-restart transaction; no claim of zero-disruption dynamic sing-box auth is
made. A renewal that changes no rendered credential is the zero-downtime path.

GitHub's public Actions API returned HTTP 403 in this environment and `gh` had no authentication,
so latest remote CI failures could not be inspected. Local CI-equivalent checks are recorded with
the change; remote CI status must be read from the resulting pull request.
