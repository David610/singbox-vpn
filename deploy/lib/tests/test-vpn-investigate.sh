#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
TOOL="$ROOT/deploy/lib/vpn-investigate.sh"
bash -n "$TOOL"
"$TOOL" --help | grep -q 'at most 300s'
"$TOOL" --help | grep -q 'streaming'
"$TOOL" --help | grep -q 'mtu'
"$TOOL" --help | grep -q 'youtube'
"$TOOL" --help | grep -q 'tiktok'
"$TOOL" --help | grep -q 'client'
if "$TOOL" capture 'not-an-ip' /tmp/no.pcap 1 2>/dev/null; then exit 1; fi
if "$TOOL" capture 192.0.2.1 /tmp/no.pcap 301 2>/dev/null; then exit 1; fi
if "$TOOL" target 'bad host!' 443 2>/dev/null; then exit 1; fi
if "$TOOL" target example.com 70000 2>/dev/null; then exit 1; fi

# udp-egress-capture: same input-validation contract as capture.
"$TOOL" --help | grep -q 'udp-egress-capture'
if "$TOOL" udp-egress-capture 'not-an-ip' /tmp/no.pcap 1 2>/dev/null; then exit 1; fi
if "$TOOL" udp-egress-capture 192.0.2.1 /tmp/no.pcap 301 2>/dev/null; then exit 1; fi
if "$TOOL" udp-egress-capture 192.0.2.1 /tmp/no.notpcap 1 2>/dev/null; then exit 1; fi

# udp-egress-verdict: the Phase-1 A/B/C/D correlator
# (docs/YOUTUBE_NATIVE_APP_INVESTIGATION.md §9.1/§8). Input validation is
# always checked; the real tshark-driven verdict is only exercised when
# tshark and tcpdump are both available (same optional-tool convention as
# test-idn-punycode.sh's SKIP path) since this sandbox/CI does not
# guarantee either is installed.
"$TOOL" --help | grep -q 'udp-egress-verdict'
# Regression for the local-address direction fix: this deployment's own
# Hysteria2 inbound also listens on UDP/443 on this same host (same port
# as application-QUIC egress toward Google), so udp-egress-verdict must
# anchor egress-vs-inbound direction to this host's own addresses rather
# than port number alone — otherwise Hysteria2 control traffic (or
# internet scanner noise hitting the public port) reads as false Case
# B/C "application QUIC reached Google" evidence. See the function's own
# comment in vpn-investigate.sh for the full mechanism.
"$TOOL" --help | grep -q "anchored to this host's own local IP addresses"
# local_addrs() must never fail even when 'ip' is unavailable (this
# sandbox has no 'ip' command, which exercises exactly that path) — it
# must return empty output with exit 0, not error out and abort the
# whole verdict. bash -c runs without this script's `set -e`, so a
# nonzero exit from local_addrs propagates as bash -c's own exit status,
# which this script's `set -e` then catches as a test failure.
LOCAL_ADDRS_SRC=$(sed -n '/^local_addrs() {/,/^}/p' "$TOOL")
bash -c "$LOCAL_ADDRS_SRC"$'\n''local_addrs -4' >/dev/null
bash -c "$LOCAL_ADDRS_SRC"$'\n''local_addrs -6' >/dev/null
if "$TOOL" udp-egress-verdict /tmp/no.pcap 'not-an-ip' 2>/dev/null; then exit 1; fi
if "$TOOL" udp-egress-verdict /tmp/does-not-exist.pcap 192.0.2.1 2>/dev/null; then exit 1; fi
if ! command -v tshark >/dev/null 2>&1 || ! command -v tcpdump >/dev/null 2>&1; then
  echo "SKIP: udp-egress-verdict's real pcap-driven verdict needs tshark and tcpdump, neither guaranteed present here — input-validation contract above still applies."
else
  VERDICT_DIR=$(mktemp -d)
  trap 'rm -rf "$VERDICT_DIR"' EXIT
  CLIENT_IP=192.0.2.55

  # Deterministic fixture: a valid, empty capture (a bounded tcpdump window
  # on loopback with a filter that never matches real host traffic) — this
  # exercises the "0 TCP/443 packets to the client" branch reliably,
  # without depending on this sandbox actually being able to complete a
  # TCP/443 handshake to itself.
  EMPTY_PCAP="$VERDICT_DIR/empty.pcap"
  sudo timeout 2 tcpdump -i lo -w "$EMPTY_PCAP" 'tcp port 1' >/dev/null 2>&1 || true
  if [[ -f "$EMPTY_PCAP" ]]; then
    EMPTY_OUT="$(sudo "$TOOL" udp-egress-verdict "$EMPTY_PCAP" "$CLIENT_IP" 2>&1)"
    echo "$EMPTY_OUT" | grep -q '0 TCP/443 packets'
    echo "$EMPTY_OUT" | grep -q 'VERDICT: INCONCLUSIVE'
    echo "$EMPTY_OUT" | grep -q 'FACT'
    echo "$EMPTY_OUT" | grep -q 'INFERENCE'
    if echo "$EMPTY_OUT" | grep -qiE 'private_key|reality[_ ]?private|vless_uuid|hysteria2_password'; then
      echo "FAIL: udp-egress-verdict printed something secret-shaped" >&2
      exit 1
    fi
  else
    echo "SKIP: this sandbox could not write a loopback pcap fixture — the input-validation contract above still applies."
  fi
