#!/usr/bin/env bash
# Regression test for a `set -e` interaction bug in
# deploy/lib/vpn-benchmark.sh's tunnel_benchmark(): the outbound-
# discovery call used to be written as
#
#   out_tag="$(vpn_benchmark_discover_outbound_tag "$sub_json" "$transport")"
#   rc=$?
#
# Under `set -Eeuo pipefail` (which vpn-benchmark.sh uses), a failed
# command substitution inside a plain assignment terminates the WHOLE
# SCRIPT right there — `rc=$?` never runs, and neither does anything
# after it. `vpn_benchmark_discover_outbound_tag` deliberately returns
# 2 (no match — a normal, expected case for a deployment offering only
# one transport) and 3 (ambiguous) as ROUTINE outcomes, not just
# "something went catastrophically wrong" — so this bug would have
# killed the entire `vpn-benchmark` run on the very first deployment
# that only offers one transport, instead of cleanly skipping that
# layer and continuing.
#
# This test does NOT reimplement or paraphrase the fix — it extracts
# the REAL `tunnel_benchmark` function body verbatim from the actual
# shipped `deploy/lib/vpn-benchmark.sh` via `sed` and runs it in a real
# `bash -Eeuo pipefail` subprocess, so it fails against the pre-fix code
# and only passes once the real fix is in place. Each case runs in its
# own subprocess (not sourced into this test's own shell) specifically
# so a still-buggy `tunnel_benchmark` killing its OWN process doesn't
# also kill this test harness — that's what lets this test tell "the
# call site survived" apart from "the whole test run crashed too".
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
VPN_BENCHMARK_SH="$REPO_ROOT/deploy/lib/vpn-benchmark.sh"
LIB_DIR="$REPO_ROOT/deploy/lib"

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

FUNC_SRC="$(sed -n '/^tunnel_benchmark() {/,/^}/p' "$VPN_BENCHMARK_SH")"
if [ -z "$FUNC_SRC" ]; then
  echo "FAIL: could not extract tunnel_benchmark() from $VPN_BENCHMARK_SH — has it been renamed/restructured? This test needs updating, not silently skipping." >&2
  exit 1
fi
printf '%s\n' "$FUNC_SRC" > "$TMPDIR_TEST/tunnel_benchmark.inc"

# Stub vpn-admin: `user create --json` -> fixed create_json; anything
# else (in particular `user remove`, called by tunnel_benchmark's own
# cleanup trap) -> success no-op.
VPN_BIN_STUB="$TMPDIR_TEST/vpn-admin-stub"
cat > "$VPN_BIN_STUB" <<'STUB'
#!/usr/bin/env bash
case "$*" in
  *"user create"*) echo '{"id":"test-bench-id","subscription_url":"https://sub.example.com/sub/test-token"}' ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$VPN_BIN_STUB"

# Stub sing-box: `check` always fails on purpose. A run that reaches it
# has, by definition, already gotten past outbound discovery — this
# test is only about whether the discovery call site survives `set -e`,
# not about end-to-end tunnel behavior (covered by the real interop
# tests elsewhere), so stopping cleanly right after a successful
# discovery is exactly what's needed and nothing more.
SINGBOX_BIN_STUB="$TMPDIR_TEST/sing-box-stub"
cat > "$SINGBOX_BIN_STUB" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = "check" ]; then
  echo "stub sing-box: deliberately rejecting (test doesn't need a live tunnel)" >&2
  exit 1
fi
exit 0
STUB
chmod +x "$SINGBOX_BIN_STUB"

CONFIG_STUB="$TMPDIR_TEST/deployment.toml"
cat > "$CONFIG_STUB" <<'EOF'
[subscription]
listen_port = 9100
EOF

