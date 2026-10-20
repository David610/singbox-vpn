# Acceptance thresholds, sample sizes and observation windows

Proposed 2026-10-20. Values marked **(calibrate)** are provisional until a
baseline run on the target hardware exists; everything else is a hard gate.
"PASS" always means measured, never assumed. Not run = `UNVERIFIED`.

## Evidence classes (never merged in one verdict)
1. **Unit / integration (CI)** — deterministic, offline, loopback sing-box.
   Proves rendering, parsing, doctor logic, scripts, exit codes.
2. **Staging VPS** — real host, real certificates, loopback client on the node
   (`subscription_acceptance.py`, `doctor --protocol`). Proves install,
   lifecycle, credentials, data path, egress. Not an external-network result.
3. **External network** — a client outside the node's provider network.
4. **Native device** — physical iPhone/Android, real app. The only class that
   can verify client behavior and Shorts (INV-12).

## Hard gates (zero tolerance)
| Gate | Sample / window | Pass rule |
|---|---|---|
| Release evidence | the release commit | `release_evidence.py --require PRODUCTION_READY` |
| CI | the release commit | fmt, clippy, workspace tests, shell tests, audit all green; none skipped |
| Subscription formats | 6 URLs x 1 run, TLS verified | 100% of `subscription_acceptance.py` checks pass |
| Real-client data path | each protocol x each served format | egress address equals the node's declared egress |
| Invalid / revoked token | 3 variants | 404 every time |
| Certificate | at test time | SAN covers every declared host; >= 14 days left; `certbot renew --dry-run` PASS |
| Security | at test time | no password SSH on a production node; firewall = intended ports only; no secret in logs/artifacts |
| Routing invariants | every build | INV tests green; fingerprint of rendered routing equals the contract |
| Shorts, accepted egress | >= 10 distinct true Shorts, 3 runs >= 1 h apart, per address family in use | 10/10 each run (refusal is per-IP and effectively binary; a single miss is investigated, not averaged away) |
| Doctor | after each lifecycle stage | exit 0, no `[FAIL]`; deliberate unintended exposure makes it exit nonzero |

## Reliability / performance (calibrate)
| Metric | Method | Threshold |
|---|---|---|
| Availability, REALITY and Hysteria2 | synthetic handshake + bytes each minute, 7 days staging | >= 99.5%, no outage > 5 min unexplained |
| Restart recovery | `systemctl restart sing-box` x 10 | REALITY clients resume <= 10 s; Hysteria2 <= 35 s (1.14.1 stall, I-22); 10/10 |
| Soak | 24 h, 1 client per protocol, probe every 60 s | >= 99% probes OK, 0 unexplained restarts |
| Throughput | `vpn-benchmark.sh`, 5 runs median | >= 70% of direct line rate **(calibrate)** |
| Added latency | median RTT via tunnel minus direct, 100 samples | <= 40 ms same-region **(calibrate)** |
| Packet loss through tunnel | 1000 ICMP/UDP echoes | <= 1% |
| Multiple clients | 5 concurrent clients, 30 min | no auth failures, no restarts |
| Rollback | injected failed update x 3 | previous release restored, doctor clean, 3/3 |

## Native device matrix (each cell recorded in `release_evidence.py` format)
Transports `reality`, `hysteria2` x networks `wifi`, `cellular` = 4 mandatory
cells, each with: device model + OS, app + core version, date, release tag and
commit, **>= 10 consecutive Shorts**, the known-failing Short, a normal video
with seeking, a sustained session >= 30 min, reconnect, lock/unlock,
Wi-Fi<->cellular switch, subscription refresh, app relaunch, and DNS/IPv6
leak observations. A cell with any item not recorded is not PASS.

## Monitoring (production, after acceptance)
| Signal | Probe | Alert |
|---|---|---|
| REALITY / Hysteria2 reachability | every 1 min, loopback + one external vantage | 3 consecutive failures = page |
| Subscription HTTPS + TLS | every 5 min, verify host + expiry | failure x2 = page; expiry < 14 d = warn, < 7 d = page |
| Shorts health on the **actual exit path** | every 30 min, 3 true Shorts, aggregate counts only | 2 consecutive < 3/3 = warn, 4 = page |
| Restart frequency | journald count | > 3 in 10 min = page (start-limit risk, I-11) |
| Config drift | rendered-routing fingerprint vs contract, hourly | any difference = page, never auto-fix |
| Resources | CPU/RAM/disk/conntrack | > 85% sustained 10 min = warn |
| Security-relevant change | sshd effective config, ufw rules, authorized_keys hash | any change = notify |
Telemetry is aggregate only; subscription tokens and destinations are never logged.
