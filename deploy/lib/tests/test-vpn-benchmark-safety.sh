#!/usr/bin/env bash
# Safety contract of deploy/lib/vpn-benchmark.sh's protocol layers:
#
#   1. A default run (no --production-listener) never creates or removes a
#      user, so it cannot re-render the config or restart sing-box.
#   2. --production-listener creates its temporary user WITH a short expiry
#      and removes it when the layer finishes.
#   3. A run interrupted by SIGTERM while that user exists still removes it.
#   4. A run whose disposable instance cannot start reports FAILED/SKIPPED,
#      never a fabricated throughput number.
#
# Runs the REAL script with stub `vpn`, `sing-box` and `curl` executables
# placed first on PATH; the stubs only record their arguments. No root, no
# network, no sing-box binary, no deployment needed.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/deploy/lib/vpn-benchmark.sh"

if ! command -v jq >/dev/null 2>&1; then
  echo "SKIPPED: jq not installed — vpn-benchmark.sh needs it"
  exit 0
fi

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin"
LOG="$T/calls.log"

cat >"$T/bin/vpn" <<STUB
#!/usr/bin/env bash
echo "vpn \$*" >>"$LOG"
case "\$*" in
  *"user create"*) echo '{"id":"bench-user-id","subscription_url":"https://sub.example.invalid/sub/tok?format=hiddify"}' ;;
esac
exit 0
STUB

# sing-box stub: key generation fails (so the disposable instance cannot
# start), `check` behaves as \$STUB_CHECK says.
cat >"$T/bin/sing-box" <<STUB
#!/usr/bin/env bash
echo "sing-box \$*" >>"$LOG"
case "\$1" in
  generate) exit 1 ;;
  check)
    # bash runs a trap only after the current foreground command ends, so
    # keep this short: SIGTERM lands while it runs, cleanup follows it.
    if [ "\${STUB_CHECK:-fail}" = sleep ]; then sleep 3; fi
    exit 1 ;;
esac
exit 0
STUB

# curl stub: the local subscription fetch returns a one-transport document;
# every other request fails fast (no network needed).
cat >"$T/bin/curl" <<'STUB'
#!/usr/bin/env bash
case "$*" in
  *"/sub/"*) echo '{"outbounds":[{"type":"hysteria2","tag":"Hysteria2","tls":{"enabled":true}},{"type":"vless","tag":"Reality","tls":{"enabled":true,"reality":{"enabled":true}}}]}' ;;
  *) exit 7 ;;
esac
STUB
chmod +x "$T/bin/vpn" "$T/bin/sing-box" "$T/bin/curl"

cat >"$T/deployment.toml" <<'EOF'
[reality]
listen_port = 443
handshake_server = "decoy.example.invalid"

[subscription]
listen_port = 9100
EOF

run_bench() {
  PATH="$T/bin:$PATH" bash "$SCRIPT" --config "$T/deployment.toml" --quick \
    --target-host 127.0.0.1 "$@"
}

echo "--- 0. every tunnel download ignores NO_PROXY ---"
# curl applies NO_PROXY/no_proxy to SOCKS proxies too; a host listing the
# destination there would turn a "tunnel" sample into a direct download.
socks_lines="$(grep -c -- '--socks5-hostname' "$SCRIPT" || true)"
guarded="$(grep -- '--socks5-hostname' "$SCRIPT" | grep -c -- "--noproxy ''" || true)"
if [ "$socks_lines" -ge 2 ] && [ "$socks_lines" = "$guarded" ]; then
  ok "all $socks_lines SOCKS downloads pass --noproxy ''"
else
  fail "$guarded of $socks_lines SOCKS downloads pass --noproxy ''"
fi

echo
echo "--- 1. default run is non-mutating ---"
: >"$LOG"
out="$(run_bench --json 2>"$T/err1")" || true
if grep -q 'user create\|user remove' "$LOG"; then
  fail "default run invoked a mutating vpn command:
$(grep 'vpn ' "$LOG")"
else
  ok "default run never calls 'vpn user create/remove'"
fi
if echo "$out" | jq -e '.production_listener_status | test("SKIPPED")' >/dev/null 2>&1; then
  ok "default run reports the live-listener layer as SKIPPED"
else
  fail "default run did not report the live-listener layer as skipped:
$out"
fi
if echo "$out" | jq -e 'to_entries | map(select(.key | test("mbps_median$"))) | map(select(.key | test("^raw_") | not)) | length == 0' >/dev/null 2>&1; then
  ok "no tunnel throughput number is reported when the disposable instance cannot start"
else
  fail "a tunnel throughput number was reported without a working instance:
$out"
fi
if grep -q 'sing-box generate' "$LOG"; then
  ok "the default protocol layer uses the disposable instance (generates throwaway keys)"
else
  fail "the default protocol layer never tried to build the disposable instance"
fi

echo
echo "--- 2. --production-listener creates an expiring user and removes it ---"
: >"$LOG"
before="$(date +%s)"
run_bench --production-listener >"$T/out2" 2>&1 || true
after="$(date +%s)"
create_line="$(grep 'user create' "$LOG" | head -1 || true)"
if [ -n "$create_line" ]; then
  ok "--production-listener creates a temporary user"
else
  fail "--production-listener did not create a user"
fi
exp="$(printf '%s\n' "$create_line" | grep -oE -- '--expires-at [0-9]+' | awk '{print $2}' || true)"
if [ -n "$exp" ] && [ "$exp" -gt "$before" ] && [ "$exp" -le $((after + 1800 + 5)) ]; then
  ok "temporary user expires within the benchmark TTL (expires_at=$exp)"
else
  fail "temporary user has no bounded expiry: [$create_line]"
fi
creates="$(grep -c 'user create' "$LOG" || true)"
removes="$(grep -c 'user remove bench-user-id' "$LOG" || true)"
if [ "$creates" -ge 1 ] && [ "$creates" = "$removes" ]; then
  ok "every temporary user is removed ($creates created, $removes removed)"
else
  fail "temporary users not cleaned up ($creates created, $removes removed)"
fi
if grep -q 'MUTATING' "$T/out2"; then
  ok "the mutating layer says so in its output"
else
  fail "the mutating layer did not warn that it restarts sing-box"
fi

echo
echo "--- 3. SIGTERM while the temporary user exists still removes it ---"
: >"$LOG"
STUB_CHECK=sleep PATH="$T/bin:$PATH" bash "$SCRIPT" --config "$T/deployment.toml" --quick \
  --target-host 127.0.0.1 --production-listener >"$T/out3" 2>&1 &
pid=$!
for _ in $(seq 1 150); do
  grep -q 'sing-box check' "$LOG" 2>/dev/null && grep -q 'user create' "$LOG" && break
  sleep 0.2
done
kill -TERM "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
if grep -q 'user create' "$LOG"; then
  if grep -q 'user remove bench-user-id' "$LOG"; then
    ok "an interrupted run removed its temporary user"
  else
    fail "an interrupted run left its temporary user behind:
$(cat "$LOG")"
  fi
  if [ "$(grep -c 'user create' "$LOG")" = 1 ]; then
    ok "the interrupted run stopped instead of creating another user"
  else
    fail "the run continued after SIGTERM and created more users:
$(cat "$LOG")"
  fi
else
  fail "test setup: the run never reached user creation"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "all vpn-benchmark safety tests passed"
  exit 0
fi
echo "$failures test(s) FAILED"
exit 1
