#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
TOOL="$ROOT/deploy/lib/vpn-investigate.sh"
bash -n "$TOOL"
"$TOOL" --help | grep -q 'at most 300s'
if "$TOOL" capture 'not-an-ip' /tmp/no.pcap 1 2>/dev/null; then exit 1; fi
if "$TOOL" capture 192.0.2.1 /tmp/no.pcap 301 2>/dev/null; then exit 1; fi
if "$TOOL" target 'bad host!' 443 2>/dev/null; then exit 1; fi
if "$TOOL" target example.com 70000 2>/dev/null; then exit 1; fi
echo 'vpn-investigate validation tests: PASS'
