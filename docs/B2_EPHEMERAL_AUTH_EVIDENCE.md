# B2 evidence: ephemeral managed authorization on a real sing-box server

Design: vpn-web `docs/ADR/0003-ephemeral-managed-authorization.md`.
Contract: [`PROVISIONING_CONTRACT.md`](PROVISIONING_CONTRACT.md#ephemeral-managed-authorization-the-lease-pool-adr-0003).
Scripts used, verbatim: [`b2-evidence/`](b2-evidence/). No credential,
uuid, password or obfs secret appears in this document or in the scripts'
output; secrets lived only in 0600 files on the test host.

## Setup (2026-09-26, host `vps2`)

| Item | Value |
|---|---|
| OS | Ubuntu 24.04.4 LTS, x86_64 |
| sing-box | 1.14.1, downloaded from the upstream release and checked against `SINGBOX_SHA256_AMD64` in `deploy/lib/versions.env` (`sb.tgz: OK`) |
| vpn-admin / agent | this branch (`feat/b2-ephemeral-auth`), built on the host |
| Server config | rendered by `vpn-admin` from `role = "exit"`, VLESS-REALITY on TCP 443 (handshake `www.cloudflare.com`), Hysteria2 with salamander obfs on UDP 443; keys from `vpn-admin init` |
| Control plane | `b2-evidence/mockcp.py` on 127.0.0.1:8788: implements `/api/agent/leases/sync` with the same semantics as the `agent_sync_lease_slots` RPC, plus a control file to mark slots `leased`/`revoked` |
| Agent | real `vpn-provisioning-agent`, `lease_pool_size = 4`, `lease_slot_lifetime_secs = 900` (the minimum), poll 3 s |
| Client | real sing-box 1.14.1 as a client (socks inbound → vless/hysteria2 outbound) to the server's public IP, `curl https://www.gstatic.com/generate_204` through it |

Deviations from production, all test-only: the sing-box unit runs as root
without the production hardening; the Hysteria2 certificate is
self-signed, so the test client sets `insecure: true`; client and server
run on the same host (the client dials the public IP).

## Results

### 1. Slots are live before they are reported, and pass traffic (15:29)

```
15:29:04.99  agent: lease pool applied live slots=4
15:29:05.00  sync slots=4 with_secret=4       # first report, only after the live apply
15:29:08.01  sync slots=4 with_secret=0       # later syncs carry no secrets
15:29:51Z proto=vless cred=current:slot0 gen=1 result=PASS traffic curl=204
15:29:52Z proto=vless cred=current:slot1 gen=1 result=PASS traffic curl=204
15:29:54Z proto=hy2   cred=current:slot0 gen=1 result=PASS traffic curl=204
15:29:56Z proto=hy2   cred=current:slot1 gen=1 result=PASS traffic curl=204
```

Rendered inbounds: `vless 443 users=[lease-0000..lease-0003]`,
`hysteria2 443 users=[lease-0000..lease-0003] obfs=salamander`. Users are
named only `lease-NNNN`.

### 2. Revocation: old credential refused for new connections (15:30)

`b2-evidence/revoke_test.sh`: slots 0 and 1 leased, slot 0's credential
snapshotted, then slot 0 revoked at the control plane.

```
15:30:31Z revoking slot 0 at control plane
15:30:35.43  agent: lease pool: rotating slots rotated=1
15:30:36.83  agent: lease pool applied live slots=4        # 1 sing-box restart
15:30:39Z proto=vless cred=snap:slot0    gen=1 result=REFUSED curl=000 exit=35
15:30:40Z proto=hy2   cred=snap:slot0    gen=1 result=REFUSED curl=000 exit=97
15:30:42Z proto=vless cred=current:slot0 gen=2 result=PASS traffic curl=204
15:30:44Z proto=hy2   cred=current:slot0 gen=2 result=PASS traffic curl=204
15:30:45Z proto=vless cred=current:slot1 gen=1 result=PASS traffic curl=204   # leased slot untouched
state after: slot0 (gen 2, active)  slot1 (gen 1, leased)
```

Revocation → refusal: about 5 s (one sync + one apply).

### 3. What a rotation does to already-open connections (15:31)

`b2-evidence/openconn.sh`: two server-paced streams (httpbin `drip`, 40
bytes over 40 s, cannot be pre-buffered) through slots 2 and 3; then only
slot 3 is leased and revoked.

```
15:31:19  both streams start
15:31:27Z leasing+revoking slot 3 (slot 2 untouched)
15:31:37Z OPEN-CONNECTION slot3 ended after 18.1s: curl exit=18 got=18B
15:31:37Z OPEN-CONNECTION slot2 ended after 18.1s: curl exit=18 got=18B
sing-box restarts in window: 1
```

Every apply restarts sing-box (the unit has no `ExecReload`, and sing-box
1.14.1 cannot change inbound users at runtime), and the restart cuts
**every** open connection on the node, including the untouched slot 2.
So an open connection on a revoked/expired credential does not survive the
rotation, and every rotation also costs every other user on the node one
reconnect. (An earlier bulk-download probe appeared to survive a restart;
that was client-side buffering of an already-delivered body, which is why
the server-paced stream is the measurement recorded here.)

### 4. Expiry with the control plane DOWN and after an agent restart (15:32–15:45)

`b2-evidence/expiry_test.sh`. Slot 1 (gen 1) was leased; its
`valid_until` was 15:45:00Z. At 15:32:14 the mock control plane was
stopped and the agent restarted.

```
15:32:14.87  agent: provisioning agent starting          # restart, table loaded from disk
15:32:15.01  agent: lease pool applied live slots=4        # re-proven; sing-box NOT restarted (already current)
15:32:18     agent: lease pool tick failed: POST /api/agent/leases/sync request failed   (repeats; CP down)
15:44:23Z proto=vless cred=snap:slot1 gen=1 result=PASS traffic curl=204   # 37 s before expiry
15:44:25Z proto=hy2   cred=snap:slot1 gen=1 result=PASS traffic curl=204
15:45:02.30  agent: lease pool: rotating slots rotated=2  # slot 1 + unleased slot 2, same 60 s grid
15:45:02     systemd: Stopping/Started sing-box.service
15:45:03.73  agent: lease pool applied live slots=4
15:45:10Z proto=vless cred=snap:slot1 gen=1 result=REFUSED curl=000 exit=35
15:45:11Z proto=hy2   cred=snap:slot1 gen=1 result=REFUSED curl=000 exit=97
```

The expired credential was refused 2.3 s after `expires_at` (sweeper tick
granularity) with no control plane reachable, by an agent that had been
restarted mid-lease.

### 5. Recovery when the control plane returns (15:45:35)

```
15:45:36.88  sync slots=4 with_secret=2      # the two generations minted while CP was down
state: slot0 (3, active) slot1 (2, active) slot2 (2, active) slot3 (3, active)
15:45:45Z proto=vless cred=current:slot1 gen=2 result=PASS traffic curl=204
15:45:47Z proto=hy2   cred=current:slot1 gen=2 result=PASS traffic curl=204
```

Slots 0 and 3 also rotated on reconnect: the first post-outage snapshot
confirmed them `active` (never leased) after their leasable window had
closed — the "confirmed unleased" rotation rule.

### 6. Hygiene

```
secret-leak check: 13 distinct secrets checked against 673 log lines -> 0 found
   (agent + sing-box + mock journals and the mock's sync log)
700 root /var/lib/vpn-provisioning-agent
600 root /var/lib/vpn-provisioning-agent/lease-pool.json
users.json lease users: ['lease-0000', 'lease-0001', 'lease-0002', 'lease-0003']
```

### 7. Control-plane SQL on real Postgres 16 (same host)

vpn-web's migration `20260929000000_ephemeral_lease_pool.sql` applied to
PostgreSQL 16 with a minimal Supabase stand-in (auth.users, accounts,
devices, nodes, anon/authenticated roles), then
`supabase/tests/ephemeral_lease_pool_test.sql`:
`ephemeral_lease_pool_test: ok` (Privacy+ exhaustion writes nothing,
earliest-hop expiry, replay, conflict, single use, min-remaining,
revocation, monotonic generations, rate limit, expired-key reuse, pool
shrink, service-role-only grants).

Concurrency (`b2-evidence/conc.sh`, two sessions, one slot, first session
holds its transaction open 2 s): `session1: ok  session2 (concurrent):
exhausted`, 1 lease row. `SKIP LOCKED` never blocks and never double-leases.

## Not covered here

- Two-hop (relay → exit) traffic with lease credentials on both hops: the
  atomic two-hop lease is covered by the SQL and vitest suites; this run
  used a single exit node.
- A production-hardened unit (sing-box user, Let's Encrypt certificate).