fi

# streaming: input validation (P2). Real network behavior is not exercised
# here — this sandbox/CI has no representative sustained-flow network path
# to assert timing/throughput numbers against, so only the argument-bounds
# contract is checked. Manual verification against a real VPS is required
# before trusting its PASS/WARN output (see docs).
if "$TOOL" streaming 4 2>/dev/null; then exit 1; fi   # below the 5s floor
if "$TOOL" streaming 121 2>/dev/null; then exit 1; fi # above the 120s ceiling
if "$TOOL" streaming notanumber 2>/dev/null; then exit 1; fi

# mtu: input validation (P5). The real DF-bit ping sweep requires ICMP
# permissions/tooling this sandbox does not reliably have; only hostname
# validation is checked here.
if "$TOOL" mtu 'bad host!' 2>/dev/null; then exit 1; fi
if "$TOOL" mtu '' 2>/dev/null; then exit 1; fi

# client: input validation, secret-safety, and FACT/INFERENCE/UNKNOWN
# labeling contract (P9). Real journalctl/ss/firewall data is not
# guaranteed representative in this sandbox/CI, so beyond input
# validation this only asserts on the static contract every environment
# must uphold: never print a secret, always label every line.
if "$TOOL" client 'not-an-ip' 2>/dev/null; then exit 1; fi
if "$TOOL" client '' 2>/dev/null; then exit 1; fi
CLIENT_OUT="$("$TOOL" client 203.0.113.5 2>&1)"
echo "$CLIENT_OUT" | grep -q 'FACT'
echo "$CLIENT_OUT" | grep -q 'INFERENCE'
if echo "$CLIENT_OUT" | grep -qiE 'private_key|reality[_ ]?private|vless_uuid|hysteria2_password'; then
  echo "FAIL: client subcommand printed something secret-shaped" >&2
  exit 1
fi

# pairing: usage presence, missing-config validation, PAIRED/UNPAIRED/exit
# verdicts, secret-safety, and FACT/INFERENCE/UNKNOWN labeling. Pure config
# reading — no service, firewall, or route mutation. Guarded on awk (used
# only to parse the deployment config), same optional-tool convention as the
# tshark/tcpdump path above.
"$TOOL" --help | grep -q 'pairing'
if "$TOOL" pairing /tmp/does-not-exist-deployment.toml 2>/dev/null; then exit 1; fi
if command -v awk >/dev/null 2>&1; then
  PAIR_DIR=$(mktemp -d)
  if [[ -n "${VERDICT_DIR:-}" ]]; then
    trap 'rm -rf "$PAIR_DIR" "$VERDICT_DIR"' EXIT
  else
    trap 'rm -rf "$PAIR_DIR"' EXIT
  fi

  cat >"$PAIR_DIR/paired.toml" <<'TOML'
schema_version = 2
node_id = "ru1"
role = "relay"
public_host = "135.106.178.167"
subscription_host = "135.106.178.167"

[reality]
listen_port = 443
handshake_server = "www.microsoft.com"

[hysteria2]
listen_port = 443

[subscription]
listen_port = 8443
public_port = 8443

[udp_probe]
ipv4_resolvers = ["1.1.1.1"]
retries = 2
timeout_ms = 2000
delay_ms = 250

[[access_paths]]
id = "relay-egress"
kind = "relay"
via_endpoint_id = "reality-1"
capabilities = ["tcp"]

[[peer_endpoints]]
id = "de1-via-ru1"
tag = "DE via RU"
host = "91.244.71.165"
port = 443
transport = "vless_reality"
failure_domain = "de-node"
reality_public_key = "test-public-key-that-must-never-print"
path = "relay-egress"
TOML

  PAIRED_OUT="$("$TOOL" pairing "$PAIR_DIR/paired.toml" 2>&1)"
  echo "$PAIRED_OUT" | grep -q 'RELAY-PAIRED'
  echo "$PAIRED_OUT" | grep -q 'FACT'
  echo "$PAIRED_OUT" | grep -q 'INFERENCE'
  echo "$PAIRED_OUT" | grep -q 'UNKNOWN'
  if echo "$PAIRED_OUT" | grep -qiE 'test-public-key-that-must-never-print|private_key|reality[_ ]?private|vless_uuid|hysteria2_password|obfs_password'; then
    echo "FAIL: pairing printed something secret-shaped" >&2
    exit 1
  fi

  sed '/\[\[peer_endpoints\]\]/,$d' "$PAIR_DIR/paired.toml" >"$PAIR_DIR/unpaired.toml"
  UNPAIRED_OUT="$("$TOOL" pairing "$PAIR_DIR/unpaired.toml" 2>&1)"
  echo "$UNPAIRED_OUT" | grep -q 'RELAY-UNPAIRED'

  sed 's/role = "relay"/role = "exit"/; /\[\[access_paths\]\]/,$d' "$PAIR_DIR/paired.toml" >"$PAIR_DIR/exit.toml"
  EXIT_OUT="$("$TOOL" pairing "$PAIR_DIR/exit.toml" 2>&1)"
  echo "$EXIT_OUT" | grep -q 'NOT role=relay'
