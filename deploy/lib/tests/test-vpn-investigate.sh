#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
TOOL="$ROOT/deploy/lib/vpn-investigate.sh"
bash -n "$TOOL"
"$TOOL" --help | grep -q 'at most 300s'
"$TOOL" --help | grep -q 'streaming'
"$TOOL" --help | grep -q 'mtu'
"$TOOL" --help | grep -q 'youtube'
"$TOOL" --help | grep -q 'client'
if "$TOOL" capture 'not-an-ip' /tmp/no.pcap 1 2>/dev/null; then exit 1; fi
if "$TOOL" capture 192.0.2.1 /tmp/no.pcap 301 2>/dev/null; then exit 1; fi
if "$TOOL" target 'bad host!' 443 2>/dev/null; then exit 1; fi
if "$TOOL" target example.com 70000 2>/dev/null; then exit 1; fi

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

# youtube takes no arguments and performs real outbound network calls
# (DNS/TCP/TLS to public Google/YouTube domains) with no bounded
# input-validation path of its own to test in isolation — CI/this sandbox
# has no guaranteed representative path to those domains, so it is
# exercised manually against a real VPS rather than asserted on here.

echo 'vpn-investigate validation tests: PASS'
