# Protocol-level health probes: real-infrastructure evidence

Date: 2026-09-26. Branch `feat/protocol-health` (singbox-vpn) together with
vpn-web `feat/protocol-health`.

## What was tested

Two real VPSs on different providers, countries and ASNs:

| node id    | host                            | OS           | sing-box |
|------------|---------------------------------|--------------|----------|
| `de-ber-1` | vps1 157.173.27.46 (Berlin)     | AlmaLinux 9  | 1.13.19  |
| `fi-hel-1` | vps2 62.238.46.190 (Helsinki)   | Ubuntu 24.04 | 1.14.1   |

Each node ran:

* a standalone singbox-vpn server rendered by this branch's `vpn-admin`
  (`init` + `render-config`, `state_dir=/root/ph/compat`, REALITY on
  2443/tcp, Hysteria2 with salamander obfs on 2443/udp, unit `ph-singbox`).
  Other services on the hosts (including the B2 evidence stack on vps2
  port 443) were not touched;
* this branch's `vpn-provisioning-agent`, a static musl build made on vps2,
  with `[protocol_probe]` enabled, `heartbeat_interval_secs = 20` and
  `interval_secs = 20`.

The control plane was `docs/health-probe-evidence/mockcp.mjs`, reached over
SSH reverse tunnels. It does not reimplement the health rules. It imports
vpn-web's own `functions/lib/protocol-health.js`,
`protocol-health-store.js`, `node-health-transition.js` and
`node-lifecycle.js`, and runs them against an in-memory store shaped like the
Supabase client. It mirrors only the heartbeat handler's glue: apply the
protocol report first, then run the node's own `evaluateProbeResult` with the
recovery gate, then run silence detection. `FEATURE_AUTO_NODE_HEALTH` was on.
The raw event log is in `docs/health-probe-evidence/events.log`. The setup
script is `docs/health-probe-evidence/setup-node.sh`.

Hysteria2 used self-signed certificates, so the probe client ran with
`tls_insecure_for_tests = true`. Production certificates verify, and the
flag defaults to false.

## Probe credential provisioning (as exercised)

1. On startup, each agent ran `vpn-admin user create --name arcana-probe
   --json`, or reused an existing user with that name. It then read the share
   links with `vpn-admin user links <id>`.
2. The agent posted the links to `POST /api/agent/probe-credential`. The log
   shows `CREDENTIAL published by de-ber-1 (reality=true hysteria2=true)`.
3. Each agent polled `GET /api/agent/probe-targets` and received the peer's
   links plus that peer's expected public IPv4.

The probe user is not a customer and has no subscription, device or traffic
record. The URIs never appear in logs. `grep -ciE
"vless://|hysteria2://|password|uuid"` over both agent journals and the mock
CP logs returned 0. The temporary client config that holds the credential
lives in a `.vpn-probe-*` directory. Such a directory is removed on drop, and
also on the next start if a hard kill left it behind. That sweep was added
after this run found one orphaned directory from a `systemctl stop` that
landed mid-probe.

## Probe user egress confinement (fix: probe credential is not an open proxy)

**Problem.** Before this fix `arcana-probe` was an ordinary user with
unlimited egress. Its share links go to up to 3 peer agents, so anyone who
held one (a compromised peer, or a leaked link) had a free, unaccounted
proxy.

**Fix.** `compat_config::server::apply_probe_user_confinement` runs at the
end of `render_server_config_for_deployment` for both roles. When an active
user named `compat_config::model::PROBE_USER_NAME` (`arcana-probe`) exists,
it prepends three `route.rules` entries keyed on that user's rendered id.
They sit ahead of every other rule: the exit hairpin rules, the relay
forwarding rules and the relay's final reject.

1. `auth_user` + `network: tcp` + `port: 443` + `ip_cidr: ["1.1.1.1/32"]`
   route to `direct`
2. `auth_user` + `network: tcp` + `port: 443` +
   `domain: ["www.gstatic.com", "ipv6.icanhazip.com"]` (exact hosts, not
   suffixes) route to `direct`
3. `auth_user` → `reject`, which covers everything else, UDP included

`auth_user` is the sing-box 1.14.1 route rule field that matches
`metadata.User` (`route/rule/rule_item_auth_user.go`). The VLESS and
Hysteria2 inbounds set that field from the user's `name`, which this renderer
fills with the user id. The allowlist is built from the same constants the
prober fetches: `PROBE_TRACE_V4_URL`, `PROBE_DNS_URL`, `PROBE_V6_ONLY_URL`,
`PROBE_ALLOWED_*` in `crates/compat-config/src/model.rs`.
`protocol_probe.rs` imports those constants, and a unit test fails if a URL
host and the allowlist drift apart in either direction. The agent now
refuses to publish a probe credential when `probe_user_name` is not the
reserved name, because only that name is confined. `vpn-admin doctor`
ignores the three leading probe rules when it checks the exit or relay
policy shape. Users without that name get no new rules, and without a probe
user the rendered document is byte-identical to before. Tests:
`exit_confines_probe_user_and_leaves_customers_unrouted`,
`probe_rules_precede_exit_hairpin_rules`,
`probe_rules_precede_relay_policy_which_is_otherwise_unchanged`,
`disabled_probe_user_gets_no_rules`,
`probe_urls_are_exactly_covered_by_the_probe_allowlist`.

