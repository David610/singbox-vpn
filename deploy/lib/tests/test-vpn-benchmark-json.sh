#!/usr/bin/env bash
# Functional tests for deploy/lib/vpn-benchmark.sh's --json, --quick,
# --output, and --compare additions. Runs the real script (not an
# extracted fragment) with --skip-tunnel so it needs no root/production
# deployment — everything exercised here (host/kernel-tuning/raw-
# throughput/UDP-error sections) works on a plain unprivileged host.
# Network access is required for the raw-throughput download; if it's
# unavailable this test still passes (the script degrades to
# "unavailable" rather than failing, and this test only checks JSON
# validity/shape + the flags' own contracts, not specific numbers).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/deploy/lib/vpn-benchmark.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

if ! command -v jq >/dev/null 2>&1; then
  echo "SKIPPED: jq not installed — test-vpn-benchmark-json.sh needs it (same as the feature it tests)"
  exit 0
fi

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

echo "--- --json --quick --skip-tunnel produces valid, well-shaped JSON ---"
out="$("$SCRIPT" --json --quick --skip-tunnel --target-host 127.0.0.1 2>"$TMPDIR_TEST/stderr")"
if echo "$out" | jq . >/dev/null 2>&1; then
  ok "--json output is valid JSON"
else
  fail "--json output is not valid JSON:
$out"
fi
if echo "$out" | jq -e '.schema_version == 1' >/dev/null 2>&1; then
  ok "JSON has schema_version: 1"
else
  fail "JSON missing/wrong schema_version"
fi
if echo "$out" | jq -e '.quick_mode == true' >/dev/null 2>&1; then
  ok "--quick sets quick_mode: true in JSON"
else
  fail "--quick did not set quick_mode: true"
fi
if echo "$out" | jq -e '.rmem_max | type == "number"' >/dev/null 2>&1; then
  ok "rmem_max is a JSON number, not a string"
else
  fail "rmem_max is not a JSON number — the numeric-string-to-number pass may be broken:
$(echo "$out" | jq '.rmem_max')"
fi
if echo "$out" | jq -e 'has("tcp_congestion_control") and has("default_qdisc") and has("wmem_max")' >/dev/null 2>&1; then
  ok "effective sysctl/qdisc keys are present"
else
  fail "effective sysctl/qdisc keys missing from JSON:
$out"
fi
if echo "$out" | jq -e 'has("udp_rcvbuf_errors_delta") and has("udp_sndbuf_errors_delta")' >/dev/null 2>&1; then
  ok "UDP RcvbufErrors/SndbufErrors delta keys are present"
else
  fail "UDP buffer error delta keys missing from JSON:
$out"
fi
# --skip-tunnel means no throwaway user is created — the JSON must not
# claim it has real tunnel measurements it didn't take.
if echo "$out" | jq -e 'has("vless-reality_mbps_min") or has("hysteria2_mbps_min")' >/dev/null 2>&1; then
  fail "--skip-tunnel run reported tunnel throughput numbers it never measured"
else
  ok "--skip-tunnel run reports no fabricated tunnel throughput numbers"
fi

echo
echo "--- --json output round-trips through jq's regex numeric-conversion pass safely ---"
# A value that LOOKS numeric-ish but isn't a bare number (e.g. the RTT
# summary "min/avg/max/mdev" line, or an "unavailable" string) must stay
# a JSON string, not get mangled into a number or throw a jq error.
# tcp_congestion_control (e.g. "bbr"/"cubic") is set unconditionally by
# the script regardless of the host's ping/network state, unlike
# ping_status (only set on ping's SKIPPED path — a real, reachable ping
# target sets packet_loss/rtt_* instead, so asserting on ping_status
# here would depend on whether the CI runner's loopback ping succeeds).
if echo "$out" | jq -e '.tcp_congestion_control | type == "string"' >/dev/null 2>&1; then
  ok "a non-numeric status string (tcp_congestion_control) stays a JSON string"
else
  fail "tcp_congestion_control was not a plain JSON string:
