#!/usr/bin/env bash
# Unit tests for deploy/lib/vpn-benchmark-lib.sh's outbound-discovery
# logic. Pure bash + jq, no sing-box/network/root required — run as part
# of CI's `shell` job. Fixtures below mirror the actual shape
# `compat_config::render::render_singbox_client_subscription` produces
# (crates/compat-config/src/render.rs), not a hand-simplified guess, so
# a real renderer change that breaks discovery would also break these.
set -Eeuo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=deploy/lib/vpn-benchmark-lib.sh
. "$LIB_DIR/vpn-benchmark-lib.sh"

failures=0
assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $desc — expected [$expected], got [$actual]"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

STANDARD_SUB='{
  "outbounds": [
    {"type":"vless","tag":"Reality","server":"vpn.example.com","server_port":443,
     "tls":{"enabled":true,"reality":{"enabled":true,"public_key":"pk","short_id":"sid"}}},
    {"type":"hysteria2","tag":"Hysteria2","server":"vpn.example.com","server_port":443,
     "tls":{"enabled":true}},
    {"type":"urltest","tag":"auto","outbounds":["Reality","Hysteria2"],
     "url":"https://www.gstatic.com/generate_204","interval":"1m"},
    {"type":"selector","tag":"select","outbounds":["Reality","Hysteria2","auto"],"default":"Reality"},
    {"type":"direct","tag":"direct"}
  ],
  "route": {"final":"select"}
}'

# --- normal case: operator-renamed labels still discovered correctly ---
RENAMED_SUB='{
  "outbounds": [
    {"type":"vless","tag":"Germany - Fast Reality Node",
     "tls":{"enabled":true,"reality":{"enabled":true}}},
    {"type":"hysteria2","tag":"Germany - QUIC Node","tls":{"enabled":true}},
    {"type":"urltest","tag":"auto","outbounds":["Germany - Fast Reality Node","Germany - QUIC Node"]},
    {"type":"selector","tag":"select","outbounds":["Germany - Fast Reality Node","Germany - QUIC Node","auto"],"default":"Germany - Fast Reality Node"},
    {"type":"direct","tag":"direct"}
  ]
}'

HYSTERIA_ONLY_SUB='{
  "outbounds": [
    {"type":"hysteria2","tag":"Hysteria2","tls":{"enabled":true}},
    {"type":"urltest","tag":"auto","outbounds":["Hysteria2"]},
    {"type":"selector","tag":"select","outbounds":["Hysteria2","auto"],"default":"Hysteria2"},
    {"type":"direct","tag":"direct"}
  ]
}'

AMBIGUOUS_SUB='{
  "outbounds": [
    {"type":"vless","tag":"Reality-A","tls":{"enabled":true,"reality":{"enabled":true}}},
    {"type":"vless","tag":"Reality-B","tls":{"enabled":true,"reality":{"enabled":true}}},
    {"type":"hysteria2","tag":"Hysteria2","tls":{"enabled":true}}
  ]
}'

# Defense-in-depth case: a hypothetical/malformed renderer output that
# reuses a reserved tag for a real endpoint must still be rejected, not
# silently benchmarked.
RESERVED_TAG_SUB='{
  "outbounds": [
    {"type":"hysteria2","tag":"auto","tls":{"enabled":true}}
  ]
}'

# A non-REALITY vless outbound (hypothetical future case) must NOT match
# the vless-reality filter.
PLAIN_VLESS_SUB='{
  "outbounds": [
    {"type":"vless","tag":"PlainVless","tls":{"enabled":true}},
    {"type":"hysteria2","tag":"Hysteria2","tls":{"enabled":true}}
  ]
}'

echo "--- vpn_benchmark_discover_outbound_tag ---"

tag="$(vpn_benchmark_discover_outbound_tag "$STANDARD_SUB" vless-reality)"
assert_eq "standard sub: vless-reality tag" "Reality" "$tag"

tag="$(vpn_benchmark_discover_outbound_tag "$STANDARD_SUB" hysteria2)"
assert_eq "standard sub: hysteria2 tag" "Hysteria2" "$tag"

tag="$(vpn_benchmark_discover_outbound_tag "$RENAMED_SUB" vless-reality)"
assert_eq "operator-renamed labels: vless-reality tag" "Germany - Fast Reality Node" "$tag"

tag="$(vpn_benchmark_discover_outbound_tag "$RENAMED_SUB" hysteria2)"
assert_eq "operator-renamed labels: hysteria2 tag" "Germany - QUIC Node" "$tag"

set +e
tag="$(vpn_benchmark_discover_outbound_tag "$HYSTERIA_ONLY_SUB" vless-reality 2>/dev/null)"
rc=$?
set -e
assert_eq "hysteria2-only deployment: vless-reality is a clean SKIP (rc=2)" "2" "$rc"
assert_eq "hysteria2-only deployment: no tag printed on SKIP" "" "$tag"

set +e
tag="$(vpn_benchmark_discover_outbound_tag "$AMBIGUOUS_SUB" vless-reality 2>/dev/null)"
rc=$?
set -e
assert_eq "ambiguous (2 REALITY outbounds): hard failure (rc=3)" "3" "$rc"
assert_eq "ambiguous: no tag printed" "" "$tag"

set +e
tag="$(vpn_benchmark_discover_outbound_tag "not valid json" vless-reality 2>/dev/null)"
rc=$?
set -e
assert_eq "invalid JSON: rc=5" "5" "$rc"
assert_eq "invalid JSON: no tag printed" "" "$tag"

set +e
tag="$(vpn_benchmark_discover_outbound_tag "$STANDARD_SUB" not-a-real-transport 2>/dev/null)"
rc=$?
set -e
assert_eq "unknown transport arg: rc=4" "4" "$rc"

set +e
tag="$(vpn_benchmark_discover_outbound_tag "$RESERVED_TAG_SUB" hysteria2 2>/dev/null)"
rc=$?
set -e
assert_eq "reserved tag 'auto' reused for a real endpoint: rejected (rc=5)" "5" "$rc"
assert_eq "reserved tag case: no tag printed" "" "$tag"

set +e
tag="$(vpn_benchmark_discover_outbound_tag "$PLAIN_VLESS_SUB" vless-reality 2>/dev/null)"
rc=$?
set -e
assert_eq "plain (non-REALITY) vless outbound: not matched as vless-reality (rc=2)" "2" "$rc"

# Never matches the selector/urltest/direct outbounds present in a
# standard subscription — confirms this by construction, not by
# checking a hardcoded exclusion list.
tag="$(vpn_benchmark_discover_outbound_tag "$STANDARD_SUB" vless-reality)"
case "$tag" in
  select|auto|direct)
    echo "FAIL: discovered tag '$tag' is a selector/urltest/direct outbound, not a real endpoint"
    failures=$((failures + 1))
    ;;
  *) echo "ok: discovered tag '$tag' is not a selector/urltest/direct outbound" ;;
esac

echo
if [ "$failures" -eq 0 ]; then
  echo "all vpn-benchmark-lib tests passed"
  exit 0
else
  echo "$failures test(s) FAILED"
  exit 1
fi
