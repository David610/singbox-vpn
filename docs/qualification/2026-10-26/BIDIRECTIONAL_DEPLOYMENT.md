# Bidirectional deployment record, 2026-10-26 (v1.4.0-rc.1 on two historical hosts)

Evidence level: LIVE HOSTS + a real Hiddify core (v5.0.3, Linux CLI) on a workstation. This is **not** native-device, not
Russian-network and not AlmaLinux 9 evidence. No secret appears here.

## Topology and versions

| | RU entry/exit | EU entry/exit |
|---|---|---|
| Address | `135.106.178.167` | `91.244.71.165` |
| OS | Ubuntu 24.04 (best-effort tier) | Ubuntu 24.04 (best-effort tier) |
| Node id / role | `ru1` / relay + exit principals | `91-244-71-165` / relay + exit principals |
| Release | `v1.4.0-rc.1` (updated in place from `v1.3.1-rc.6`) | `v1.4.0-rc.1` (reinstalled; was exit `v1.3.1-rc.6`) |
| sing-box | 1.14.1 | 1.14.1 |
| Resources | 1 vCPU, 961 MB RAM (sing-box about 58 MB) | 2 vCPU, 3.9 GB RAM |

Profile A: client -> RU -> EU -> Internet (egress `91.244.71.165`). Profile B: client -> EU -> RU -> Internet (egress
`135.106.178.167`). Germany/Finland staging was not touched.

## Operations performed (actual results)

| Step | Result |
|---|---|
| Fail-closed pre-gate (release, role, node id, backup digest, disk, memory, services, doctor, connections) | The first run stopped on 73 established connections (an unidentified client on the relay and the EU host's own traffic). The owner explicitly accepted the interruption. |
| ru1 `update.sh --version v1.4.0-rc.1` | Exit status 0, release `v1.4.0-rc.1`, services active, `doctor` 0 FAIL, `doctor --protocol` 0 FAIL |
| ru1 `[exit_principals]` enabled | `render-config` ok, relay policy fail-closed, users and TLS-spoof drop-in preserved |
| EU `uninstall.sh --yes` + pinned installer (`--role relay --node-id 91-244-71-165`) | Exit status 0, under one minute; certificate issued (89 days), ufw rules and SSH preserved, `doctor` 0 FAIL, protocol ok |
| EU `[exit_principals]` enabled | ok |
| Pairing A (principal issued by EU, accepted on RU) and B (issued by RU, accepted on EU) | Both pipes exit 0; no credential printed |
| ru1 stale declarations of the old EU exit | `pair accept-exit` refuses to replace a declaration whose keys changed. A first manual removal deleted the sections after the stale blocks (my regex); the file was restored from the pre-change copy and redone precisely. Net effect: only the two stale blocks and the pilot's stale credential were removed. |
| EU leftover `RUST_LOG=debug` drop-in | Removed (backed up); `doctor` warning cleared |
| Stale `evolus-hairpin` user on ru1 | Removed. `fin1-hairpin` left in place (its owner is the staging host, not inspected) |

## Acceptance (live, real Hiddify core; headless browser for Shorts)

| Test | Profile A | Profile B | Evidence | Result |
|---|---|---|---|---|
| Pinned profile has one chained exit, nothing selectable bypasses the entry | yes | yes | `hiddify_core_acceptance.py --mode relay` | PASS |
| Egress IP | `91.244.71.165` | `135.106.178.167` | same | PASS |
| Entry credential alone cannot reach the Internet | rejected | rejected | entry-only client, 3 targets each | PASS |
| Entry host down / exit host down | egress none | egress none | stopped each service | PASS (fail-closed) |
| Independent revocation | user disabled: link 404, B unaffected | n/a | live | PASS |
| Revoke only the exit principal | egress none, B unaffected | n/a | live | PASS |
| Reboot both hosts | services up, doctor 0 FAIL, both profiles pass | same | live reboot | PASS |
| `doctor --protocol` | 0 FAIL | 0 FAIL | both hosts | PASS |
| DNS | probe name seen only at the exit (0 at entry) | same | tcpdump port 53 on both | PASS |
| IPv6 | no leak: IPv6 request returned nothing; dual-stack request used IPv4 | same | core + curl | PASS (no IPv6 path provided) |
| Load, 6 parallel streams 40 s | 4 streams about 17-22 Mbit each (2 rate-limited by the test server) | 4 streams 19-25 Mbit each | host sampling | PASS: no errors; ru1 sing-box peak 13% CPU. Profile A CPU samples missed the load window |
| Native Hiddify (Android/iPhone), Russian mobile/Wi-Fi | not performed | not performed | requires the owner's devices | UNVERIFIED |
| Network handover, app restart, subscription refresh on device | not performed | not performed | same | UNVERIFIED |
| YouTube Shorts, synthetic headless (signed-out), 35 s each | 0/6, all "unavailable" | 15/16 played to 35 s (one early stall in the first run; 10/10 in the second) | `shorts_sustained.py` | A: FAIL. B: promising, NOT release evidence |
| YouTube signed-in, 900 s continuous, long-form, both transports, devices | not performed | not performed | required by `docs/RELEASE.md` | UNVERIFIED |

## Findings

1. The EU exit's address is refused for Shorts by YouTube (0/6), as recorded before. A user who needs Shorts should use Profile B.
2. Profile B passes the synthetic Shorts probe through the Russian exit; this is consistent with the exit-principal rules
   applying the same Google egress policy (including the TLS spoof) as the exit role. It is one headless test, not device evidence.
3. On a relay `user create` already prints the pinned query; adding it again yields HTTP 400 (runbook corrected).
4. `pair accept-exit` has no supported way to forget a rebuilt exit; the migration needs one hand edit of stale
   `[[peer_endpoints]]` blocks (documented in the runbook). Candidate improvement: a `pair forget-exit` command.
5. Plain `?format=hiddify` links stop working on a relay; clients must import the pinned URL.

## Rollback artifacts (no secrets)

Both hosts: `/root/pre-bidir/` (0700) holds the `vpn-admin backup` archive, a state archive and a manifest taken before
changes, plus the pre-change copies of `deployment.toml`. Encrypted off-host copies and the key are held by the owner.
Restoring the EU exit archive requires reinstalling `--role exit --node-id 91-244-71-165` first.
