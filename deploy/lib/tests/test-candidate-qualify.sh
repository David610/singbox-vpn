#!/usr/bin/env bash
# Hermetic regression tests for deploy/lib/diag/candidate_qualify.sh (egress qualification).
#
# The candidate VPS is a directory on this machine: ssh/scp/ssh-keyscan are shims that call
# fixtures/qualify/fake_tool.py, and the browser probe is fixtures/qualify/fake_probe.py. Nothing
# touches the network, Google/YouTube or a real host, and the suite leaves no state behind.
#
# Each case pins one failure that used to make the tool lie or leak (see docs in
# deploy/lib/diag/README.md): missing evidence reported as REJECTED, silent trust of unknown host
# keys, an 8/9 pass, no cleanup on interruption, a hard dependency on /tmp/ips_union.json, a
# SOCKS tunnel used without verification, and fragile positional arguments.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TOOL="$ROOT/deploy/lib/diag/candidate_qualify.sh"
FIX="$ROOT/deploy/lib/tests/fixtures/qualify"

[ -f "$TOOL" ] || { echo "FAIL: $TOOL missing"; exit 1; }
command -v python3 >/dev/null || { echo "SKIP: python3 not available"; exit 0; }

T="$(mktemp -d)"
PIDS_TO_KILL=()
cleanup_suite() {
  local p
  for p in "${PIDS_TO_KILL[@]:-}"; do
    [ -n "$p" ] && kill "$p" 2>/dev/null || true
  done
  if [ -n "${QT_KEEP:-}" ]; then echo "kept: $T" >&2; else rm -rf "$T"; fi
}
trap cleanup_suite EXIT

BIN="$T/bin"
mkdir -p "$BIN"
for tool in ssh scp ssh-keyscan; do
  printf '#!/bin/sh\nexec python3 "%s/fake_tool.py" %s "$@"\n' "$FIX" "$tool" >"$BIN/$tool"
  chmod +x "$BIN/$tool"
done
ssh-keygen -q -t ed25519 -N '' -C hostkey -f "$T/hostkey"
HOST_FP="$(ssh-keygen -lf "$T/hostkey.pub" | awk '{print $2}')"
ssh-keygen -q -t ed25519 -N '' -C probekey -f "$T/client_key"
printf '["142.250.1.1","142.250.1.2","172.217.0.1"]' >"$T/ips.json"

FAILURES=0
PASSES=0
CASE=""
ok() { PASSES=$((PASSES + 1)); echo "  ok   - $1"; }
bad() { FAILURES=$((FAILURES + 1)); echo "  FAIL - $CASE: $1"; }
check() { # check "description" command...
  local d="$1"
  shift
  if "$@"; then ok "$d"; else bad "$d"; fi
}

# new_env NAME -> sets S (state), EV (evidence base)
new_env() {
  CASE="$1"
  S="$T/$1/state"
  EV="$T/$1/evidence"
  mkdir -p "$S" "$EV"
  echo "== $CASE"
}

# qualify ARGS... ; runs the tool; sets RC. stdout+stderr go to $T/$CASE/out.txt
qualify() {
  set +e
  env PATH="$BIN:$PATH" QT_STATE="$S" QT_HOSTKEY_PUB="$T/hostkey.pub" \
    QT_SCENARIO="${SCENARIO:-good}" QT_PROBE_MODE="${PROBE_MODE:-all_played}" \
    QUALIFY_PROBE_PYTHON=python3 QUALIFY_PROBE_SCRIPT="$FIX/fake_probe.py" \
    timeout 120 bash "$TOOL" "$@" >"$T/$CASE/out.txt" 2>&1
  RC=$?
  set -e
}

# run_ok [extra args] -> a fully specified, valid `run` invocation (1 round, short dwell)
run_args() {
  echo run --target root@192.0.2.10 --label cand --ssh-key "$T/client_key" \
    --host-key-sha256 "$HOST_FP" --evidence-dir "$EV" --ips-file "$T/ips.json" \
    --rounds 1 --dwell 10 --sustained-video aaaaaaaaaaa --sustained-dwell 12 --min-sustained-s 5 \
    --timeout 60
}

vdir() {
  local all
  all="$(find "$EV" -mindepth 1 -maxdepth 1 -type d)"
  echo "${all%%$'\n'*}"
}
vfield() {
  [ -s "$(vdir)/verdict.json" ] || { echo "<no verdict.json>"; return 0; }
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1]));print(d.get(sys.argv[2],""))' "$(vdir)/verdict.json" "$1"
}
reason_has() {
  [ -s "$(vdir)/verdict.json" ] || return 1
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if sys.argv[2] in [d.get("reason")]+list(d.get("reasons",[])) else 1)' "$(vdir)/verdict.json" "$1"
}
alive() { [ -n "$1" ] && kill -0 "$1" 2>/dev/null; }

