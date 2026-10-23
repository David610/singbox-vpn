# Security and privacy report — 2026-10-23

Evidence classes: **code** (read in source), **test** (an automated test that passes; red-before where stated), **real stack** (run against the disposable PostgreSQL + PostgREST + Node API + real agent
stack described in INTEGRATION-EVIDENCE.md), **UNVERIFIED** (not tested here). Nothing in this report was observed on the real VPS or a real client: both are BLOCKED.

## Trust status

| Item | Status |
|---|---|
| fin1 (62.238.46.190) trust | **Login attribution resolved by the owner (user-reported, 2026-10-23); host trust NOT established.** Original text of this row: "BLOCKED — five successful root password logins on 2026-10-22 not confirmed (I-86, #167)". Still open: root password login enabled with heavy brute-force exposure (I-39), source-network ownership unverified, logs of a possibly-compromised root are not proof |
| Any production credential rotation / host change this session | none |

## Confirmed risks found this session

| ID | Risk | Mitigation | Evidence |
|---|---|---|---|
| I-103 | `/sub/<token>` bearer token written to the app log on any handler error (vpn-web Node adapter) | log the route template | test red→green; branch (vpn-web #85) |
| I-99 | revoked (lapsed/suspended) credentials stay live on the node: **authorization bypass by omission** (exists on `main`) | singbox-vpn #186 | test red→green + real stack |
| I-98 | DB fault answered 401 ⇒ agent drops job results | vpn-web #85 | test + real stack |
| I-100 | renewed customer stays locked out (fail-closed liveness) | none — design decision | unit test |
| I-102 | one invalid credential freezes all updates for a node, including revocations | none — design decision | real stack |
| (vpn-web) | `next` ≤ 15.5.27 critical advisories incl. unauthenticated RCE in image optimization (SVG) | vpn-web #86 (15.5.28; `npm audit` 0) | audit output; exposure of the deployed site UNVERIFIED |
| I-85 | `main` had no enforced required checks | ruleset updated | verifier PASS; PR #185 BLOCKED |

## What the design provides (code/test evidence)

| Property | Evidence |
|---|---|
| sing-box does not log destinations: server log level is `fatal` with a test asserting it and that no separate log file exists | code `crates/compat-config/src/server.rs` (`log` = fatal), test |
| Subscription vhost writes no access log; `server_tokens off`; `/healthz` loopback-only; subscription service defaults to `RUST_LOG=warn`; per-unit journal rate limits | `deploy/lib/tests/test-privacy-logging-hardening.sh` (static inspection, not a live nginx) |
| Subscription tokens: 32 random bytes (base64url); stored as SHA-256 hash for lookup. **Also stored encrypted** (`subscription_token_ciphertext`, AES-GCM) so the control plane can re-display a link: anyone with the DB **and** the encryption key can recover live tokens | code (`external-credentials.js`, migration 20261017) |
| Node API keys stored only as SHA-256; revoked/quarantined nodes refused within the auth-cache TTL (default 5 s, 1 s in my test) | code + real stack |
| Agent credentials encrypted at rest (AES-GCM) and decrypted only in the API process; the ETag is computed from non-secret projection + ciphertext, never plaintext | code + real stack |
| Cross-node isolation: node B's key cannot read node A's authorizations or events | real stack (s1, s2) |
| Revocation and expiry enforced by the agent on its own clock when the control plane is unreachable (credential removed 40.3 s after the outage started) | real stack (s6) |
| Replay/downgrade of the authorization snapshot: older revision refused; schema downgrade refused; same-revision change refused unless it only revokes | test (agent) |
| Fail-closed auth: DB error ⇒ no node id returned ⇒ 5xx, never access | test + real stack |
| SSRF guard for agent probe targets exists with 13 unit tests | code/test; adversarial bypass attempts UNVERIFIED |
| Anonymous calls to every Pages Function fail closed | vpn-web test `all-routes` (1430 tests pass) |
| Secret-in-log scanner in CI for both repos | CI; blind to injected loggers (see I-103) |

## What it cannot provide, and what was not shown

- **Not anonymous.** The VPS operator (and whoever holds root on fin1) sees the user's real IP, connection times and destination IP/SNI at the moment of use; the hosting provider sees flow metadata. Logging being off reduces what is *kept*, not what is *observable*.
- **Not "zero-log".** journald records service lifecycle and warnings, `sshd` records logins (that is how I-86 was found), the control plane (Supabase/Cloudflare or the VPS-hosted API) stores accounts, devices, subscription/entitlement state, usage counters (`vpn_link_usage_daily`) and audit rows. Retention and deletion periods are **UNVERIFIED** (no policy tested; account deletion path exists as `deletion_requested_at` but its erasure of every table was not checked).
- **Not shown leak-proof.** DNS and IPv6 leakage on a real client, kill-switch behaviour, and traffic bypassing the tunnel are **UNVERIFIED** (I-30; needs a device). Hiddify discards imported route rules (CLAUDE.md), so client-side behaviour is the client's.
- **Google/YouTube traffic uses a second node's address** (the hairpin) so a third party (Google) sees that egress IP rather than the user's; the second node's operator can see those flows. No third-party relay was added.
- **Backups:** encrypted off-host backup tooling exists and passes its tests (30/30 per the pre-install checklist), but no real backup has run: confidentiality of a real backup is **UNVERIFIED**.
- **CI/CD secret handling:** workflows were not re-audited line by line this session; the required-check ruleset and the secret scanner are in place. **UNVERIFIED** beyond that.
- Command-injection and SSRF: the agent shells out to `vpn-admin` with fixed arguments and temp files created 0600 in a 0700 directory (code); no fuzzing was run this session.

## Exposed data classes

Credentials (VLESS UUID, Hysteria2 password, obfs password, node API keys, subscription tokens) · account/billing identifiers (Stripe ids, emails) · device records · usage counters · audit rows · server journals (lifecycle only) · SSH auth logs. None of these were copied into this repository; the harness uses test-only keys.