$(echo "$out" | jq '.tcp_congestion_control')"
fi

echo
echo "--- explicit --runs/--download-url override --quick's own defaults ---"
out_override="$("$SCRIPT" --json --quick --runs 2 --skip-tunnel --target-host 127.0.0.1 2>/dev/null)"
n="$(echo "$out_override" | jq -r '.raw_download_mbps_n // 0')"
if [ "$n" = "2" ] || [ "$n" = "0" ]; then
  # 2 = override honored and downloads succeeded; 0 = no network in this
  # sandbox (both raw_download attempts failed) — either is consistent
  # with "the override was honored", so both are acceptable here. What
  # would be a real failure is n=1 (== --quick's own unrequested default).
  ok "--runs 2 alongside --quick did not silently fall back to --quick's default of 1 (n=$n)"
else
  fail "expected raw_download_mbps_n to be 2 (override honored) or 0 (no network), got $n"
fi

echo
echo "--- --output PATH writes the same content shown on stdout ---"
outfile="$TMPDIR_TEST/report.json"
stdout_capture="$("$SCRIPT" --json --quick --skip-tunnel --target-host 127.0.0.1 --output "$outfile" 2>/dev/null)"
if [ -f "$outfile" ]; then
  ok "--output created $outfile"
else
  fail "--output did not create the file"
fi
if [ "$(cat "$outfile" 2>/dev/null)" = "$stdout_capture" ]; then
  ok "--output file content matches stdout"
else
  fail "--output file content diverges from stdout"
fi

echo
echo "--- --compare diffs two JSON files without Python or another new runtime ---"
a_json="$TMPDIR_TEST/a.json"
b_json="$TMPDIR_TEST/b.json"
echo '{"schema_version":1,"raw_download_mbps_median":10,"tcp_congestion_control":"cubic"}' > "$a_json"
echo '{"schema_version":1,"raw_download_mbps_median":25,"tcp_congestion_control":"bbr"}' > "$b_json"
cmp_out="$("$SCRIPT" --compare "$a_json" "$b_json")"
if echo "$cmp_out" | grep -q "raw_download_mbps_median" && echo "$cmp_out" | grep -q "15"; then
  ok "--compare computes the correct numeric delta (25-10=15) for a matched key"
else
  fail "--compare did not show the expected delta:
$cmp_out"
fi
if echo "$cmp_out" | grep -q "cubic" && echo "$cmp_out" | grep -q "bbr"; then
  ok "--compare shows both sides of a changed non-numeric key"
else
  fail "--compare did not show both non-numeric values:
$cmp_out"
fi

echo
echo "--- --compare on a missing file fails cleanly instead of crashing ---"
if "$SCRIPT" --compare "$TMPDIR_TEST/does-not-exist.json" "$b_json" >/dev/null 2>"$TMPDIR_TEST/missing.err"; then
  fail "--compare with a missing file should have exited nonzero"
else
  ok "--compare with a missing file exits nonzero"
fi
if grep -qi "cannot read" "$TMPDIR_TEST/missing.err"; then
  ok "--compare reports a clear error for a missing file"
else
  fail "--compare's error message for a missing file is unclear:
$(cat "$TMPDIR_TEST/missing.err")"
fi

echo
echo "--- text mode (no --json) is unaffected: still prints the prose report ---"
text_out="$("$SCRIPT" --quick --skip-tunnel --target-host 127.0.0.1 2>/dev/null)"
if echo "$text_out" | grep -q "^Host$" && echo "$text_out" | grep -q "Assessment"; then
  ok "default (non-JSON) mode still prints the prose sections"
else
  fail "default mode's prose report looks broken:
$text_out"
fi
if echo "$text_out" | jq . >/dev/null 2>&1; then
  fail "default (non-JSON) mode output parses as JSON — --json gating leaked into the default path"
else
  ok "default mode output is prose, not JSON"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "PASS: test-vpn-benchmark-json.sh"
  exit 0
else
  echo "FAIL: test-vpn-benchmark-json.sh ($failures failure(s))"
  exit 1
fi
