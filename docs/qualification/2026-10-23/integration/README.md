# Paired control-plane integration harness (2026-10-23)

Local, test-credentials only. No Cloudflare, Supabase cloud or real agent was touched. **Correction: it ran on the intended VPS itself (see ../README.md).**

Stack: PostgreSQL 16 (58 vpn-web migrations from `scripts/sql/supabase-bootstrap.sql` + migrations + `seed.sql`) → PostgREST 16.5 →
a path-stripping proxy (`/rest/v1`, as Supabase's gateway does) → vpn-web's real Node API (`server/index.mjs`, PR #84 + #85) → the real
`vpn-provisioning-agent` binary with a recording stand-in for `vpn-admin` (`stub-vpn-admin`, which only logs and copies its input).

| File | What it does |
|---|---|
| `lib.mjs` | starts API instances, seeds nodes/accounts/subscriptions/devices through vpn-web's own `create_external_vpn_device` RPC with real AES-GCM credentials |
| `s1.mjs` | authorizations: auth, ETag/304 on two instances, stale ETag, revocation, billing lapse, isolation, bad schema |
| `s2.mjs`, `s2b.mjs` | event channel: resync, timeout, bad cursors, job/revocation/billing wake-ups on two instances, missed-event catch-up, superseded 409, node isolation, revoked-key rejection; `s2b` is the stale-cache reproduction |
| `s3.mjs`, `s3b.mjs` | database outage: status codes and recovery |
| `s4.mjs <agent> <server>` | the real agent against a real server: first apply, 30 s idle request tally, revocation latency, 25 s DB outage tally, post-outage revocation |
| `s5.mjs` | which revocation paths advance `snapshot_revision` |
| `runmatrix*.sh` | runs the agent × server matrix |
| `results/` | raw JSON of each matrix cell |

Agent kinds: `old` = `main` (548a376); `new` = draft #172 (`claude/singbox-vpn-issue-resolution-b57cl0`); `fix` = `main` + PR #186; `new` run with `/root/tgt-new` after merging #186 into #172 is `results/newfix-new.json`.
Server kinds: `new` = vpn-web PR #84 + #85; `old` = vpn-web `main` handlers served through the same Node adapter (the legacy deployment is Cloudflare Pages Functions, which cannot run here, so **"old server" is an emulation of its handlers**, not Pages).

Paths are absolute to the audit host (`/root/vpn-web`, `/root/tgt-*`); adapt before reuse. `pgrst.conf` carries a placeholder secret; the
harness expects `service.jwt` signed with the secret you configure.
