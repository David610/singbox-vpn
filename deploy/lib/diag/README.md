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
| `shorts_media_probe.py`, `socks_family.py` | Playwright media probe (currentTime sampling, `videoplayback` bytes, bot challenge kept separate from refusal; `--out`, `--self-check`) and a single-address-family SOCKS front |
| `candidate_qualify.sh`, `qualify_lib.py`, `qualify_remote.py` | qualifies a candidate egress node *before* any VPN stack is installed (below) |

Caveats: probes are headless and short; IP acceptance can change; "unanswered" is an observation, not proof of a drop.

## Egress qualification (`candidate_qualify.sh`)

Answers one question about a fresh VPS before it is trusted as a Google/YouTube egress: **does YouTube
actually deliver media through it?** A TLS handshake, `playabilityStatus: OK` or HTTP 200 does not.
Whether YouTube accepts an address is a property of that address and its network path, not of the
software; this tool measures it, it cannot change it. A PASS describes only the measured workload on one
run, over a plain TCP tunnel, with a headless browser. It is never `DEVICE_VERIFIED`.

### Clean setup (any Linux machine that can reach the candidate, run as a normal user)

```bash
deploy/lib/diag/candidate_qualify.sh setup    # python3 venv + pinned Playwright + Chromium (once)
deploy/lib/diag/candidate_qualify.sh check    # verifies every prerequisite, changes nothing
```

Chromium's shared libraries need root once on a minimal image
(`<venv>/bin/python -m playwright install-deps chromium`); the probe refuses to run as root because
Chromium's sandbox stays on. The candidate only needs `python3` (3.9+), `sshd` and outbound HTTP; nothing
is installed there, a private `mktemp -d` holds the helper scripts for the run and is removed afterwards.

### Run

```bash
deploy/lib/diag/candidate_qualify.sh run --target root@CANDIDATE --label cand1 \
    --ssh-key ~/.ssh/id_ed25519 \
    --host-key-sha256 SHA256:... \
    --sustained-video VIDEO_ID
```

Take the fingerprint from the provider console, not from the network under test; `--sustained-video` is required for a PASS.

* **Host identity is never trusted silently.** Give exactly one of `--host-key-sha256` (fingerprint you
  obtained out of band), `--known-hosts FILE`, or `--accept-unverified-host-key`; the last caps the verdict
  at `INCONCLUSIVE`. A mismatch stops before any login. SSH always runs with `StrictHostKeyChecking=yes`.
* **The tunnel is proven before use.** The SOCKS tunnel must answer and its exit address must equal the
  address the candidate reports for itself; otherwise nothing is measured (`tunnel_egress_mismatch`).
* **Pinned or discovered address set.** `--ips-file FILE` pins the Google IPv4 set for reproducible runs;
  otherwise the union of what this machine and the candidate resolve is built inside the evidence dir
  (no `/tmp` file is read).
* Defaults: 9 Shorts, 25 s each, **all must play**, 2 rounds, then one long-form video for 900 s of which
  600 s must advance. All thresholds are flags and are recorded in `verdict.json`.

### Result: `verdict.json` and exit status

Every stage writes its own file in `<evidence-dir>/<label>-<UTC time>/` (mode 0700); nothing is parsed out
of mixed stdout. Exit status: `0` PASS, `1` FAIL, `2` INCONCLUSIVE, `64` usage error, `130`/`143` interrupted.

| Verdict | Meaning |
|---|---|
| `PASS` | every required measurement ran, was conclusive and met its threshold, on a verified host |
| `FAIL` | conclusive negative evidence: `media_refused` (zero-byte media or "unavailable"), `bot_challenge`, `stalled` (started then froze), `no_start`, or `tls_blocked` (YouTube SNIs fail while controls work) |
| `INCONCLUSIVE` | anything not fully measured; never counted as a refusal |

`INCONCLUSIVE` reasons: `prereq_missing`, `host_key_unavailable`, `host_key_mismatch`,
`host_identity_unverified`, `remote_stage_failed`, `egress_ip_unknown`, `network_unhealthy`,
`tunnel_not_ready`, `tunnel_egress_mismatch`, `probe_failed`, `probe_timeout`, `probe_error`,
`incomplete_probe`, `evidence_missing`, `evidence_unreadable`, `not_a_short` (the page left `/shorts/`),
`sustained_not_run`, `interrupted`, `internal_error`.

A near-miss is `FAIL`, not a pass-to-the-next-stage: 8 of 9 Shorts played is reported as
`media_refused` with `"played": 8`. Interruption (Ctrl-C, `SIGTERM`, `SIGHUP`) and every error path kill
only the processes this run started (tunnel, probe process group), remove the candidate's tempdir and
the local scratch directory, and still write a `verdict.json` (`interrupted` / `internal_error`).

Legacy positional form `candidate_qualify.sh TARGET LABEL [SSH_KEY] [flags]` still works but now requires a
host-identity flag.

Tests: `deploy/lib/tests/test-candidate-qualify.sh` (hermetic; the candidate is a directory, the browser a
stand-in) and `deploy/lib/tests/test_qualify_lib.py`. Neither contacts Google/YouTube, so they prove the
tool's decisions, not that any network plays video.