assert_verdict() { # verdict rc reason
  check "exit code $2" test "$RC" -eq "$2"
  check "verdict.json says $1" test "$(vfield verdict)" = "$1"
  if [ -n "${3:-}" ]; then check "reason $3" reason_has "$3"; fi
}

# ---------------------------------------------------------------- usage / input validation
new_env usage_validation
# shellcheck disable=SC2046
for bad_args in \
  "run --label x --host-key-sha256 $HOST_FP" \
  "run --target root@192.0.2.10 --host-key-sha256 $HOST_FP" \
  "run --target root@192.0.2.10 --label x" \
  "run --target root@192.0.2.10 --label ../evil --host-key-sha256 $HOST_FP" \
  "run --target root@192.0.2.10 --label x --host-key-sha256 notafingerprint" \
  "run --target root@192.0.2.10 --label x --host-key-sha256 $HOST_FP --sustained-video short" \
  "run --target root@192.0.2.10 --label x --host-key-sha256 $HOST_FP --rounds zero" \
  "run --target root@192.0.2.10 --label x --host-key-sha256 $HOST_FP --timeout 0" \
  "run --target ;rm@host --label x --host-key-sha256 $HOST_FP" \
  "run --target root@192.0.2.10 --label x --host-key-sha256 $HOST_FP --no-such-flag"; do
  # shellcheck disable=SC2086
  qualify $bad_args
  check "usage error (64) for: $bad_args" test "$RC" -eq 64
done
check "usage errors made no ssh/scp/keyscan call" test ! -s "$S/calls.log"

new_env legacy_positional
# the legacy form `TARGET LABEL [KEY] [--sustained ID]` must not mistake a flag for the key
qualify root@192.0.2.10 cand --sustained aaaaaaaaaaa --host-key-sha256 "$HOST_FP" --evidence-dir "$EV" --ips-file "$T/ips.json" \
  --rounds 1 --dwell 10 --sustained-dwell 12 --min-sustained-s 5 --timeout 60
assert_verdict PASS 0
check "no key flag leaked as an identity file" bash -c '! grep -q -- "-i --sustained" "$1"' _ "$S/calls.log"

# ---------------------------------------------------------------- success path and hygiene
new_env happy_path
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict PASS 0
check "schema_version present" test "$(vfield schema_version)" = "1"
check "host identity recorded as verified" test "$(vfield host_identity)" = "verified"
check "ssh host-key checking is strict (never accept-new/no)" bash -c '! grep -Eq "StrictHostKeyChecking=(accept-new|no)" "$1" && grep -q "StrictHostKeyChecking=yes" "$1"' _ "$S/calls.log"
check "all steps share ONE multiplexed SSH connection (hosts rate-limit new connections, e.g. ufw LIMIT)" bash -c 'grep -q "ControlMaster=auto" "$1" && grep -q "ControlPath=" "$1" && [ "$(grep -c "^SSH " "$1")" -gt 0 ] && ! grep "^SSH " "$1" | grep -qv "ControlMaster=auto"' _ "$S/calls.log"
check "the multiplexed master is closed at the end (-O exit)" grep -q -- "-O exit" "$S/calls.log"
check "no reliance on /tmp/ips_union.json (IP union is built in the evidence dir)" test -s "$(vdir)/ips_union.json"
check "remote tempdir was created and removed" bash -c 'grep -q "mktemp" "$1" && grep -q "^REMOTE_RM " "$1" && [ -z "$(find "$2/remote" -mindepth 1 2>/dev/null)" ]' _ "$S/calls.log" "$S"
check "tunnel was opened and is gone" bash -c '[ -s "$1/tunnel.pid" ] && ! kill -0 "$(cat "$1/tunnel.pid")" 2>/dev/null' _ "$S"
check "probe went through the verified SOCKS proxy" grep -q '"proxy": "socks5://127.0.0.1:' "$S/probe.args"
check "evidence dir is private (0700)" test "$(stat -c %a "$(vdir)")" = "700"
check "verdict has no key path or secret material" bash -c '! grep -Eiq "client_key|BEGIN|password|token" "$1"' _ "$(vdir)/verdict.json"
check "verdict states its measured scope" bash -c 'grep -q "scope" "$1"' _ "$(vdir)/verdict.json"

