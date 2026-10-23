# Google-egress hairpin: candidate target qualification package

Status 2026-10-20: **nothing here has been run against any other host.** Every step
below that touches a host other than the staging node (62.238.46.190) is listed with
its exact operation, risk and rollback so you can authorize it step by step.
Mechanism and protected invariants: `docs/YOUTUBE_FIX_INVARIANTS.md` (INV-1..13),
decision record: `YOUTUBE_EGRESS_ARCHITECTURE.md`. Do not use WARP, residential
proxies, or new third-party/VPS targets (INV-7); the only acceptable target is an
owner-approved existing node (INV-8).

## A. Authorizations required (each is separate; none is implied)
| # | Operation | Host | Risk | Rollback |
|---|---|---|---|---|
| A1 | Interactive SSH login with **read-only** commands (section B) | candidate target | none intended; a login is logged by that host | none needed |
| A2 | `ssh -N -D 127.0.0.1:1090 root@<target>` (a SOCKS tunnel, no remote command) to measure whether YouTube accepts the target's IP | candidate target | traffic of the probe exits from the target's IP; test videos only | close the tunnel |
| A3 | `vpn-admin user create --name hairpin-fin1` | target | **restarts sing-box on the target** (INV-10: drops every connection there, Hysteria2 clients can stall ~30 s). Pick a quiet window; creates one static, non-expiring user | `vpn-admin user disable`/`remove` that user (restarts again) |
| A4 | `vpn-admin google-egress-hairpin set ... --uuid-stdin` | staging exit (62.238.46.190) | all Google/YouTube traffic of every staging user now exits via the target; fail-closed by design (INV-6) | `vpn-admin google-egress-hairpin clear` (instant) |

A3/A4 move a credential between hosts; it travels only by stdin (`--uuid-stdin`), never argv,
logs, tickets or chat (INV-9). Do not give the user an expiry or a native lease (INV-10).
**Do not re-enable a provisioning agent on a node whose control plane is unreachable**
(INV-10, the 2026-10-06 `ru1` incident) — check it in B-4 before anything else.

> **CLI gotcha (I-61, fixed in the branch, present in every published release):** REALITY public keys
> are base64url and start with `-` about 1 time in 64; older `vpn-admin` rejects
> `--relay-reality-public-key -AbC...` with "unexpected argument". Always write it as
> `--relay-reality-public-key=<key>`.

## B. Read-only inspection of a candidate target (after A1)
All commands only read. Nothing restarts, renders, rotates or creates anything. Output
may contain host details but no credentials (do not paste `users.json`, keys or tokens).
1. Identity: `hostnamectl; . /etc/os-release; echo "$PRETTY_NAME"; uname -r; id -un; date -u`
2. Addresses and routes: `ip -br addr; ip -4 route; ip -6 route`; whether it has a global IPv6 (the hairpin target historically has an **IPv4-only path**, INV-5).
3. Release identity: `vpn-admin --version; python3 -c 'import json;d=json.load(open("/var/lib/singbox-vpn/install-state.json"));print({k:d.get(k) for k in ("singbox_vpn_version","role","node_id","acceptance","os_family")})'`
4. **Restart hazards (INV-10):** `systemctl is-enabled vpn-provisioning-agent 2>&1; systemctl is-active vpn-provisioning-agent 2>&1; systemctl list-timers --all | grep -E "vpn-"; systemctl show sing-box -p NRestarts -p ActiveEnterTimestamp -p MainPID`. Required: the agent is stopped/disabled while its control plane is unreachable; `NRestarts` is small and not climbing.
5. Health: `vpn-admin status; vpn-admin doctor` (read-only; expect `FAILS=0`). Do **not** add `--protocol` without A-level approval of its throwaway probe.
6. Contract/drift: `python3 -I deploy/lib/check_deployment_contract.py --contract <a-contract-for-that-node>`
7. Capacity: `uptime; free -m; df -h /; ss -s; nproc` — headroom for extra relayed QUIC sessions (measured cost +~190 ms per new QUIC session, INV-8).
8. Firewall/exposure: `ufw status verbose 2>&1 | head -30; ss -tulnp | awk '{print $1,$5}' | sort | head -40`; the REALITY port must be reachable from the exit.
9. Egress addresses: `curl -4 -s https://ifconfig.me; echo; curl -6 -s -m8 https://ifconfig.me || echo "no IPv6 egress"`.
10. Logging is privacy-preserving: `python3 -c 'import json;print(json.load(open("/etc/vpn/compat/sing-box/config.json")).get("log"))'` (expect level `fatal`).
Evidence from B is recorded as *server verified (read-only)* and attached to the qualification record; any surprise stops the process.

