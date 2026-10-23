# Security and reliability issue ledger — 2026-10-23

The canonical source is `docs/handoffs/2026-10-20/issue-registry.json`, regenerated with
`python3 deploy/lib/registry/build_issue_registry.py` (62 re-assessed + 34 carried = 96 entries). Roughly 90 historical entries are **not**
90 current defects: the table classifies by state, and only the rows below need action.

## Registry by state (after this regeneration)

| State | Meaning | Count |
|---|---|---|
| CI_VERIFIED / SERVER_VERIFIED | evidence at the stated class (CI run / staging host / external service) | 12 / 21 |
| FIXED_IN_BRANCH | fix and test exist; no class-level evidence beyond that | 27 |
| UNVERIFIED | carried from the 2026-10-15 audit, not re-verified | 14 |
| BLOCKED | needs the host, a device or an owner decision | 7 |
| CONFIRMED_OPEN | reproduced, not fixed | 11 |
| FLAKY / IN_PROGRESS | 3 / 1 | 4 |

## New this session

| ID | Sev | Finding | Root cause location | Fix | Regression test | State |
|---|---|---|---|---|---|---|
| I-85 | high | `main` had no enforced required checks | ruleset 22482226 | required_status_checks added (strict), nothing removed | `verify-main-protection.py` PASS; PR #185 BLOCKED while `offline-full-verdict` failed | SERVER_VERIFIED (external service; no merge call attempted) |
| I-99 | high | entitlement revocations never applied by the agent (**on `main`**) | `external_authorization.rs::adopt_snapshot` | singbox-vpn #186 | `same_revision_*` (fail on the old code) + end-to-end | FIXED_IN_BRANCH |
| I-98 | high | DB fault → 401 → agent drops job reports | vpn-web `functions/lib/node-auth.js` | vpn-web #85 | node-auth tests + app test + real stack | FIXED_IN_BRANCH |
| I-97 | medium | stale event-hub cache delays revocation ≤25 s | vpn-web `functions/lib/agent-events.js` | vpn-web #85 | `agent-events.test.js` + real stack | FIXED_IN_BRANCH |
| I-101 | medium | app-before-migration ⇒ 503 on `/events` ⇒ agent throttled | vpn-web `server/event-source.mjs`, `events.js` | vpn-web #85 | two tests + real stack | FIXED_IN_BRANCH |
| I-100 | medium | un-revoke (renewal) never reaches the node at the same revision | agent rule × control plane | none — design decision | unit test pins refusal | CONFIRMED_OPEN |
| I-102 | medium | one out-of-policy credential blocks the whole snapshot (incl. revocations) | `compat-config` validate × no DB CHECK | none — design decision | reproduced; no test yet | CONFIRMED_OPEN |

## Open findings, classified by what is needed (not all are defects to fix in this repository)

| Class | IDs | Needs |
|---|---|---|
| Owner decision / host trust | I-86, I-93 (critical), I-39 | owner confirms the 5 logins; key-only SSH proven before password login is disabled |
| Native device | I-38, I-20, I-21, I-22, I-28; Shorts differential (#169) | iPhone matrix; staging host good hairpin target |
| Client-side upstream defects | I-08, I-09 (Hiddify iOS), I-05 (INCY), I-04 | not fixable server-side; document and track upstream |
| Product/design | I-10 (7 of 10 user operations restart sing-box and drop connections), I-60 (REALITY decoy adds ~60–80 ms/connection), I-100, I-102 | design decision |
| Release | I-13 (stable v1.3.0 lacks fixes on main) | resolved by the next release containing main |
| Test coverage | I-30 (DNS-leak, IPv6-leak, kill-switch, disk/memory pressure) | hosted test environment |
| Carried, UNVERIFIED | I-01, I-02, I-03, I-06, I-07, … | I-01/I-02 re-verified at code level this session (vpn-web regression tests, suite 1424/1424); the registry still lists them UNVERIFIED because no client import was run |

## Preserved invariants

No Google/YouTube route, sniff order, literal-IPv6 reject, UDP/443 handling, hairpin or egress-isolation code was modified. No routing change was made because no routing defect
was demonstrated this session.
