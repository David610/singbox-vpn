# REALITY connection-setup latency and the decoy (measured 2026-10-20, I-60)

**Finding (server verified, loopback client, node 62.238.46.190):** every *new* VLESS+REALITY
connection costs a median **~60-80 ms** more than a direct connection; Hysteria2 costs ~2.5 ms
(`tunnel_perf.py`, corrected metric: end-to-end time of a tiny HTTP exchange, tunnel minus direct).
It is a per-connection setup cost, not a throughput limit (REALITY moves ~900 Mbps in the same test).

**Cause, isolated by experiment** (two throwaway REALITY server/client pairs, same sing-box 1.14.1, same
keys/flow, identical except the decoy; 40 requests each, direct = 3.6 ms):
| decoy (`handshake.server`) | median per-request time | added vs direct |
|---|---|---|
| `www.microsoft.com:443` (remote; TCP RTT 8.7 ms, TLS 1.3 handshake ~29 ms) | 84.5 ms | **+80.9 ms** |
| local TLS 1.3 server on 127.0.0.1 | 13.3 ms | **+9.7 ms** |
The server completes a TLS handshake with the decoy for each incoming connection; that remote handshake
dominates the added latency (more than the decoy's RTT alone suggests).

**Candidate remote decoys** (TLS 1.3 handshake median from this node, 8 samples each): ikea.com 20.9 ms,
cloudflare.com 23.9, amazon.com 26.9, dl.google.com 27.7, mozilla.org 28.2, **microsoft.com 28.9 (current)**,
apple.com 31.7, bbc.com 32.0, spotify.com 32.5, samsung.com 62.9, cdn77.com 77.4, nvidia.com 87.9.
Swapping to the fastest remote one would save only a few ms: remote decoys are not the lever.

**The lever (owner decision, not applied):** a decoy served *from the node itself* (a TLS 1.3 site on a
loopback port with a real certificate for a name you control, as the installer allows: "a decoy hostname you
control"). Expected saving ~70 ms per new connection. Trade-offs to weigh: needs a domain/certificate and a
small TLS site on the node; the node's IP and SNI become coupled (a consistency/DPI consideration);
**changing the decoy changes `sni=` in every client profile, so all users must re-import**;
it restarts sing-box (all connections drop). Until decided: prefer Hysteria2 where UDP/443 works, and treat
the REALITY setup cost as a known property.

Not measured: real-network behaviour (the decoy RTT from the node is independent of the client's network, so the
server-side part of this cost will be the same for a real client; the client-to-node RTT adds on top).

## CORRECTION 2026-10-21 - the decoy choice IS the lever (the earlier "not the lever" conclusion was wrong)
The section above judged remote decoys by their raw TLS 1.3 handshake time from this node and concluded that switching to the fastest one
"would save only a few ms". Measuring the cost **through REALITY** instead (`deploy/lib/reality_decoy_latency_experiment.py`: throwaway
REALITY server/client pairs on loopback, identical except for `handshake.server`, N sequential NEW connections each, variants interleaved over
rounds, minus the direct request; evidence `docs/handoffs/2026-10-21/evidence/perf/i60-decoy-*.json`) shows the opposite. Server verified,
loopback client, NOT a real client network (the decoy leg is independent of the client's network, so the server-side saving carries over).

| decoy (remote, third-party) | added ms per new connection (round medians) | median |
|---|---|---|
| www.amazon.com | 9.0, 9.6 | 9.3 |
| www.cloudflare.com | 8.9, 9.8 | 9.4 |
| www.ikea.com | 11.8, 10.5 (and 11.2, 12.7, 11.9 in the first run) | 11.2 |
| dl.google.com (excluded: see below) | 13.4, 12.4 | 12.9 |
| www.spotify.com / www.bbc.com / www.mozilla.org | 19-20 | 19-20 |
| www.apple.com | 44.8, 84.2 | 64.5 |
| **www.microsoft.com (current default)** | 61.8, 83.5 (and 62.0, 83.1, 83.6) | **72.7** |
| local TLS 1.3 server on 127.0.0.1 (lower bound) | 7.9, 8.9, 9.0 | 8.9 |

Controls: pinning the decoy's IP (`handshake.server` = address, SNI unchanged) did not change www.microsoft.com (81/81/83 ms), so per-connection DNS is **not**
the cause. All variants 0 failed requests. Raw handshake time did not predict the REALITY cost (microsoft.com 28.9 ms raw vs ikea.com 20.9 ms raw, but 73 vs 11 ms
through REALITY); *why* microsoft.com and apple.com are slow through REALITY is not established (not the DNS, not the raw handshake). Do not generalize past the
hosts measured: the ranking can change with the decoy's own behaviour and PoP.

**Consequence for I-60.** A remote decoy that measures about 9-13 ms through REALITY would put the added latency well inside the 40 ms gate **without** a local decoy,
a new domain or certificate, or any new service on the node. It is the same security class as today (a third-party site; the known IP/SNI mismatch is unchanged).
It is still a service-affecting change: `sni=` changes in **every** client profile (all users re-import; Hiddify caches profiles) and sing-box restarts.
Not applied. The 40 ms gate is unchanged and PERF-2 stays FAIL until an owner decision is carried out and re-measured on the node.
Do not use a Google-owned name (dl.google.com measured fast) as the decoy: it sits inside the protected Google/YouTube hairpin domain set
(`docs/YOUTUBE_FIX_INVARIANTS.md`), an interaction nobody has analysed.

**Before a switch** (same list as `REALITY_DECOY_EVALUATION.md`): (1) a native client imports the changed profile and connects (UNVERIFIED), (2) `doctor --protocol`
on the changed node, (3) re-run `tunnel_perf.py` on the node and confirm added ms <= 40, (4) the decoy keeps answering TLS 1.3 + h2 (checked now for amazon/cloudflare/ikea/microsoft).
The installer's default decoy is a separate product decision: changing it affects only new installs and should follow the same evidence.
