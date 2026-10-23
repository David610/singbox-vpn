# Shorts start-then-freeze: egress qualification (2026-10-23)

Aggregate, address-free summary. Host-specific detail is kept privately. Evidence classes follow `CLAUDE.md`.
**Status: Shorts are NOT fixed. Standalone VPN: NO-GO. Commercial Arcana: NO-GO.**

## Timeline of what was established
1. With Google traffic exiting the entry node's own datacenter IP, native Incognito Shorts failed (USER_REPORTED). A browser probe from that IP shows the
   documented media refusal (tiny `videoplayback` reply) on 0 of 9 Shorts, and bot challenges on long-form (SERVER_VERIFIED).
2. After pointing the Google hairpin at the paired relay, Shorts start but freeze after about 20 s (USER_REPORTED, reproduced once with synchronized captures).
3. A synchronized capture (entry + relay, headers only) during native playback showed, at the relay's egress: 9 of 185 UDP/443 flows to Google got any reply, and
   112 of 429 TCP/443 flows never received data after the handshake (SERVER_VERIFIED, header-only capture).
4. Identical TLS probes run on the relay itself, with no VPN involved: 472 of 484 handshakes naming YouTube-class hostnames got no reply, across every Google address
   range; control hostnames succeeded 350 of 363. The entry node got 481 of 484 for the same probes. The failures are hostname-dependent and independent of destination range.
   The only successes were in-country caches. The relay host's firewall allows all outbound traffic.
5. A TTL-limited sweep shows replies for a YouTube-named hello stopping after the third router while a google.com-named hello continues. This locates the *region* of the
   loss upstream of the host; **it does not prove the exact filtering hop or who operates it.**

Strongly supported conclusion: upstream-path interference with YouTube-named TLS/QUIC on the relay's network. Not conclusive for the production freeze until validated through a working alternative path.

## Corrections made during the investigation (kept for honesty)
- An early "zero-window / back-pressure" finding was wrong: `grep 'win 0'` also matches RSTs. Parsed correctly there were 2 real zero-window packets in one run and 0 in the other.
- An early "35-40% of connections stuck" figure was wrong: "no application data back" includes idle and single-send flows. Only ~2-3% showed the repeated-retry signature.
- A ~1% unanswered-probe figure came from QUIC version-negotiation probes that carry no server name; they understate real-traffic loss.

## Probe validity
The media probe saw playback on the relay path (2 Shorts played, 1 froze after 13.7 s, 6 never started), which is the positive control and a browser-level reproduction of the symptom.
It is not DEVICE_VERIFIED. Datacenter IPs on two unrelated networks (entry node and a CI runner), with both bundled Chromium and real Chrome, gave identical refusals, so the refusals are not a browser artifact.

## Open
Alternative Google egress qualification (no candidate purchased yet), then native iPhone Incognito + signed-in, REALITY + Hysteria2, full 900 s matrix. Tools: `deploy/lib/diag/`.
