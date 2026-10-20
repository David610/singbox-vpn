# YouTube Shorts egress architecture — decision record (2026-10-20)

Status: **OPEN — blocked on an owner decision** (paired node authorization).
Evidence classes follow `CLAUDE.md`. Companion: `docs/YOUTUBE_FIX_INVARIANTS.md`
(protected), `docs/operations/ACCEPTANCE_THRESHOLDS.md`.

## Measured facts (browser verified, 2026-10-20, staging host 62.238.46.190)
Both address families of this host fail Shorts playback: IPv4 domain 1/4 and
2/8 (repeat), IPv6 domain 1/4, literal IPv4 0/4, literal IPv6 0/4; normal videos
0/2 and 0/4 with `LOGIN_REQUIRED "not a bot"` on one. Raw evidence:
`docs/handoffs/2026-10-20/evidence/`. The single playing item was not a true Short.
Caveat: no same-run known-good control exists on this host.

## Options

| | A. Single exit, verified-good IP | B. Selective Google hairpin (exists) | C. Multiple validated hairpin targets + failover (new) |
|---|---|---|---|
| Fits today's evidence | **No** — this IP/family is refused | Yes, if a good node is authorized | Yes, strictly more than needed today |
| Code state | n/a (plain exit) | Implemented: `[google_egress_hairpin]`, `vpn-admin google-egress-hairpin set/clear`, rules `sniff -> reject literal-IPv6 -> route`, tests INV-1/4/5/6 | **Not implemented.** Schema has one hairpin; INV-6 forbids selector/urltest alternatives for the domain set |
| Reliability | Single point: IP reputation | Single point: hairpin node + its IP reputation | Removes the single point; adds a health-decision component that must itself be correct |
| Failure mode | Shorts silently refused | Fail-closed: YouTube visibly fails (INV-6) | Fail-closed only if the last resort is REJECT, never `direct` |
| Latency / throughput | Best | +~190 ms on new QUIC sessions, extra REALITY hop (INV-8 measured) | Same per path; failover adds a switch delay equal to probe interval x threshold |
| Privacy | Best | Google traffic leaves via second operator's node (already owner-accepted for Selectel) | Same, multiplied by each target; every target sees Google-bound flows |
| Operating cost | Lowest | One extra node's capacity | N nodes |
| Testability | Probe only | Fully offline-testable rendering + loopback system test; live probe | Simulated-failure tests mandatory before enabling |
| Rollback | n/a | `google-egress-hairpin clear` (instant, all users) | Same plus failover disable |
| Approvals needed | none | Owner names/authorizes target (INV-7/INV-8) | Owner approves >=2 targets AND an INV-6/7/8 amendment |

## Decision (evidence-based, provisional)
1. **A is rejected for 62.238.46.190** on measured evidence. It remains the
   target for any node whose IP passes `youtube_evidence_probe.py` (Case A).
2. **B is the approved mechanism** (already protected by 13 invariants, already
   device-verified once on 2026-10-11). It needs one authorized good target.
   Until then this host stays staging; no real user is switched to it.
3. **C is designed, not built.** Building it before one working target exists
   adds a failure-decision component with nothing to decide between. Build it only
   after the owner authorizes >=2 targets.

## Proof required before any hairpin target is used
1. Owner names the target, confirms ownership and that Google/YouTube-bound
   traffic of all exit users may pass through it (INV-7, INV-8).
2. Target reachability from the exit: REALITY handshake succeeds.
3. **Target IP accepted by YouTube**, per family it will actually egress:
   `youtube_evidence_probe.py` >=10 distinct true Shorts, 10/10 PLAY (media
   delivered AND clock advancing), 3 runs >=1 h apart; plus a same-run
   known-bad control (this host) showing the probe can fail.
4. Dedicated user `hairpin-<exit>`, credential via `--uuid-stdin` only (INV-9).
5. Rendered config satisfies INV-1/4/5/6; `vpn-admin doctor --protocol` passes.
6. Through the real tunnel (VLESS+REALITY and Hysteria2) with hairpin on:
   Shorts 10/10, domain + literal-IPv4 + literal-IPv6 cases recorded separately.
7. Native iPhone acceptance (INV-12) — the only thing that closes the issue.

## Option C design sketch (for the owner decision, not implemented)
- Config: `[[google_egress_hairpin]]` list, each target with a priority.
- Renderer: ONE route for the Google domain set to an outbound group
  (sing-box `fallback`-style group over the hairpin outbounds only — never
  `direct`); group exhaustion => reject. This replaces INV-6's "exactly one
  route and no selector/urltest" with "exactly one route; the group contains no
  direct outbound" — an amendment of the invariant, its test and this document
  in one commit.
- Health: a node-local timer runs the Shorts media probe through each target
  (low rate, e.g. 3 Shorts / 30 min), writes aggregate pass counts only,
  demotes a target after N consecutive failures and alerts.
- Tests before enabling: render-contract tests (no direct in the group,
  ordering preserved), a loopback system test killing the primary target
  (traffic moves to the secondary; with both down, Google flows are rejected and
  non-Google flows are unaffected), restart/reinstall preservation.
- Session continuity: established QUIC/TCP flows to a failed target break and
  must be retried by the app; only new flows move.

## Not acceptable (owner decision 2026-10-11, INV-7)
Cloudflare WARP, residential/mobile proxy services, new third-party egress
services, new VPSs without approval. DNS/MTU/QUIC/transport/IPv6-disable changes
are not remedies for a source-IP refusal (INV-2).
