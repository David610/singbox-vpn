# Shorts / Google-egress diagnostics

Diagnostic tools used in the 2026-10-23 Shorts-freeze investigation (see
`docs/qualification/2026-10-23/SHORTS-FREEZE-EGRESS.md`). They are **diagnostics, not acceptance**:
nothing here is DEVICE_VERIFIED, and a browser probe never replaces the native iPhone matrix
(`docs/DEVICE_ACCEPTANCE_TESTS.md`).

No host address, credential or capture is stored in this directory. Addresses and key paths come from the environment:

| Variable | Meaning |
|---|---|
| `ENTRY_IP` | public IPv4 of the entry node |
| `RELAY_IP` | public IPv4 of the Google-egress relay |
| `RELAY_SSH_KEY` | path to a key that can log in to the relay (never committed) |
| `EVIDENCE_DIR` | where `candidate_qualify.sh` writes (default `./egress-qualification`) |

| Tool | Purpose |
|---|---|
| `pcap_analyze.py`, `stuck_classify.py`, `window_flows.py` | stdlib-only analysis of header-only captures (per-flow bytes, real zero-window vs RST, retransmissions; classifies "no reply" flows so idle/short flows are not counted as failures) |
| `relay_summary.py`, `relay_tcp_detail.py` | relay-side egress summary (UDP flows with/without reply, TCP flows that never receive data) |
| `sync_capture.sh` | synchronized entry+relay header-only capture for one native playback attempt (needs the owner's phone) |
| `tls_survey.py`, `tls_matrix.py`, `collect_ips.py`, `v6_check.py` | identical TLS/SNI probes from two vantage points over many Google/YouTube destinations |
| `ttl_sweep.py` | TTL-limited ClientHello to see where replies stop (locates a *region*, does not prove the filtering hop) |
| `quic_vn_probe.py`, `dead_flow_study.py` | QUIC version-negotiation probes (no SNI, so they understate real-traffic loss) |
| `shorts_media_probe.py`, `socks_family.py` | Playwright media probe (currentTime sampling, `videoplayback` bytes, bot challenge kept separate from refusal) and a single-address-family SOCKS front |
| `candidate_qualify.sh` | qualifies a candidate egress node *before* any VPN stack is installed |

Caveats: probes are headless and short; IP acceptance can change; "unanswered" is an observation, not proof of a drop.
Run `collect_ips.py` on both nodes and merge into `/tmp/ips_union.json` before `candidate_qualify.sh`.