## C. Acceptance criteria for a target (all mandatory; none waivable by assumption)
| ID | Criterion | Method | Pass |
|---|---|---|---|
| C1 | Target IP accepted by YouTube per family the hairpin will use | A2 tunnel + `deploy/lib/youtube_evidence_probe.py --proxy socks5://127.0.0.1:1090` (domain, literal IPv4, literal IPv6 forms recorded separately) | >= 10 distinct **genuine** Shorts, 10/10 PLAY (media delivered and clock advancing), in each of 3 runs at least 1 h apart; normal videos reported separately |
| C2 | The probe can fail (negative control) | same run, same script through the staging IP | staging 0/N while target N/N; otherwise C1 is not trusted |
| C3 | Target is stable | B-4, B-7 | no restart loop, agent not auto-restarting sing-box, load headroom >= 50% |
| C4 | Rendered exit policy satisfies INV-1/4/5/6 | offline: `cargo test -p compat-config exit_hairpin`, plus `check_deployment_contract.py` routing fingerprint after A4 | tests green; fingerprint recorded in the contract |
| C5 | Fail-closed | with the hairpin set, stop the target path (simulated first, then real in a window) | YouTube fails visibly; Google set is **never** served from the staging IP; non-Google traffic unaffected |
| C6 | Through the real tunnels | REALITY and Hysteria2 clients on the exit with the hairpin on | 10/10 genuine Shorts per protocol; egress address for non-Google traffic is the exit's, for the Google set the target's |
| C7 | Restart/reinstall preservation | update, reinstall and reboot of the exit | hairpin config and routing fingerprint unchanged; contract check PASS |
| C8 | Native device (INV-12) | iPhone Hiddify, both protocols, Wi-Fi and cellular | >= 10 consecutive Shorts incl. the known-failing one, normal video with seeking, sustained session, reconnect — recorded in the release-evidence device matrix |
Until C1-C8 pass, Shorts remain `UNVERIFIED`/`BLOCKED` and no user is moved to the exit.

## D. Local, offline preparation already available (no other host needed)
* Render-level invariants: `crates/compat-config/tests/relay_role_policy.rs` (`exit_hairpin_*`, 13 invariants doc test).
* Two-hop real-sing-box system tests (loopback dummy interface, S1-S17) including fail-closed and wrong-credential cases.
* Simulated two-node hairpin test: see `deploy/lib/hairpin_simulation.py` and `deploy/lib/tests/` (run in CI; a simulation, not a two-node device or VPS result).

## E. Executable form of A1/A2/C1/C2 (prepared 2026-10-21; **not run against any host**)
`deploy/lib/hairpin_target_suitability.py` (13 hermetic tests, mutation-checked) implements exactly the read-only steps above:

    python3 deploy/lib/hairpin_target_suitability.py plan      --target root@HOST       # prints every command, executes nothing
    python3 deploy/lib/hairpin_target_suitability.py inspect   --target root@HOST --authorized-by-owner --out inspect.json   # A1 (11 fixed read-only commands)
    python3 deploy/lib/hairpin_target_suitability.py probe     --target root@HOST --authorized-by-owner --out run1.json      # A2 + same-run control
    python3 deploy/lib/hairpin_target_suitability.py aggregate run1.json run2.json run3.json                                # C1 needs 3 runs >= 1 h apart

* Refuses to touch a host without `--authorized-by-owner`; the remote commands are constants checked against a deny-list of
  mutating verbs; the tunnel step uses `ssh -N` (no remote command); strict host keys, BatchMode, no forwarding except the one SOCKS.
* Reports: egress IPv4/IPv6 as the world sees them (through the tunnel and for the control), ICMP latency, sshd posture
  (password login is flagged), firewall/listeners, restart hazards (INV-10), doctor FAIL/WARN, capacity, logging level, and per-class Shorts results.
* A target pass is only trusted if the SAME run's control (the refused staging path) fails; a normal-video pass never substitutes for Shorts;
  a single run is never "suitable". The tool explicitly lists C3-C8 as NOT covered (they stay UNVERIFIED).
* Minimum owner decision if the historical Selectel node is unsuitable: authorize ONE existing, owner-controlled node whose IPv4
  (and, only if the hairpin will use it, IPv6) passes C1/C2 - no new third-party service, WARP or residential proxy (INV-7).
