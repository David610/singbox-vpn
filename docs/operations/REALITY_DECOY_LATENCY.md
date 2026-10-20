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