# Builds and runs one case in an isolated subprocess. $1=case name,
# $2=transport to request, $3=canned subscription JSON `curl` should
# return for the local subscription-backend fetch.
run_case() {
  local case_name="$1" transport="$2" sub_json="$3"
  local harness="$TMPDIR_TEST/harness-$case_name.sh"
  cat > "$harness" <<HARNESS
#!/usr/bin/env bash
set -Eeuo pipefail
LIB_DIR="$LIB_DIR"
# shellcheck source=deploy/lib/vpn-benchmark-lib.sh
. "\$LIB_DIR/vpn-benchmark-lib.sh"

kv() { printf '%-28s %s\n' "\$1:" "\$2"; }
section() { :; }
have() { command -v "\$1" >/dev/null 2>&1; }
sample_min_median_max() { echo "unused"; }

VPN_BIN="$VPN_BIN_STUB"
SINGBOX_BIN="$SINGBOX_BIN_STUB"
CONFIG="$CONFIG_STUB"
DOWNLOAD_URL="https://example.invalid/unused"
RUNS=1
SKIP_TUNNEL=0

curl() {
  case "\$*" in
    *"/sub/"*) echo '$sub_json' ;;
    *) command curl "\$@" ;;
  esac
}

. "$TMPDIR_TEST/tunnel_benchmark.inc"

tunnel_benchmark "$transport" "test-label"
echo "PARENT_SURVIVED_AFTER_CALL rc=\$?"
HARNESS
  bash "$harness" 2>&1
}

failures=0
assert_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -qF "$needle"; then
    echo "ok: $desc"
  else
    echo "FAIL: $desc — expected to find [$needle] in:"
    echo "$haystack" | sed 's/^/    /'
    failures=$((failures + 1))
  fi
}
assert_not_contains() {
  local desc="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -qF "$needle"; then
    echo "FAIL: $desc — did not expect to find [$needle] in:"
    echo "$haystack" | sed 's/^/    /'
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

echo "--- Case 1: no matching transport (discovery rc=2) must not kill the calling script ---"
out1="$(run_case no-match vless-reality '{"outbounds":[{"type":"hysteria2","tag":"Hysteria2","tls":{"enabled":true}}]}')"
assert_contains "case 1: caller survives past the tunnel_benchmark call" "$out1" "PARENT_SURVIVED_AFTER_CALL"
assert_contains "case 1: reaches the SKIPPED branch (no vless-reality outbound present)" "$out1" "SKIPPED: no vless-reality outbound"

echo
echo "--- Case 2: ambiguous match (discovery rc=3) must not kill the calling script ---"
ambiguous_json='{"outbounds":[
  {"type":"vless","tag":"Reality-A","tls":{"enabled":true,"reality":{"enabled":true}}},
  {"type":"vless","tag":"Reality-B","tls":{"enabled":true,"reality":{"enabled":true}}}
]}'
out2="$(run_case ambiguous vless-reality "$ambiguous_json")"
assert_contains "case 2: caller survives past the tunnel_benchmark call" "$out2" "PARENT_SURVIVED_AFTER_CALL"
assert_contains "case 2: reaches the FAILED branch (refuses to guess between 2 REALITY outbounds)" "$out2" "FAILED: could not identify the vless-reality outbound"

echo
echo "--- Case 3: exactly one match (discovery rc=0) still discovers and uses the correct tag ---"
standard_json='{"outbounds":[
  {"type":"vless","tag":"Reality","tls":{"enabled":true,"reality":{"enabled":true}}},
  {"type":"hysteria2","tag":"Hysteria2","tls":{"enabled":true}}
]}'
out3="$(run_case success vless-reality "$standard_json")"
assert_contains "case 3: caller survives past the tunnel_benchmark call" "$out3" "PARENT_SURVIVED_AFTER_CALL"
# Discovery succeeded and got as far as generating+validating the
# client config against the (deliberately failing) sing-box stub — this
# is only reachable if $out_tag was actually resolved to "Reality" and
# used to build the client config, i.e. discovery's rc=0 path worked.
assert_contains "case 3: got past discovery to the sing-box check step (proves the correct tag was resolved and used)" "$out3" "FAILED: sing-box check rejected"
assert_not_contains "case 3: does NOT report a SKIP (a real match was found)" "$out3" "SKIPPED: no vless-reality"

echo
if [ "$failures" -eq 0 ]; then
  echo "all vpn-benchmark set -e regression tests passed"
  exit 0
else
  echo "$failures test(s) FAILED"
  exit 1
fi