**Rate and bandwidth.** sing-box 1.14.1 has no per-user connection-rate or
bandwidth limit in route rules. Hysteria2 `up_mbps`/`down_mbps` apply to the
whole inbound, and there is no per-user equivalent for VLESS. So no limit
is rendered. The residual exposure is bounded instead. A holder of the link
can only fetch three fixed, tiny HTTPS endpoints (a trace page, a 204 and an
IP echo) on third-party CDNs. That gives no general egress and no way to
choose a destination. It also cannot be used for amplification toward
arbitrary targets. Per-user throttling would need a sing-box feature that
does not exist in the pinned version.

**Real test (vps2, sing-box 1.14.1, isolated rig in `/root/pc`, port
2543 tcp+udp, unit `pc-singbox`; no existing service was touched).** A
debug `vpn-admin` built from this branch ran `init`, then `user create
--name arcana-probe`, then `user create --name customer1`, then
`render-config`. `sing-box check` passed on the rendered config. The route
contained exactly the three rules above for the probe user's id, and
nothing for `customer1`. Two local sing-box clients were built from each
user's own share links, one over VLESS+REALITY and one over Hysteria2. curl
ran through each client's SOCKS inbound (`socks5h` for hostnames), and each
row shows the HTTP status or the curl failure:

| URL | probe / REALITY | probe / Hysteria2 | customer / REALITY | customer / Hysteria2 |
|---|---|---|---|---|
| `https://1.1.1.1/cdn-cgi/trace` | 200 | 200 | 200 | 200 |
| `https://www.gstatic.com/generate_204` | 204 | 204 | 204 | 204 |
| `https://ipv6.icanhazip.com` | 200 | 200 | 200 | 200 |
| `https://example.com` | refused (curl 35) | refused (curl 35) | 200 | 200 |
| `https://8.8.8.8` (raw IP not listed) | refused (curl 35) | refused (curl 35) | 302 | 302 |
| `http://1.1.1.1/` (allowed IP, port 80) | refused (curl 52) | refused (curl 52) | 301 | 301 |
| `https://gstatic.com/` (suffix, not exact host) | refused (curl 35) | refused (curl 35) | 204 | 204 |

The rig was stopped and its unit file removed afterwards.

## Scenario results

Thresholds come from `node-health-transition.js`: 3 consecutive failures take
READY to DEGRADED, and 5 consecutive passes take DEGRADED back to READY.

### 0. Baseline: every dimension passes

From `de-ber-1`, probing its peer `fi-hel-1` (18:06:04Z):

```
fi-hel-1/peer/reality=ok   tcp=true dns=true v4=true v6=egress(2a01:4f9:c014:13bd::1) egress=62.238.46.190 match=true lat=33 loss=0
fi-hel-1/peer/hysteria2=ok tcp=null dns=true v4=true v6=egress(2a01:4f9:c014:13bd::1) egress=62.238.46.190 match=true lat=33 loss=0
de-ber-1/self/reality=ok   tcp=true dns=true v4=true v6=egress(2a12:bec4:19a5:924::) egress=157.173.27.46 match=null lat=5 loss=0
```

What this shows:

* The REALITY TCP listener was reachable.
* Real REALITY and Hysteria2 handshakes completed.
* IPv4 HTTPS worked through the tunnel (`https://1.1.1.1/cdn-cgi/trace`).
* DNS resolved on the server side (socks5h to `www.gstatic.com`).
* The observed egress IPv4 equals the target node's IP (`match=true`).
* IPv6 egress works, and the egress IPv6 is the target's own address, so
  nothing leaks via the prober.
* Latency is the median of 3 samples, and loss is measured over those same
  3 samples.
* The Hysteria2 certificate expiry was reported as `cert_days=29`.

### 1. sing-box stopped on the peer: handshakes fail, DEGRADED after hysteresis, recovery

At 18:06:47Z, `systemctl stop ph-singbox` ran on fi-hel-1. The next peer
reports from de-ber-1 were:

```
18:07:05 fi-hel-1/peer/reality=ok | hysteria2=FAIL(handshake_failed)       (1)
18:07:25 reality=FAIL(tcp_unreachable) | hysteria2=FAIL(handshake_failed)  (2)
18:08:05 reality=FAIL(tcp_unreachable) | hysteria2=FAIL(handshake_failed)  (3)
18:08:05 TRANSITION fi-hel-1 READY -> DEGRADED [protocol-health-store CAS]
```

