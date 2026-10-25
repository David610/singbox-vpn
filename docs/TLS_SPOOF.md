# Google-egress TLS spoof (exit or relay, opt-in)

Some networks (observed: a Russian hosting provider, 2026-10-24) drop TLS handshakes whose
SNI is a blocked name, so an exit there cannot reach YouTube at all. sing-box 1.14
`tls_spoof` sends a forged ClientHello with an allow-listed SNI ahead of the real one; the
forged segment is made unacceptable to the real server (`wrong-sequence` by default), so the
genuine handshake, the genuine SNI and certificate verification are untouched. It is a
middlebox workaround, not encryption or authentication, and the filter owner can defeat it.

Scope: TLS to the Google/YouTube domain set (`GOOGLE_EGRESS_DOMAINS`), TCP/443, on the node
that dials Google. QUIC/UDP is not covered. On a relay it applies only to the dedicated hairpin principal (the relay is the Google egress for another exit's hairpin). Not allowed together with `[google_egress_hairpin]`
(an exit that hairpins elsewhere does not dial Google itself). Absent = rendering unchanged.

## Enable

One command, on an exit or on a relay (a relay spoofs only for its hairpin principal):

    vpn-admin tls-spoof set --sni ya.ru            # --method wrong-sequence is the default
    vpn-admin tls-spoof show
    vpn-admin tls-spoof clear

`set` patches only `[google_egress_tls_spoof]` in `deployment.toml` (validated; never a Google name for
`--sni`), installs the managed capability drop-in, and restarts sing-box through the normal apply path. It is
rolled back (file and runtime) if validation or the restart fails.
`set`/`clear` also fail (and restore the previous setting) if the drop-in cannot be written/removed, systemd
cannot reload it, or the restarted sing-box still lacks the capabilities (for example another drop-in narrows
`CapabilityBoundingSet`); a success line is never printed in those cases. Other `render-config` callers (user
changes, the expiry timer) only warn on a drop-in failure, so an unwritable systemd directory can never stop an
expired user from being removed; `doctor` reports the gap.

`update.sh` rollback: it restores the previous binaries, units and `deployment.toml` but does not touch the
drop-in. An update never changes the spoof setting, so the drop-in stays consistent with the restored
`deployment.toml` (present while configured, absent otherwise); a drop-in created by the failed update remains
and only grants capabilities the restored configuration already requires. No outage or privilege mismatch
results.


The drop-in is `/etc/systemd/system/sing-box.service.d/tls-spoof.conf`. Policy:

- It exists only while the spoof is configured, and grants exactly CAP_NET_BIND_SERVICE (repeated because the
  lists replace the unit's), CAP_NET_RAW and CAP_NET_ADMIN. The shipped unit is never edited and no other
  hardening is changed. No AF_PACKET change is needed under the shipped sandbox (verified on Ubuntu 24.04).
- Its first line is a marker. Only a file with that marker is rewritten or removed. An operator-made file at
  the same path with the same capabilities is adopted (rewritten once with the marker); one with different
  capabilities is left untouched and reported.
- Every `render-config` (installer, `update.sh`, `--repair`, the reconcile timer) keeps it in step with the
  setting, and restarts sing-box only when the drop-in changed or the running process is missing the
  capabilities (at most once per drop-in content, so a host that cannot grant them is not restarted in a loop).
- `uninstall.sh` removes a managed drop-in; `vpn-admin doctor` (exit and relay) FAILs when the running
  sing-box lacks the capabilities (without them it starts and the spoof silently does nothing).

## Evidence and limits

See `docs/qualification/2026-10-24/` for the measurements. TCP handshake success is not
playback: verify anonymous Shorts and normal-video media delivery, then the native iPhone
matrix, before relying on it. Rejecting or tolerating UDP/443 to Google on that node is a
separate decision.
