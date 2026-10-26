# Bidirectional two-hop deployment on two physical hosts

Goal: two independently selectable Hiddify profiles on the SAME two servers.

| Profile | Chain | Egress of ordinary traffic |
|---|---|---|
| **A: RU -> EU** | client -> `ru1` (Russia) -> `eu1` (Europe) -> Internet | Europe |
| **B: EU -> RU** | client -> `eu1` (Europe) -> `ru1` (Russia) -> Internet | Russia |

Both are genuine two-hop connections: the client holds an end-to-end credential for the SECOND host inside the
first host's tunnel (the first host only forwards bytes), exactly as in the single-direction relay design
(`docs/RELAY_EXIT_VISION.md`, `docs/PAIRING.md`).

## Decision: a relay that can also be an exit (exit principals)

Options compared:

| | A. two isolated instances per host | **B. one relay deployment + exit principals (chosen)** |
|---|---|---|
| Processes / units / state dirs | 2 sing-box, 2 subscription services, 2 state trees per host | 1 of each (unchanged) |
| TCP 443 | only ONE REALITY inbound can own it on one IPv4 address; the other instance must use an alternate port | TCP 443 for both entries, no alternate port |
| Code touched | unit names are hardcoded in ~85 Rust call sites and in install/update/uninstall/health-check/watchdog scripts: a wide change to the lifecycle that is already qualified | one new block in the RELAY renderer, one shared verifier, two small pairing changes |
| New attack surface | new units, firewall rules, certificates, ports | one new, explicit, opt-in credential class |
| Recursion / loop risk | structurally impossible (separate users) | prevented in the rule block (below) and tested |
| Footprint on `ru1` (1 vCPU, 961 MB RAM, 4.6 GB free) | a second sing-box + services on the tightest host | none |

**Exit principal**: a dedicated user flagged `exit_principal` on a relay that set `[exit_principals] enabled = true`.
Connections authenticated as it leave through the relay's own `direct` outbound, preceded (in this order, all scoped
to that principal and to the REALITY inbound) by: `resolve`, the C-16 private/special-range and SMTP rejects, the
node's own address, the declared peers (so a principal can never bounce through the other relay), the exit IPv6/spoof
policy. Every other user keeps the relay's behaviour (forward only to declared exits, else reject). `doctor` verifies
the block against what the renderer emits and fails on any deviation. Disabled, expired, hairpin, probe and
non-opted-in cases get no route (fail closed). `pair issue-exit` on an opted-in relay creates such a principal.

