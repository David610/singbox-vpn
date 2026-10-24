# Google-egress TLS spoof (exit-only, opt-in)

Some networks (observed: a Russian hosting provider, 2026-10-24) drop TLS handshakes whose
SNI is a blocked name, so an exit there cannot reach YouTube at all. sing-box 1.14
`tls_spoof` sends a forged ClientHello with an allow-listed SNI ahead of the real one; the
forged segment is made unacceptable to the real server (`wrong-sequence` by default), so the
genuine handshake, the genuine SNI and certificate verification are untouched. It is a
middlebox workaround, not encryption or authentication, and the filter owner can defeat it.

Scope: TLS to the Google/YouTube domain set (`GOOGLE_EGRESS_DOMAINS`), TCP/443, on the node
that dials Google. QUIC/UDP is not covered. Not allowed together with `[google_egress_hairpin]`
and not valid on a relay. Absent = rendering unchanged.

## Enable

1. `deployment.toml`:

       [google_egress_tls_spoof]
       sni = "ya.ru"                 # a name the local filter allows; never a Google name
       method = "wrong-sequence"     # default; also wrong-checksum, wrong-ack, wrong-md5, wrong-timestamp

2. Capabilities. sing-box needs CAP_NET_RAW and CAP_NET_ADMIN. Without them it starts and the
   spoof silently does nothing (handshakes still time out), so `vpn-admin doctor` FAILs.
   Narrow, per-node drop-in (the shipped unit is not widened for other nodes):

       # /etc/systemd/system/sing-box.service.d/tls-spoof.conf
       [Service]
       AmbientCapabilities=CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN
       CapabilityBoundingSet=CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN

   `systemctl daemon-reload`, then `vpn-admin render-config` and restart sing-box.
   No AF_PACKET change is needed under the shipped sandbox (verified on Ubuntu 24.04).

3. `vpn-admin doctor` must report the capability check as PASS.

## Evidence and limits

See `docs/qualification/2026-10-24/` for the measurements. TCP handshake success is not
playback: verify anonymous Shorts and normal-video media delivery, then the native iPhone
matrix, before relying on it. Rejecting or tolerating UDP/443 to Google on that node is a
separate decision.
