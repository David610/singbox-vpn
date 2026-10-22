# Google-egress acceptance: ordered run sheet (prepared 2026-10-21; NOTHING here has been run against any other host)

Why: the staging exit's own IP is refused by YouTube for Shorts (0/5 `REFUSED`, repeatedly, path controls OK). That is decided by the
address, so only a different Google-facing egress can fix it. Mechanism and protected rules: `docs/YOUTUBE_FIX_INVARIANTS.md` (INV-1..13) and
`docs/operations/HAIRPIN_TARGET_QUALIFICATION.md` (acceptance C1-C8). This sheet only orders them and names the evidence. The historical
Selectel node is a **candidate, not a solution**: nothing about its past behaviour is assumed.

Authorization needed from the owner, per step (none is implied by another): see `docs/handoffs/2026-10-21/OWNER-ACTIONS.md` item 3.

## Phase 0 - offline, already possible (no other host)
| Step | Command (on staging) | Expected |
|---|---|---|
| 0.1 | `python3 -I deploy/lib/hairpin_target_suitability.py plan --target root@TARGET` | prints the 11 read-only commands + the SOCKS-only probe; executes nothing |
| 0.2 | `cargo test -p compat-config exit_hairpin` and `youtube_invariants_doc` | green (the invariants are enforced by tests, not by this sheet) |
| 0.3 | `sudo python3 -I deploy/lib/hairpin_simulation.py --vpn-admin /usr/local/bin/vpn-admin --sing-box /usr/local/bin/sing-box --out /tmp/sim.json` (local netns, needs root) | 22/22 and both mutations still fail: routing and fail-closed, SIMULATED only |

## Phase 1 - read-only qualification of ONE authorized target (owner approves A1, A2)
| Step | Command | Output kept as | Computed verdict |
|---|---|---|---|
| 1.1 (A1) | `python3 -I deploy/lib/hairpin_target_suitability.py inspect --target root@TARGET --authorized-by-owner --out evidence/egress/inspect.json` | `inspect.json` | `UNSUITABLE` / `INCONCLUSIVE` / `INSPECT_OK`; **stop on UNSUITABLE** (restart hazards INV-10, password-only ssh flagged, no IPv4 path, etc.) |
| 1.2 (A2) | `... probe --target root@TARGET --authorized-by-owner --out evidence/egress/run1.json` | `run1.json` | per-run `RUN_PASS` only if the target plays >= 10 distinct genuine Shorts 10/10 **and** the same-run control (the refused staging path) fails (C2) |
| 1.3 | repeat 1.2 twice more, each >= 1 h after the previous | `run2.json`, `run3.json` | |
| 1.4 | `... aggregate run1.json run2.json run3.json` | `aggregate.json` | C1 is met only by three `RUN_PASS` runs >= 1 h apart. A single run is never "suitable" |

If 1.1-1.4 fail, the target is rejected and **nothing changes on staging**. A replacement must be another existing, owner-controlled node
(INV-7/8: no WARP, no residential proxy, no new third-party VPS): public IPv4 YouTube accepts (and IPv6 only if the hairpin will use that
family), key-only ssh, no provisioning agent that can restart sing-box unattended (INV-10), `log.level=fatal`, headroom for relayed QUIC.

## Phase 2 - only after Phase 1 passes (owner approves A3, A4, each separately)
| Step | Action | Notes |
|---|---|---|
| 2.1 (A3) | on the target: `vpn-admin user create --name hairpin-fin1` | **restarts sing-box there** (INV-10); pick a quiet window; no expiry, no lease |
| 2.2 (A4) | on staging: `printf %s "$UUID" | vpn-admin google-egress-hairpin set --relay-host H --relay-port P --relay-server-name SNI --relay-reality-public-key=<key> --relay-reality-short-id <sid> --uuid-stdin` (write `--relay-reality-public-key=<key>` with the `=`: keys can start with `-`; read the UUID from a file, never type it into a shared terminal) | credential travels by stdin only (INV-9); all Google/YouTube traffic of every staging user now exits via the target; **fail-closed** (INV-6) |
| 2.3 | `python3 -I deploy/lib/check_deployment_contract.py ...` | records the routing fingerprint (C4) |
| 2.4 | stop the target path (simulated first, then once in a window) | C5: YouTube fails visibly, the Google set is **never** served from the staging IP, other traffic unaffected |

## Phase 3 - real-path acceptance (this is what flips the gates)
1. **Monitor the real path, not the staging IP.** `exit-direct` measures the staging address directly. Add a label that goes through a
   throwaway tunnel client (what a user's traffic really takes): put `PROXY_exit_hairpin=socks5://127.0.0.1:<port>` in
   `/etc/vpn/shorts-health.env` for a throwaway sing-box client pointed at the staging REALITY listener, then
   `install-shorts-monitor.sh --label exit-hairpin` and `--check --label exit-hairpin`.
2. Expected: `exit-direct` stays `CRITICAL/MEDIA_PLAYBACK_FAILED` (it is the control; the explanation is `EGRESS_REFUSED` only with a same-run reference that plays, see I-83), `exit-hairpin` goes OK after the first run, and after two consecutive
   OK runs the alert clears with `RECOVERED` (**this is the real recovery evidence EXT-2 needs**; the hermetic tests do not replace it).
3. C6: REALITY and Hysteria2 clients, 10/10 genuine Shorts each; non-Google traffic exits from the staging IP, the Google set from the target.
4. C7: update, `--repair` and a reboot leave the hairpin config and the routing fingerprint unchanged.
5. C8 (native): the iPhone procedure, step S1 (10 consecutive Shorts incl. the known-failing one, normal video with seeking, both transports,
   Wi-Fi and cellular, reconnect). Until this is recorded, EXT-1 and CLI-2 stay BLOCKED/UNVERIFIED.

## Failure handling and failover (design, not implemented)
* One approved target means **no failover exists**. On target loss the hairpin fails closed and the monitor reports it (`PATH_DOWN` or
  `MEDIA_PLAYBACK_FAILED`); the operator is told, traffic is never silently routed through the refused address or any unapproved one.
* A second approved target is the precondition for any failover. It would need its own C1-C8, a bounded switch with hysteresis, and an owner
  decision; none exists, so none is built.

## Gates this moves
EXT-1 (real Shorts), EXT-2 (monitor healthy/recovery/alert-clear on a real egress), CLI-2 step S1, and the "Shorts" half of the production definition
of done. Evidence goes under `docs/handoffs/<date>/evidence/egress/` and is referenced from `GATES.json`; the verdict is computed, never typed.
