# Evidence for the 2026-09-27 production-readiness audit

Local, reproducible checks used by
[`../../SINGBOX_VPN_PRODUCTION_READINESS_AUDIT_2026-09-27.md`](../../SINGBOX_VPN_PRODUCTION_READINESS_AUDIT_2026-09-27.md).
All of it runs on one Linux host as root, in network namespaces, with the
pinned sing-box 1.14.1 binary (`deploy/lib/versions.env`) and binaries built
from this commit. No real VPS, device or credential is involved. Generated
credentials live only in the lab state directory and are thrown away.

| File | What it is for |
|---|---|
| `netlab.sh` | Creates namespaces `inet` (router), `client`, `relay`, `exit`, `target`, plus a simulated cloud metadata address `169.254.169.254` on the router. |
| `services.sh` | Starts the target HTTP server, a TLS 1.3 REALITY decoy (`decoy.test`), a DNS server, the simulated metadata service and a loopback-only service on the exit (`127.0.0.1:8081`). |
| `subscription-flood-through-tunnel.py` | Sends HTTP requests through the client's local SOCKS inbound, over the REALITY tunnel, to the exit's `127.0.0.1:9100` subscription backend (finding SVPN-F06). |
| `connection-churn.py` | Sequential new-connection latency through a SOCKS inbound (performance section). |
| `lease-pool-cadence-sim.rs.txt` | A `#[cfg(test)]` module appended to `apps/provisioning-agent/src/lease_pool.rs` in a scratch copy; drives the real pure rotation functions for 6 simulated hours (finding SVPN-F02). |
| `timeout-does-not-kill-child.rs.txt` | Minimal tokio program with the same `timeout(.., Command::output())` shape as `dispatch.rs` (finding SVPN-F04). |

Important when re-running: `curl` honours `no_proxy`/`NO_PROXY`. In a sandbox
that lists private or link-local ranges there, `curl --socks5-hostname`
silently bypasses the tunnel for those addresses. Run lab `curl` commands
under `env -i PATH=/usr/bin:/bin` (the audit's first metadata result was
invalid for exactly this reason and was re-run).

Lab limitations: the audit host kernel had no IPv6 and no `netem`, and no
systemd as PID 1. IPv6, latency/loss, MTU, reboot and systemd restart
behaviour are therefore NOT covered by this evidence.