new_env two_rounds
# shellcheck disable=SC2046
qualify $(run_args) --rounds 2 --round-gap 0
assert_verdict PASS 0
check "Shorts probe ran once per round plus once for the sustained check (3 runs)" test "$(wc -l <"$S/probe.args")" -eq 3

# ---------------------------------------------------------------- conclusive failures are FAIL
for spec in "refused:media_refused" "bot:bot_challenge" "frozen:stalled" "no_start:no_start" "not_short:not_a_short"; do
  mode="${spec%%:*}"
  want="${spec##*:}"
  new_env "fail_$mode"
  PROBE_MODE="$mode"
  # shellcheck disable=SC2046
  qualify $(run_args)
  if [ "$mode" = "not_short" ]; then
    # a page that left /shorts/ is not evidence either way
    assert_verdict INCONCLUSIVE 2 "$want"
  else
    assert_verdict FAIL 1 "$want"
  fi
  PROBE_MODE=all_played
done

new_env eight_of_nine_is_not_enough
PROBE_MODE=one_refused
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict FAIL 1 media_refused
check "records played count so the near-miss is visible" bash -c 'grep -q "\"played\": 8" "$1"' _ "$(vdir)/verdict.json"
PROBE_MODE=all_played

new_env sustained_stall_fails
PROBE_MODE=sustained_frozen
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict FAIL 1 stalled
PROBE_MODE=all_played

new_env navigation_error_is_not_a_refusal
PROBE_MODE=nav_error
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict INCONCLUSIVE 2 probe_error
PROBE_MODE=all_played

new_env tls_blocked
SCENARIO=tls_blocked
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict FAIL 1 tls_blocked
check "browser stage skipped once the network stage is conclusive" test ! -e "$S/probe.pid"
SCENARIO=good

# ---------------------------------------------------------------- missing/failed execution is INCONCLUSIVE
for spec in "crash:probe_failed" "truncated:incomplete_probe" "garbage:evidence_unreadable" "no_output:evidence_missing"; do
  mode="${spec%%:*}"
  want="${spec##*:}"
  new_env "inconclusive_$mode"
  PROBE_MODE="$mode"
  # shellcheck disable=SC2046
  qualify $(run_args)
  assert_verdict INCONCLUSIVE 2 "$want"
  PROBE_MODE=all_played
done

new_env sustained_not_run_cannot_pass
# shellcheck disable=SC2046
qualify run --target root@192.0.2.10 --label cand --ssh-key "$T/client_key" --host-key-sha256 "$HOST_FP" \
  --evidence-dir "$EV" --ips-file "$T/ips.json" --rounds 1 --dwell 10 --timeout 60
assert_verdict INCONCLUSIVE 2 sustained_not_run

new_env survey_crash
SCENARIO=survey_crash
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict INCONCLUSIVE 2 remote_stage_failed
SCENARIO=good

new_env no_egress_ip
SCENARIO=no_egress_ip
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict INCONCLUSIVE 2 egress_ip_unknown
SCENARIO=good

new_env tunnel_exits_elsewhere
SCENARIO=tunnel_wrong_ip
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict INCONCLUSIVE 2 tunnel_egress_mismatch
check "probe never ran through an unverified tunnel" test ! -e "$S/probe.pid"
SCENARIO=good

new_env missing_prerequisite
PROBE_MODE=selfcheck_fail
# shellcheck disable=SC2046
qualify $(run_args)
assert_verdict INCONCLUSIVE 2 prereq_missing
check "no connection attempted when prerequisites are missing" test ! -s "$S/calls.log"
PROBE_MODE=all_played

new_env check_subcommand
qualify check
check "check passes when prerequisites are present" test "$RC" -eq 0
PROBE_MODE=selfcheck_fail
qualify check
check "check exits 2 when the probe self-check fails" test "$RC" -eq 2
PROBE_MODE=all_played

# ---------------------------------------------------------------- host identity
new_env host_key_mismatch
# shellcheck disable=SC2046
qualify run --target root@192.0.2.10 --label cand --ssh-key "$T/client_key" \
  --host-key-sha256 "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" --evidence-dir "$EV" --rounds 1 --timeout 60
assert_verdict INCONCLUSIVE 2 host_key_mismatch
check "no login or file transfer attempted against an unverified host" bash -c '! grep -Eq "^(SSH|SCP) " "$1"' _ "$S/calls.log"