The old exit role is unchanged and still supported. A host can only be converted from exit to relay by reinstalling
(a node's role is never rewritten), which is the migration step for the European host below.

## Physical topology and ports (no new ports)

| Host | Role | Listeners | Public ports |
|---|---|---|---|
| `ru1` 135.106.178.167 (Moscow) | relay + exit principals | sing-box REALITY TCP 443, Hysteria2 UDP 443 (unused by relays), nginx 8443 -> `vpn-subscription` 127.0.0.1:9100 | 22, 443/tcp, 443/udp, 8443/tcp |
| `eu1` 91.244.71.165 (Berlin) | relay + exit principals | same | same |

Firewall: unchanged (the existing `ufw` rules already allow exactly these). Provider firewalls: unchanged. DNS: the
existing `sslip.io` names keep working; real domain names are an owner choice, not required.

Each host publishes ITS OWN subscription (the host a profile enters at): profile A on `ru1:8443`, profile B on
`eu1:8443`. Distinct relay users per profile and per friend; distinct dedicated exit principals per user and
direction; separate tokens. Revoking one never touches the other (tested).

## Pairing (documented pipes; since rc.8 only the bundle travels on stdout)

    # profile A: exit principal issued BY eu1, accepted ON ru1
    ssh eu1 'sudo vpn-admin pair issue-exit --label friend1a' \
      | ssh ru1 'sudo vpn-admin pair accept-exit --user <friend1-id-on-ru1> --endpoint-id eu1-via-ru1 --bundle-stdin'
    # profile B: exit principal issued BY ru1, accepted ON eu1
    ssh ru1 'sudo vpn-admin pair issue-exit --label friend1b' \
      | ssh eu1 'sudo vpn-admin pair accept-exit --user <friend1-id-on-eu1> --endpoint-id ru1-via-eu1 --bundle-stdin'

Run with `set -o pipefail` and check `pair status` on both hosts after each (see `docs/PAIRING.md`).

## The two Hiddify subscriptions (exact formats)

    A (enters Russia, exits Europe):  https://135-106-178-167.sslip.io:8443/sub/<token-A>?format=singbox&compat=hiddify-pinned
    B (enters Europe, exits Russia):  https://91-244-71-165.sslip.io:8443/sub/<token-B>?format=singbox&compat=hiddify-pinned

Each yields ONE selectable server (`<exit> · via <entry>`) and refuses every form that would skip the entry.


## Security model (reviewed; each item is covered by a test named in the PR)

| Property | Where enforced / tested |
|---|---|
| Opt-in only: no `[exit_principals]` section, or `enabled = false`, renders exactly the old relay document; a flagged user is ignored | `exit_principals.rs::a_node_that_did_not_opt_in_ignores_flagged_users_and_stays_fail_closed` |
| Ordinary relay users never gain `direct`; the only `direct` route is the four-key rule scoped to the REALITY inbound and the principal ids | `only_principals_leave_directly_and_clients_never_gain_a_route`, `bi02_*` (real sing-box) |
| Disabled, expired, hairpin, probe accounts are never principals (fall to the terminating reject) | `hairpin_probe_inactive_and_expired_accounts_never_become_exit_principals` |
| Private/reserved ranges, SMTP, the node itself and every declared peer are rejected BEFORE `direct`, so a principal cannot loop through the other relay | `the_block_rejects_private_ranges_smtp_self_and_the_declared_peers_before_direct`, `bi02_*` |
| Each direction has its own users, principals, tokens; revoking one leaves the other working | CLI test `bidirectional_two_hosts_pair_both_directions_with_independent_identities_and_revocation`, `bi04_*` |
| A down host fails closed in both profiles; no wrong-country fallback | `bi03_*` |
| Pinned Hiddify profile: one selectable server, hidden first hop, share-link forms refused on relays | unchanged code path of the single-direction relay design; `services/subscription` tests |
| `doctor` fails on a missing, reordered, extra or tampered block, or on a block when the node did not opt in | `the_verifier_detects_every_tampering_of_the_block` |
| TLS spoof / existing hairpin / exit-only deployments unaffected | real `sing-box check` of the hybrid document with and without spoof; the unchanged hairpin and exit suites |
| Section is relay-only; unknown keys rejected | `validation_keeps_the_section_relay_only_and_unknown_keys_out` |

What `doctor` does and does not establish (three distinct guarantees):

1. **The rendered policy is sound.** `report_relay_policy` checks the document freshly rendered from the current
   store: shape, rule order, the exact exit-principal block, the terminating reject, and that the block's principals
   equal `exit_principal_ids` (the accounts the store currently authorizes: active, flagged, opted in).
2. **The on-disk file matches current authorization.** The L4 drift check compares `config.json` with that fresh
   document byte-for-semantics; a stale grant for a revoked principal, an extra principal or a missing block FAILS
   (`doctor_fails_stale_or_unauthorized_exit_principal_rules_on_disk`).
3. **The running sing-box process uses that file** is NOT established by either check. It is covered only by
   `doctor --protocol` (a real handshake against the live process) and by the post-deployment acceptance matrix.

Known limit (accepted, documented): `DeploymentConfig` and `CompatUser` do not deny unknown fields, so a binary older
than this change silently ignores `[exit_principals]` and the user flag. That fails closed (the principal then has
no route; the relay stays forward-or-reject) but `doctor` of the older binary will not mention it. Rollback therefore
removes the section explicitly.

## Google/YouTube policy (no live egress policy is changed by this work)

Nothing here is verified on a device; the recorded evidence is the only basis.

| Profile | Initial policy | Status |
|---|---|---|
| A: RU -> EU | Google/YouTube leaves via the European exit (no hairpin on the Russian entry) | **UNVERIFIED on `91.244.71.165`**: its IP was previously refused Shorts by YouTube (`docs/YOUTUBE_FINAL_ROOT_CAUSE.md`); only the German staging IP has played Shorts |
| B: EU -> RU | ordinary traffic exits Russia | **Google/YouTube playback NOT QUALIFIED.** The Russian host's network closes YouTube TLS handshakes; TLS spoof exists for that and is not verified in this direction. Needs further authorization and real-device testing |

Sustained Shorts playback has not been shown for either profile, and no claim is made. A change of the REALITY decoy
does not affect destination-side blocking. The historical hosts' live egress policy is not touched. The earlier
comparison through the historical hosts was stopped by the operator and is not repeated.

## Resource assessment (ru1: 1 vCPU, 961 MB RAM, 4.6 GB free disk)

- Runtime: unchanged process set (one sing-box, one subscription service, nginx). The new rule block adds a handful of
  routing rules; no extra memory-resident component.
- Backup: the archive holds only users, key material and `deployment.toml` (hard limits 32 entries / 64 MB); a few KB
  in practice. Keep at most two copies on `ru1`; copy others off-host over SSH.
- Update: binaries are prebuilt and checksum-verified (no compile on the host). Need roughly 150 MB free for download,
  staging and the rollback copy; 4.6 GB is ample. Check `df -h /` and `free -m` before starting; abort below 1 GB free.
- Reinstall of the European host (2 vCPU, 3.9 GB, 56 GB free) has no resource concern.

## Migration (needs separate explicit deployment authorization; nothing here was applied to any host)

Treat the European conversion as a disruptive production migration, not an update.

### Facts established read-only (2026-10-26)

`ru1` is a relay (rc.6) with test/pilot users and 0 connected clients; `91.244.71.165` is an EXIT (rc.6) with a hairpin
and test users, 0 connected clients; both `doctor` clean, certificates ~85 days. A backup is valid only for the node it came
from: it carries `deployment.toml`, and `restore` REFUSES (`refuse_cross_node_restore`) when the archived role or
node id differs from the installed one. An exit backup therefore can never be restored into a relay (nor the reverse),
and a restore needs the original node id. Recorded ids: `ru1` = relay, node id `ru1`; Evolus = exit, node id
`91-244-71-165` (not `eu1`). Write both down in the pre-flight and keep them with the backups.

### 0. Preconditions

1. Merged PR, a tagged release candidate (checksums + attestation verified as for rc.8), release notes.
2. Owner's written authorization naming both hosts and the window.
3. Rehearsal: the Germany/Finland staging pair stays untouched, so the rehearsal is the CI `bi01`-`bi04` scenarios, the CLI
   pairing test, and a `vpn-admin restore` of each backup into a scratch config tree on the workstation.
4. Both hosts: `df -h /`, `free -m`, `vpn-admin doctor` (0 FAIL), `systemctl is-active sing-box vpn-subscription nginx`,
   certificate expiry > 30 days, port 22/443/8443 reachable from outside, no connected client
   (`vpn-admin status`), the previous release tag available.

### 1. Protected backups (both hosts, as root, files 0600, contents never printed or committed)

    install -d -m 0700 /root/pre-bidir
    vpn-admin backup --output /root/pre-bidir/<host>-<UTC>.tar
    cp -a /etc/vpn/deployment.toml /root/pre-bidir/deployment.<host>.toml
    ( cd /root/pre-bidir && sha256sum * > SHA256SUMS )
    vpn-admin doctor > /root/pre-bidir/doctor-before.<host>.txt

Then copy the directory off-host over SSH to the owner's workstation, encrypted at rest (`age`), mode 0600; verify
`sha256sum -c` there. Keep the existing `/root/pre-rc*` archives. Record (not print) fingerprints of `users.json`,
REALITY public key and the rendered sing-box config. Prove restorability BEFORE any change by running
`vpn-admin restore` of each archive against a scratch config tree on the workstation (same role as the archive).

### 2. `ru1`: in-place update (role stays relay, ~1-2 min of sing-box reload)

1. Pinned-tag update to the new release (same path used for rc.7 -> rc.8 on the staging pair).
2. Append to `/etc/vpn/deployment.toml`:

        [exit_principals]
        enabled = true

3. `vpn-admin render-config` (validate-then-apply, automatic rollback on failure), `vpn-admin doctor` (L2 must read
   "fail-closed ... exit-principal block" only once a principal exists; without principals it is the old line).
4. Existing relay users and the pilot route to Evolus keep working at this point (Evolus is still an exit).

### 3. European host: exit -> relay (reinstall; profile A offline from here until step 5)

1. `uninstall.sh` (confirm what it removes: units, `/etc/vpn`, state; the `/root/pre-bidir` copies stay).
2. Pinned-tag installer with `--role relay --node-id eu1` and the same public host (`91-244-71-165.sslip.io` or the owner's
   domain), then the same `[exit_principals]` section and `render-config`, `doctor`.
3. Not carried over (no real users exist): the three test users and the old hairpin. The firewall/ports are the same.

### 4. Pair both directions and create users

Commands in "Pairing" above. One relay user per person per profile (on the profile's entry host), one dedicated
principal per user per direction, fresh tokens. Pilot `friend-pilot-ru1` is re-paired
(`accept-exit --replace-credential`) because its old Evolus exit credential died with the reinstall.

### 5. Pre-handout validation (disposable users, then deleted)

`vpn-admin doctor --protocol` on both; `pair status` on both; the acceptance matrix below from a machine outside Russia
(egress IP check) and then from a Russian network.

### Downtime and impact

| Item | Impact |
|---|---|
| `ru1` update + reload | existing relay route interrupted ~1-2 min; no real users |
| Evolus reinstall | its exit role is offline ~10-15 min; the only consumer is the `ru1` pilot route |
| Germany/Finland staging | **none** — no step touches `157.173.27.46` or `62.238.46.190` |
| DNS | none changed (sslip.io names are derived from the IPs) |
| Certificates | Evolus gets a new certificate on reinstall; check issuance limits/ports 80 or the DNS-01 path first |
| Ports | none added; TCP 443 REALITY on both, 8443 subscription, UDP 443 unused by relays |

### Acceptance matrix (after deployment, before any user link is shared)

| # | Check | Pass criterion |
|---|---|---|
| 1 | Profile A egress IP | `91.244.71.165` (and no `135.106.178.167`) |
| 2 | Profile B egress IP | `135.106.178.167` (and no `91.244.71.165`) |
| 3 | Pinned profile content | exactly one selectable server, no `direct`/other outbound selectable |
| 4 | Entry host down (each) | the matching profile fails closed; the other still works |
| 5 | Exit host down (each) | same |
| 6 | Revoke one relay user | only that profile stops; the other still works; exit principal revoked separately |
| 7 | Ordinary relay user opens a connection to arbitrary hosts | rejected |
| 8 | Principal connects to the other host's 443/ private ranges / SMTP | rejected |
| 9 | DNS and IPv6 | resolver and IPv6 egress follow the tunnel (no leak; IPv6 policy of the exit) |
| 10 | `doctor`, `doctor --protocol`, `repair`, update dry-run, `backup` | clean on both after the change; restart both services, confirm routes persist |
| 11 | Germany/Finland staging | unchanged `doctor` + working EU-only profile |
| 12 | Russian-network reachability of BOTH entries (mobile and Wi-Fi), Hiddify Android/iPhone | connects and loads sites |
| 13 | Google/YouTube | marked per policy table; Shorts only after sustained playback evidence |

### Rollback (decision points are explicit; executed only if needed)

- **`ru1`**: remove `[exit_principals]` from `deployment.toml` (older binaries ignore it silently), then
  `vpn-admin restore /root/pre-bidir/ru1-<UTC>.tar` (same role = relay), `vpn-admin render-config`, `doctor`; downgrade
  to the previous tag via the pinned installer only if the new binary is at fault.
- **European host**: its new installation is a relay, so rollback is a reinstall, not a restore: `uninstall.sh`,
  pinned-tag installer of the previous release with `--role exit --node-id 91-244-71-165` and the same host (the
  restore refuses any other node id), THEN
  `vpn-admin restore /root/pre-bidir/eu1-<UTC>.tar` (exit archive into an exit install). Re-pair the `ru1` pilot route if the
  exit's credentials were regenerated. Verify `doctor --protocol` on both.
- If the new Evolus relay misbehaves but `ru1` is fine: profile B is withdrawn, profile A can be kept off; ru1 stays a
  plain relay without principals (step 2 reversed) — no user-facing state is lost.
- The off-host encrypted copies are the recovery path if the host itself is lost.

### Risks

1. Evolus IP is known to be refused by YouTube Shorts (profile A).
2. Russian network may interfere with either entry (profile reachability from Russia is unqualified).
3. Reinstall destroys the old exit identity; any client still holding it must be re-paired (none expected).
4. Older-binary downgrade silently ignores the new section (documented, fail closed).
5. 1 GB RAM on `ru1`: observe memory after the update.

## Remaining validation (cannot be done from CI or this workstation)

- Profile A and B from a real Russian network, in Hiddify on Android and iPhone.
- Sustained signed-in/out Shorts for each profile; Google/YouTube remains per the policy table.
- UDP/QUIC through either relay is not carried (TCP only), unchanged.
