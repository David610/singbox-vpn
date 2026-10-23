# Production-readiness record — 2026-10-23

> **CORRECTION (2026-10-23, later):** the agent session runs on the intended VPS itself, so the integration stack described below as "disposable" ran on that host (build tools, a test PostgreSQL cluster, Node and Rust under `/root` and `/opt`; nothing left listening; the test stack was later removed from the host with the owner's approval, apart from some leftover clone directories). Statements below that no host was touched are superseded by this notice. Host-specific findings (logins, SSH and firewall state) are deliberately **not** published in this public repository; they are in the owner's private evidence.

**Decision: NO-GO for both scopes.** This is a dated record, not a release and not authorization to change a host.
It supersedes nothing in `../2026-10-22/`; it adds what was done since.

| Scope | Verdict | Why |
|---|---|---|
| Standalone VPN (≤10 trusted users) | **NO-GO** | host not yet hardened or baselined (I-39; login attribution itself is resolved, see corrections log), no exact-artifact install, no native device evidence; Shorts root cause UNKNOWN |
| Commercial Arcana | **NO-GO** | agents intentionally still disabled; vpn-web #84/#85 not merged or deployed; I-100/I-102 fixes in review (see INTEGRATION-EVIDENCE.md); Worker CPU unmeasured; accounts/billing/Telegram end-to-end not run |

## Status table

| Gate | Status | Evidence |
|---|---|---|
| Code on `main` 548a376 | PASS | CI and Security success on that exact SHA |
| Main-branch protection requires CI + CodeQL | PASS (external service) | ruleset 22482226; `verify-main-protection.py` PASS; PR #185 `BLOCKED` while `offline-full-verdict` failed. No merge call attempted. Required approvals still 0 (owner decision) |
| Paired agent × control-plane integration | FAIL → fixes in review | 5 defects found (2 in #84, 3 on `main`); 3 fixed in branches, 2 open — [INTEGRATION-EVIDENCE.md](INTEGRATION-EVIDENCE.md) |
| Worker CPU / real 429 / replay through HTTP | UNVERIFIED | no Workers runtime here |
| Documentation links/anchors | PASS (offline gate added to CI) | [DOCS-AUDIT.md](DOCS-AUDIT.md) |
| Release candidate rc.3 | NOT CREATED | [RELEASE-CANDIDATE.md](RELEASE-CANDIDATE.md); local build determinism PASS, SBOM PASS; signing/provenance/container UNVERIFIED |
| VPS access | read-only inspection done; trust and hardening still open (details private) | superseded earlier text: SSH refused the only available key; handoff `docs/handoffs/2026-10-23/VPS-ACCESS-AND-TRUST.md` |
| Login attribution (I-86 / #167) | RESOLVED by owner statement (user-reported) | owner confirmed on 2026-10-23 that all five historical root logins were theirs and authorized. This does **not** prove ownership of the source networks or that the host is secure |
| Host trust / hardening | OPEN | root password SSH still enabled; the owner's own key is not installed yet; no clean baseline signed off (details private) |
| Server qualified | BLOCKED | the VPN is not installed and no exact artifact exists; read-only inspection is not qualification |
| Native Shorts, 8 cells | BLOCKED | runbook `docs/operations/IPHONE_SHORTS_MATRIX_RUNBOOK.md`; all cells BLOCKED |
| Shorts root cause | UNKNOWN | no routing change made; none was justified by evidence |
| Backups / monitoring / DR on the real host | BLOCKED | needs the host and an off-host target |

## Corrections log (original claim → correction → currently verified)

Earlier text is kept in place in the documents it appears in; this table is the reconciliation.

| Topic | Original claim | Correction | Currently verified (as of the last update of this file) |
|---|---|---|---|
| Test isolation | "disposable", "no production system touched", nothing run on a host (`INTEGRATION-EVIDENCE.md`, `integration/README.md`, `status.json`, handoff) | The agent session runs on the intended VPS itself, so the integration stack ran on that host | Build tools, Node, Rust and a test PostgreSQL cluster were installed and later removed (PostgreSQL, Node, Rust and the build directories were removed; verified absent on 2026-10-23, remover not recorded). Leftover: build logs and a package cache under `/root` (private inventory). Test results are unaffected; the isolation claim was false |
| Host access | "SSH refuses the only available key", "No production connection succeeded", "Server qualified: no host access" (handoff, earlier README rows) | True when written; superseded when the owner installed the staging key | Key-based root login works (staging key only). Read-only inspection performed; the only change the agent itself made on the host was disabling the test database's autostart; a firewall allowing only SSH is now active (enabled by the owner or on the owner's instruction) |
| VPS modifications | "nothing was deployed" (`INTEGRATION-EVIDENCE.md`) | Accurate for the VPN and for any real service, inaccurate as to the host: packages, a database and toolchains were installed | See isolation row. No VPN software, no sing-box, no credential or SSH configuration change by the agent |
| Database removal | "autostart disabled" (earlier README correction) | Later superseded | PostgreSQL packages, clusters, the extra apt source and the `postgres` account are gone (verified 2026-10-23) |
| Trust status | "host trust unresolved / BLOCKED on owner confirmation" (`SECURITY-PRIVACY.md`, README, `status.json`, handoff, I-86/I-93) | Owner attributed all five logins on 2026-10-23 | **Login attribution: resolved (user-reported).** Not resolved and tracked separately: source-network ownership (unverified), overall host security (I-39), possibility of altered logs |
| SSH access | "password authentication enabled; owner key needed" | unchanged | Root password login is still enabled. The owner's own public key is not installed. Hardening has not started |
| Installation / server qualification | BLOCKED | unchanged | The VPN is not installed; no exact artifact; BLOCKED |
| Owner-confirmed logins | "needs the owner's yes/no" | answered | see Trust status |

## Where the work is

| Item | Location |
|---|---|
| Agent revocation fix | singbox-vpn PR #186 |
| Agent/server pairing fixes | vpn-web PR #85 (stacked on #84) |
| Docs gate + stale-reference fixes | singbox-vpn PR from `docs/link-checks` |
| Device runbook + VPS handoff | singbox-vpn PR from `docs/device-runbook-and-access-handoff` |
| Registry + this record | singbox-vpn PR from `qual/2026-10-23-report` |
| #172 | still a draft; held; independently re-verified (173 tests); needs I-100/I-102 decisions and the vpn-web deploy order |

## Reports

[INTEGRATION-EVIDENCE.md](INTEGRATION-EVIDENCE.md) · [ISSUE-LEDGER.md](ISSUE-LEDGER.md) · [SECURITY-PRIVACY.md](SECURITY-PRIVACY.md) · [DOCS-AUDIT.md](DOCS-AUDIT.md) · [RELEASE-CANDIDATE.md](RELEASE-CANDIDATE.md) · [integration harness](integration/README.md) · [status.json](status.json)

## Privacy and security statements this record supports (and does not)

Supported by code/tests: credentials are encrypted at rest in the control plane (AES-GCM) and never appear in ETag inputs; a revoked node key is refused within the cache TTL;
expiry is enforced locally by the agent without the control plane; node A cannot read node B's credentials (tested).
**Not** supported and not claimed: anonymity, zero-log, leak-proof operation, DNS/IPv6 leak freedom on a real client, Google egress privacy, backup confidentiality, data retention.