new_env host_key_unverified_cannot_pass
# shellcheck disable=SC2046
qualify run --target root@192.0.2.10 --label cand --ssh-key "$T/client_key" --accept-unverified-host-key \
  --evidence-dir "$EV" --ips-file "$T/ips.json" --rounds 1 --dwell 10 --sustained-video aaaaaaaaaaa --sustained-dwell 12 --min-sustained-s 5 --timeout 60
assert_verdict INCONCLUSIVE 2 host_identity_unverified
check "identity recorded as unverified" test "$(vfield host_identity)" = "unverified"

new_env known_hosts_file
printf '192.0.2.10 %s\n' "$(awk '{print $1" "$2}' "$T/hostkey.pub")" >"$T/known_hosts"
# shellcheck disable=SC2046
qualify run --target root@192.0.2.10 --label cand --ssh-key "$T/client_key" --known-hosts "$T/known_hosts" \
  --evidence-dir "$EV" --ips-file "$T/ips.json" --rounds 1 --dwell 10 --sustained-video aaaaaaaaaaa --sustained-dwell 12 --min-sustained-s 5 --timeout 60
assert_verdict PASS 0
check "pinned known_hosts file used with strict checking" grep -q "UserKnownHostsFile=$T/known_hosts" "$S/calls.log"

# ---------------------------------------------------------------- timeouts and interruption
new_env probe_timeout
PROBE_MODE=hang
# shellcheck disable=SC2046
qualify $(run_args) --probe-timeout 3
assert_verdict INCONCLUSIVE 2 probe_timeout
check "hung probe was started and then killed" bash -c '[ -s "$1/probe.pid" ] && ! kill -0 "$(tail -n 1 "$1/probe.pid")" 2>/dev/null' _ "$S"
check "tunnel was opened and then closed after the timeout" bash -c '[ -s "$1/tunnel.pid" ] && ! kill -0 "$(cat "$1/tunnel.pid")" 2>/dev/null' _ "$S"
PROBE_MODE=all_played

for sig in INT TERM; do
  new_env "interrupted_$sig"
  PROBE_MODE=hang
  (
    # shellcheck disable=SC2046
    env PATH="$BIN:$PATH" QT_STATE="$S" QT_HOSTKEY_PUB="$T/hostkey.pub" QT_SCENARIO=good QT_PROBE_MODE=hang \
      QUALIFY_PROBE_PYTHON=python3 QUALIFY_PROBE_SCRIPT="$FIX/fake_probe.py" \
      python3 -c 'import os,signal,sys; signal.signal(signal.SIGINT, signal.SIG_DFL); os.setsid(); os.execvp("bash", ["bash"] + sys.argv[1:])' "$TOOL" $(run_args) >"$T/$CASE/out.txt" 2>&1 &
    echo $! >"$S/tool.pid"
  )
  tool_pid="$(cat "$S/tool.pid")"
  PIDS_TO_KILL+=("$tool_pid")
  waited=0
  while [ ! -s "$S/probe.pid" ] && [ "$waited" -lt 100 ]; do sleep 0.2; waited=$((waited + 1)); done
  check "probe started before the interrupt" test -s "$S/probe.pid"
  kill "-$sig" "$tool_pid" 2>/dev/null || true
  waited=0
  while alive "$tool_pid" && [ "$waited" -lt 100 ]; do sleep 0.2; waited=$((waited + 1)); done
  check "tool exited after SIG$sig" bash -c '! kill -0 "$1" 2>/dev/null' _ "$tool_pid"
  RC=0
  check "verdict.json written on interruption" test -s "$(vdir)/verdict.json"
  check "verdict INCONCLUSIVE" test "$(vfield verdict)" = "INCONCLUSIVE"
  check "reason interrupted" reason_has interrupted
  check "probe process gone" bash -c '[ -s "$1/probe.pid" ] && ! kill -0 "$(tail -n 1 "$1/probe.pid")" 2>/dev/null' _ "$S"
  check "tunnel process gone" bash -c '[ -s "$1/tunnel.pid" ] && ! kill -0 "$(cat "$1/tunnel.pid")" 2>/dev/null' _ "$S"
  check "remote tempdir removed" bash -c 'grep -q "^REMOTE_RM " "$2" && [ -z "$(find "$1/remote" -mindepth 1 2>/dev/null)" ]' _ "$S" "$S/calls.log"
  PROBE_MODE=all_played
done

echo
if [ "$FAILURES" -ne 0 ]; then
  echo "FAIL: test-candidate-qualify.sh: $FAILURES failed, $PASSES passed"
  exit 1
fi
echo "PASS: test-candidate-qualify.sh ($PASSES checks)"