else
  echo "SKIP: pairing verdict assertions need awk - only the usage/missing-config contract above applies."
fi

# session-capture / session-verdict: root-causing a mid-session failure that
# survives setup (docs/YOUTUBE_FINAL_ROOT_CAUSE.md §10 item 4). Input
# validation is always checked; the real tshark-driven verdict is only
# exercised when tshark and tcpdump are both available, same optional-tool
# convention as udp-egress-verdict above.
"$TOOL" --help | grep -q 'session-capture'
"$TOOL" --help | grep -q 'session-verdict'
if "$TOOL" session-capture 'not-an-ip' /tmp/no.pcap 60 2>/dev/null; then exit 1; fi
if "$TOOL" session-capture 192.0.2.1 /tmp/no.notpcap 60 2>/dev/null; then exit 1; fi
if "$TOOL" session-capture 192.0.2.1 /tmp/no.pcap 59 2>/dev/null; then exit 1; fi
if "$TOOL" session-capture 192.0.2.1 /tmp/no.pcap 1801 2>/dev/null; then exit 1; fi
if "$TOOL" session-verdict /tmp/no.pcap 'not-an-ip' 2026-01-01T00:00:00Z 2>/dev/null; then exit 1; fi
if "$TOOL" session-verdict /tmp/does-not-exist.pcap 192.0.2.1 2026-01-01T00:00:00Z 2>/dev/null; then exit 1; fi
if ! command -v tshark >/dev/null 2>&1 || ! command -v tcpdump >/dev/null 2>&1; then
  echo "SKIP: session-verdict's real pcap-driven verdict needs tshark and tcpdump, neither guaranteed present here — input-validation contract above still applies."
else
  SESSION_DIR=$(mktemp -d)
  trap 'rm -rf "$SESSION_DIR" "${PAIR_DIR:-}" "${VERDICT_DIR:-}"' EXIT
  SESSION_CLIENT=192.0.2.77

  # A bad FAILURE_TIME_UTC must be rejected before any pcap/tshark work, on a
  # real (if empty) pcap — not just on a missing one.
  EMPTY_SESSION_PCAP="$SESSION_DIR/empty.pcap"
  sudo timeout 2 tcpdump -i lo -w "$EMPTY_SESSION_PCAP" 'tcp port 1' >/dev/null 2>&1 || true
  if [[ -f "$EMPTY_SESSION_PCAP" ]]; then
    if "$TOOL" session-verdict "$EMPTY_SESSION_PCAP" "$SESSION_CLIENT" 'not-a-real-time' 2>/dev/null; then
      exit 1
    fi

    SESSION_OUT="$(sudo "$TOOL" session-verdict "$EMPTY_SESSION_PCAP" "$SESSION_CLIENT" 2026-01-01T00:00:00Z 2>&1)"
    echo "$SESSION_OUT" | grep -q '0 TCP/443 packets'
    echo "$SESSION_OUT" | grep -q 'VERDICT: INCONCLUSIVE'
    echo "$SESSION_OUT" | grep -q 'FACT'
    echo "$SESSION_OUT" | grep -q 'INFERENCE'
    if echo "$SESSION_OUT" | grep -qiE 'private_key|reality[_ ]?private|vless_uuid|hysteria2_password'; then
      echo "FAIL: session-verdict printed something secret-shaped" >&2
      exit 1
    fi
  else
    echo "SKIP: this sandbox could not write a loopback pcap fixture — the input-validation contract above still applies."
  fi
fi

# youtube takes no arguments and performs real outbound network calls
# (DNS/TCP/TLS to public Google/YouTube domains) with no bounded
# input-validation path of its own to test in isolation — CI/this sandbox
# has no guaranteed representative path to those domains, so it is
# exercised manually against a real VPS rather than asserted on here.

# tiktok: same shape/rationale as youtube above (real outbound network
# calls, no bounded input-validation path of its own) — exercised
# manually against a real VPS. Not asserted on here.

echo 'vpn-investigate validation tests: PASS'
