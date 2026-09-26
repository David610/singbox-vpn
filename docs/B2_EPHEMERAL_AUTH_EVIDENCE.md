# B2 evidence: ephemeral managed authorization on a real sing-box server

Design: vpn-web `docs/ADR/0003-ephemeral-managed-authorization.md`.
Contract: [`PROVISIONING_CONTRACT.md`](PROVISIONING_CONTRACT.md#ephemeral-managed-authorization-the-lease-pool-adr-0003).
Scripts used, verbatim: [`b2-evidence/`](b2-evidence/). No credential,
uuid, password or obfs secret appears in this document or in the scripts'
output; secrets lived only in 0600 files on the test host.

## Setup (2026-09-26, host `vps2`; revised run 16:49–17:35 UTC)

| Item | Value |
|---|---|
| OS | Ubuntu 24.04.4 LTS, x86_64 |
| sing-box | 1.14.1, downloaded from the upstream release and checked against `SINGBOX_SHA256_AMD64` in `deploy/lib/versions.env` (`sb.tgz: OK`) |
| vpn-admin / agent | this branch (`feat/b2-ephemeral-auth`), built on the host |
| Server config | rendered by `vpn-admin` from `role = "exit"`, VLESS-REALITY on TCP 443 (handshake `www.cloudflare.com`), Hysteria2 with salamander obfs on UDP 443; keys from `vpn-admin init` |
| Control plane | `b2-evidence/mockcp.py` on 127.0.0.1:8788: implements `/api/agent/leases/sync` with the same semantics as the `agent_sync_lease_slots` RPC (incl. `extend_to`, `urgent`, adopted `valid_until`), plus a control file to mark slots `leased` / `revoked` / `revoke_urgent` / `extend` |
| Agent | real `vpn-provisioning-agent`, `lease_pool_size = 4`, `lease_slot_lifetime_secs = 900`, `rotation_batch_interval_secs = 120` (short grid so several windows fit in a test run; default 600), poll 3 s |
| Server-paced source | `b2-evidence/dripserver.py`: 1 byte/s HTTP stream on the host (reached through the tunnel), so a cut cannot hide behind client buffering |
| Client | real sing-box 1.14.1 as a client (socks inbound → vless/hysteria2 outbound) to the server's public IP, `curl https://www.gstatic.com/generate_204` through it |

Deviations from production, all test-only: the sing-box unit runs as root
without the production hardening; the Hysteria2 certificate is
self-signed, so the test client sets `insecure: true`; client and server
run on the same host (the client dials the public IP).

## Results

This run exercises the revised design: in-place renewal, batched
rotation on the `valid_until` grid, urgent revocation. Scripts:
`renew_test.sh`, `batch_test.sh`, `urgent_test.sh`, `hup_test.sh`,
`expiry_test.sh`, `final.sh` (helpers in `lib.sh`). Restart counts are
`Started sing-box` journal lines plus the sing-box MainPID.

### 1. Slots live before they are reported, on the batch grid (16:49)

```
16:49:11.29  agent: lease pool applied live slots=4
16:49:11.30  sync slots=4 with_secret=4       # first report, only after the live apply
16:49:14.32  sync slots=4 with_secret=0
state: all 4 slots gen 1 active, valid_until 17:04:00   # floor(16:49:11 + 900 s) on the 120 s grid
16:49:30Z proto=vless cred=current:slot0 gen=1 result=PASS traffic curl=204
16:49:31Z proto=hy2   cred=current:slot0 gen=1 result=PASS traffic curl=204
```

### 2. Renewal keeps an open connection alive, zero restarts (16:50–17:08)

`renew_test.sh`: all 4 slots leased (so nothing else can rotate), slot 1
snapshotted (original `valid_until` 17:04:00). A server-paced stream
(600 B at 1 B/s) through slot 1 starts at 16:58:02. At 17:01 the control
plane renews (sets `extend_to` = 17:16:00, which is floor(now + 900 s) on
the grid).

```
17:01:00Z client renews: control plane sets extend_to=17:16:00 on all 4 leased slots
17:01:02.97  agent: lease pool: renewals adopted extended=4
17:01:06.04  agent: lease pool applied live slots=4      # store-only change: vpn-admin "already current"
state: all 4 slots gen 1 leased, valid_until 17:16:00    # same generation, same secret
--- 30 s after the ORIGINAL valid_until, same credential, new connections:
17:04:32Z proto=vless cred=snap:slot1 gen=1 result=PASS traffic curl=204
17:04:33Z proto=hy2   cred=snap:slot1 gen=1 result=PASS traffic curl=204
17:08:01Z OPEN-CONNECTION snap:slot1 (600B at 1B/s) started 16:58:02 ended after 599.3s: curl exit=0 got=200 600B -> SURVIVED
sing-box restarts since test start: 0; MainPID before=58264 after=58264
rotations logged: 0
```

### 3. Batching bounds restarts (17:08–17:16)

`batch_test.sh`: one non-urgent revocation every 30 s for 8 minutes (16
revocations) on a 120 s grid.

```
--- agent rotations:
17:08:26.43  rotating slots rotated=1   # no rotation yet in this window -> applied now
17:10:01.29  rotating slots rotated=3
17:12:00.37  rotating slots rotated=4
17:14:02.47  rotating slots rotated=4
17:16:01.48  rotating slots rotated=4   # includes the slots' own expiry at 17:16:00
--- sing-box starts per 120 s grid window:
17:08:00-17:10:00: 1
17:10:00-17:12:00: 1
17:12:00-17:14:00: 1
17:14:00-17:16:00: 1
17:16:00-17:18:00: 1
revocations=16 restarts=5 window=486s (5 grid windows touched)
```

Without batching this would have been 16 restarts. With the default
600 s window it is at most 6 per hour per node from non-urgent causes.

### 4. Honest deferral, then urgent revocation (17:18)

`urgent_test.sh`: a batch rotation at 17:18:00 uses up the window; slot 0
is then revoked non-urgently, slot 1 urgently.

```
17:18:00.58  rotating slots rotated=1                     # batch at the boundary
17:18:13Z slot 0 revoked (NON-urgent); next boundary 17:20:00
17:18:30Z proto=vless cred=snap:slot0 gen=5 result=PASS traffic   # still accepted: deferred to the batch
sing-box restarts since revoke: 0
17:18:30Z slot 1 revoked URGENT
17:18:35.17  rotating slots rotated=2                     # urgent slot 1 + pending slot 0
17:18:36.58  lease pool applied live slots=4
urgent: sing-box restarts since urgent revoke: 1 (revoke -> new generation reported: 7s)
17:18:38Z proto=vless cred=snap:slot1 gen=5 result=REFUSED curl=000 exit=35
17:18:40Z proto=hy2   cred=snap:slot1 gen=5 result=REFUSED curl=000 exit=97
17:18:42Z proto=vless cred=snap:slot0 gen=5 result=REFUSED curl=000 exit=35   # pending one rode along
17:18:43Z proto=vless cred=current:slot1 gen=6 result=PASS traffic
```

So a non-urgently revoked credential stays accepted until the next
boundary (here it would have been 17:20:00; at most one window, and never
past its `expires_at`), which is what the ADR states. An urgent one is
refused about 5–7 s later.

### 5. SIGHUP is not a cheaper apply path (17:20)

`hup_test.sh`, run between batch boundaries:

```
17:20:18Z kill -HUP sing-box (pid 78992)
17:20:18Z OPEN-CONNECTION current:slot1 (40B at 1B/s) started 17:20:07 ended after 10.5s: curl exit=18 got=200 11B -> CUT
17:20:20Z pid after HUP=78992 active=active
--- control run, no signal:
17:21:00Z OPEN-CONNECTION current:slot1 (40B at 1B/s) started 17:20:21 ended after 39.1s: curl exit=0 got=200 40B -> SURVIVED
```

sing-box 1.14.1 handles SIGHUP in process (same PID) but re-creates its
inbounds, which cuts every open connection, the same as a restart. So the
apply keeps using `systemctl reload-or-restart`. (An earlier ad-hoc run at
16:36 through httpbin `drip` gave the same result: cut at 10.5 s with
SIGHUP, full 40 B in 39.5 s without.)

### 6. Expiry with the control plane DOWN after an agent restart, ending REFUSED (17:21–17:34)

`expiry_test.sh`: slot 2 (gen 6) leased, `valid_until` = `expires_at` =
17:34:00. The mock control plane is stopped and the agent restarted.

```
17:21:15Z slot2 is LEASED; valid_until (= expires_at)=17:34:00
17:21:15Z mock control plane STOPPED
17:21:15Z agent RESTARTED
17:21:15.57  provisioning agent starting                  # table loaded from disk
17:21:15.69  lease pool applied live slots=4              # re-proven; no sing-box restart
17:21:15.70  WARN lease pool tick failed: POST /api/agent/leases/sync request failed   (repeats; CP down)
sing-box restarts caused by the agent restart: 0 (MainPID 78992 -> 78992)
--- before expiry
17:33:22Z proto=vless cred=snap:slot2 gen=6 result=PASS traffic curl=204
17:33:23Z proto=hy2   cred=snap:slot2 gen=6 result=PASS traffic curl=204
--- after expiry (control plane still down)
17:34:01.41  lease pool: rotating slots rotated=1
17:34:02.83  lease pool applied live slots=4
17:34:09Z proto=vless cred=snap:slot2 gen=6 result=REFUSED/no traffic curl=000 exit=35
17:34:11Z proto=hy2   cred=snap:slot2 gen=6 result=REFUSED/no traffic curl=000 exit=97
17:34:13Z proto=vless cred=current:slot2 gen=6 result=REFUSED/no traffic curl=000 exit=35
sing-box restarts since agent restart: 2    # 17:32:00 (slots 0,1,3 expired) and 17:34:00 (slot 2)
DONE
```

The expired credential is refused from 2.8 s after `expires_at` (next
poll + apply) with no control plane reachable, by an agent restarted
mid-lease. Expiry was not deferred by batching: `valid_until` is on the
grid. (`current:slot2` is also gen 6 because the mock never received
gen 7 while it was down.)

### 7. Recovery when the control plane returns (17:34:35)

```
17:34:35.98  sync slots=4 with_secret=4      # generations minted while CP was down
state: {'0': (7, 'active'), '1': (7, 'active'), '2': (7, 'active'), '3': (7, 'active')}
17:34:44Z proto=vless cred=current:slot1 gen=7 result=PASS traffic curl=204
17:34:45Z proto=hy2   cred=current:slot1 gen=7 result=PASS traffic curl=204
```

### 8. Hygiene

```
secret-leak check: 15 distinct secrets checked against 2025 log lines -> 0 found
   (agent, sing-box, mock and every test unit's journal, the mock's sync log, all *.out files)
700 root /var/lib/vpn-provisioning-agent
600 root /var/lib/vpn-provisioning-agent/lease-pool.json
users.json lease users: ['lease-0000', 'lease-0001', 'lease-0002', 'lease-0003']
```

### 9. Control-plane SQL on real Postgres 16 (same host)

The revised migration `20260929000000_ephemeral_lease_pool.sql` applied
to PostgreSQL 16 with the minimal Supabase stand-in, then
`supabase/tests/ephemeral_lease_pool_test.sql`:
`ephemeral_lease_pool_test: ok`. It covers the original cases plus:
renewal returns the same lease, slots and credentials with `expires_at`
floored to the node grid, writes `extend_to`, creates no lease row and
is not rate limited; a renewal is replayable by its key; no renewal
inside the 60 s lead; urgent revocation reported to the node; a revoked
lease is never renewed; rotation resets `extend_to`/`urgent`; adopted
`valid_until` recorded; policy table service-role only.
`conc.sh` (two sessions, one slot): `session1: ok  session2
(concurrent): exhausted`, 1 lease row.

## Summary of measured restart counts

| Scenario | Events | sing-box restarts |
|---|---|---|
| Renewal across original expiry, 10-min open stream | 4 renewals | **0** (PID unchanged, stream survived) |
| Non-urgent revocations, 8 min, 120 s grid | 16 | **5** (1 per window) |
| Urgent revocation (+1 pending non-urgent) | 2 | 1, refused ≈5–7 s after revoke |
| Agent restart with control plane down | 1 | 0 |
| Expiry, control plane down | 4 slots on 2 grid points | 2 |

## Not covered here

- Two-hop (relay → exit) traffic with lease credentials on both hops: the
  atomic two-hop lease and renewal are covered by the SQL and vitest
  suites; this run used a single exit node.
- A production-hardened unit (sing-box user, Let's Encrypt certificate).
- The previous run (15:29–15:45, 60 s grid, no renewal) is superseded by
  this one; its results on refusal after rotation and connection cuts on
  restart were reproduced here.
