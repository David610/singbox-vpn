# Arcana high-priority remediation — 2026-09-29

Scope: current `main` after PR #123 (`6eb4d88`). This pass did not revisit
already-merged C-16 rendering, probe-principal, SSRF, queue, timeout, dedup,
pinning, uninstall-ownership, or revision work. GitHub Actions, GitHub
security-alert state, and production infrastructure were not touched.

## Task A — F07 real protocol health: **PARTIAL / CROSS-REPO / EXTERNAL TEST REQUIRED**

### Current data-plane contract

The node already performs genuine protocol work with the structurally marked
reserved probe principal (never a customer credential). For each REALITY and
Hysteria2 URI it starts the pinned-compatible sing-box client, makes IPv4 HTTPS,
remote-DNS, and IPv6 requests through the tunnel, records observed egress IP,
and reports peer versus self vantage. A peer result is the relay-to-exit signal;
a self result is only the fallback. Hysteria2 uses normal certificate validation
unless the loudly reported test-only switch is enabled. The heartbeat attaches
this as additive `protocol_probe` version 1; its ordinary authenticated receipt
proves only that the agent is alive.

This pass makes that separation explicit with additive heartbeat fields
`agent_alive` and nullable `singbox_alive`. Protocol results now carry stable
`reason_code` values (`tcp_unreachable`, `client_start_failed`,
`client_not_ready`, `client_build_failed`, `handshake_failed`,
`no_ipv4_egress`) and `status`. One failed round is `degraded`; only the second
consecutive failure is `failed`, so a transient failure cannot flap readiness.
Success resets the streak. The report separately classifies certificate health
as `healthy`, `expiring` (<14 days), `expired`, or `unknown`. Existing raw
fields remain available: protocol identifies REALITY versus Hysteria2,
`dims.handshake`, `dims.dns`, `dims.https_ipv4`, `dims.ipv6`, observed v4/v6,
egress-IP match, latency/loss, vantage, and failure streak.

### Required vpn-web patch

Current singbox-vpn's wire contract is additive, but current vpn-web health was
not changed here. Its heartbeat handler and node-health computation must:

1. store/validate `agent_alive`, nullable `singbox_alive`, and
   `protocol_probe.version == 1` without treating an absent report as success;
2. select fresh **peer** results for managed-node READY decisions (self results
   may diagnose a listener but must not prove relay-to-exit reachability);
3. require each configured transport's `status == "healthy"`, required DNS and
   IPv4 dimensions, acceptable certificate status, and relay-to-exit result;
4. represent IPv6 as its own `egress`/`blocked`/`unknown` dimension rather than
   folding it into IPv4 readiness;
5. preserve the node's prior READY state for a `degraded` first failure, mark it
   FAILED only for `failed` or stale protocol evidence, and surface the supplied
   reason code; and
6. never promote a managed node to READY from heartbeat freshness alone.

A two-real-VPS relay/exit run using `SINGBOX_VERSION` and
`SINGBOX_SHA256_X86_64` from `deploy/lib/versions.env`, valid public DNS/TLS,
and externally reachable TCP+UDP remains required. This environment could not
reach the current vpn-web checkout (HTTPS clone was denied with proxy HTTP 403),
so no claim is made that its current main consumes these additive fields.

## Task B — host nftables C-16 lifecycle: **FIXED / EXTERNAL TEST REQUIRED**

The dedicated `inet arcana_egress_isolation` table now participates in normal
install, update, transactional rollback, reboot, and uninstall lifecycle.
Install records ownership of the unit, enables it, and applies it before
sing-box. Update snapshots/replaces the unit and applies it; rollback restores
(or removes, for an upgrade from a release without the unit) the prior state.
The oneshot is enabled for reboot persistence and its `ExecStop` removes only
the Arcana table. Uninstall stops it only when the ownership manifest proves
singbox-vpn created that path, then ownership-aware removal restores any
pre-existing file.

Apply and removal are idempotent. They never use `flush ruleset`, never flush an
unrelated table/chain/set, and include both IPv4 and IPv6 deny sets. A mock-nft
lifecycle regression test covers apply twice, remove twice, both families,
absence of broad flush/delete operations, and lifecycle wiring. A real host
with firewalld plus unrelated nftables rules still needs the release acceptance
run, including update failure injection, reboot, and uninstall comparison of
the unrelated ruleset before/after.

## Task C — F11 restart behavior: **FIXED (scoped optimization) / EXTERNAL TEST REQUIRED**

| Operation | Classification | sing-box behavior |
|---|---|---|
| CREATE_USER | routine authorization | one apply/restart |
| DELETE_USER | urgent security | immediate apply/restart |
| DISABLE_USER | urgent security | immediate apply/restart |
| ENABLE_USER | routine authorization | one apply/restart |
| future credential extension / clear future expiry | no-render-change | **no restart** |
| expiry crossing active→expired | urgent security | immediate apply/restart |
| expiry crossing expired→active | routine authorization | one apply/restart |
| credential rotation (VLESS/Hysteria2) | urgent security | immediate apply/restart |
| subscription-token rotation | metadata only | no restart (pre-existing behavior) |
| lease change | authorization change | one batched apply when effective users change |
| periodic expiry reconciliation with unchanged render | no-render-change | no restart (pre-existing fingerprint behavior) |
| idempotent probe creation | no-render-change | no restart; first creation applies once |
| dynamic revision | authorization change | one apply; stale/idempotent revision no restart |
| static revision | routine configuration | one apply only if rendered/runtime content changes |

`set-expiry` now compares effective authorization at one captured timestamp.
When both old and new records are active (the ordinary renewal/extension case),
it atomically persists metadata without rendering or restarting sing-box.
Crossing the authorization boundary keeps the existing validated,
rollback-capable live apply. A restart-count regression test performs future
set, future extension, and clear with zero additional restarts, then proves a
past expiry adds exactly one restart. Real client continuity during the
metadata-only path remains a release-host acceptance item.

## Task D — F03 full claim/reclaim semantics: **CROSS-REPO**

Current code is authoritative and still exposes only
`Job { id, job_type, payload }`; `/complete` and `/fail` identify only the job
ID. There is no claim ownership token, lease deadline/renewal, cancellation
state, stale-claim response, or way for this agent to prove it is the current
claimant. The disk-backed report queue makes delivery non-blocking and durable,
and operation-ID dedup prevents repeating mutations, but neither can enforce
the key invariant: an old claimant must never commit success after reclaim.
Adding a token only in this repository would be an incompatible one-sided
protocol, so none was invented.

The coordinated vpn-web change must atomically issue an unguessable claim
identity with a server-enforced expiry, permit reclaim only after expiry,
conditionally accept complete/fail only when job is still running, not
cancelled, and the presented identity equals the current unexpired claim, and
make repeated completion by that same identity idempotently return the stored
terminal result. A stale, cancelled, gone, or differently reclaimed job must
receive a distinct terminal response that the queue drops rather than retries.
Only after vpn-web lands and versions that contract should singbox-vpn add the
exact server-chosen response/body fields to `Job`, persist the claim identity
beside every queued report, submit it on complete/fail, renew long claims if the
server supports renewal, and test worker A claim → lease expiry → worker B
reclaim → A completion rejected → B completion accepted. Field names and status
codes intentionally remain owned by vpn-web rather than being guessed here.