At 18:08:15Z, sing-box was started again. There were 5 passing peer reports
between 18:08:45 and 18:10:06, and then
`TRANSITION fi-hel-1 DEGRADED -> READY` at 18:10:06.

fi-hel-1's own self (loopback) results were failing at the same time. They
were ignored because a fresh peer view existed.

### 2. UDP 443-equivalent (2443/udp) blocked: Hysteria2 fails, REALITY passes

At 18:10:28Z, `iptables -I INPUT -p udp --dport 2443 -j DROP` ran on
fi-hel-1:

```
18:11:26 fi-hel-1/peer/reality=ok | fi-hel-1/peer/hysteria2=FAIL(handshake_failed)
18:12:06 reality=ok | hysteria2=FAIL(handshake_failed)
18:13:06 reality=ok | hysteria2=FAIL(handshake_failed)
18:13:06 TRANSITION fi-hel-1 READY -> DEGRADED
```

The rule was removed at 18:13:09Z. After 5 passing reports came
`DEGRADED -> READY` at 18:14:46.

**2c. Loopback blind spot.** This scenario shows why peer results take
precedence. The same drop rule was repeated but limited to `-i eth0`
(18:25:19Z). fi-hel-1's own loopback probe kept reporting
`fi-hel-1/self/hysteria2=ok` on every round, while the peer saw
`hysteria2=FAIL`. The node still went `READY -> DEGRADED` at 18:27:57 on the
peer's evidence, and returned `DEGRADED -> READY` at 18:29:37 after the rule
was removed. A self-only design would have missed this outage. In scenario
2, the rule had no interface qualifier, so it also dropped loopback traffic
and the self probe failed too.

### 3. DNS broken on the target server

At 18:15:03Z, outbound udp/53 and tcp/53 were rejected on de-ber-1. The
resolver on that host is 1.1.1.1 directly. fi-hel-1's peer reports:

```
18:15:54 de-ber-1/peer/reality=FAIL(handshake_failed) tcp=true dns=false v4=false v6=unknown
         de-ber-1/peer/hysteria2=ok                   dns=false v4=true v6=unknown egress=157.173.27.46
```

* The **DNS dimension failed** as expected, while Hysteria2 IPv4 egress
  (IP literal) kept working. IPv6 is reported as `unknown`, not `blocked`,
  because its probe needs DNS.
* **Finding:** REALITY itself failed. A REALITY server must resolve its
  handshake decoy (`www.cloudflare.com`), so on a real node a broken server
  resolver breaks REALITY for users. The REALITY useful-egress signal
  catches this without DNS needing to be a lifecycle driver of its own. The
  rules were removed at 18:16:19Z, after 2 failing reports, which was below
  the degrade threshold. The next reports passed.

### 4. Recovery only for recoverable reasons

* **4a. CANARY_ABORT.** At 18:17:11Z, fi-hel-1 was set to FAILED with
  `failed_reason=CANARY_ABORT` while everything was healthy. Over the
  following 2.5 minutes there were 7 heartbeats from fi-hel-1, each carrying
  all-passing probe results, and it **stayed FAILED/CANARY_ABORT**. Final
  state: `fi-hel-1 FAILED CANARY_ABORT protocol_probe_failures=0
  successes=11`. Protocol evidence never takes a node out of FAILED.
  Peers also stop probing FAILED nodes, because `probe-targets` serves only
  WARMING_UP, READY and DEGRADED nodes.
* **4b. Silence.** An admin restored fi-hel-1 to READY. Then its agent and
  sing-box were stopped at 18:19:47Z. Peer probes produced `READY ->
  DEGRADED` at 18:21:07. The silence rule (3 x 60 s without a heartbeat)
  produced `DEGRADED -> FAILED (failed_reason=SILENCE)` at 18:22:47. FAILED
  was reached only through silence.
* **4c.** sing-box and the agent were restarted at 18:22:48Z. On the first
  heartbeat came `FAILED -> READY [own heartbeat (evaluateProbeResult)]` at
  18:22:50. SILENCE is the recoverable reason, and this recovery went
  through the existing null-probe rule, since no Clash API was configured.

## Retention

The mock ran the same pruning contract as `prune_node_probe_results`: 24 h
retention and at most 5000 rows per target. After about 25 minutes it held
2567 per-dimension rows for 2 nodes. At the production 60 s cadence, one peer
per target plus the self report stays within the cap.

## Not covered here

* The Worker endpoints against a real Supabase instance. They are covered by
  vitest with mocked clients, and the migration was not applied to a live
  project in this run.
* The Clash-API probe (`probe_ok`). It was not configured on these test
  servers, so the READY gating that combines it with protocol evidence is
  covered by unit tests only.
* IP reputation. It is a separate, informational `nodes.ip_reputation`
  column that nothing writes automatically yet, and it never drives
  lifecycle.
