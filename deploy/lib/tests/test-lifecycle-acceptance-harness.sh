#!/usr/bin/env bash
# Regression tests for deploy/almalinux/lifecycle-acceptance.sh's OWN
# logic — not the real destructive lifecycle (that needs a disposable
# AlmaLinux 9 host and is never run automatically). These tests run the
# real script with a mocked `ssh` binary on PATH that records every
# invocation's argv and command string, so assertions are made against
# actual recorded behavior, not against comments/grep of the source.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/deploy/almalinux/lifecycle-acceptance.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

[ -x "$SCRIPT" ] || fail "lifecycle-acceptance.sh is missing or not executable"

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

MOCKBIN="$TMPDIR_TEST/mockbin"
mkdir -p "$MOCKBIN"
SSH_LOG="$TMPDIR_TEST/ssh.log"

# A mock ssh that: records every argv (one call per line, tab-separated),
# always succeeds for cheap probe commands, and simulates realistic
# outputs for the specific remote commands the harness depends on so the
# whole script can run start-to-finish in a few seconds with no network
# and no real target.
cat > "$MOCKBIN/ssh" <<'MOCKSSH'
#!/bin/bash
{
  printf '%s\t' "$@"
  echo
} >> "$SSH_LOG"
cmd="${*: -1}"
STATE_FILE="$TMPDIR_TEST/singbox_state"
singbox_state() { cat "$STATE_FILE" 2>/dev/null || echo active; }
case "$cmd" in
  true) exit 0 ;;
  # Stage 0a's existing-installation guard: matched BEFORE any broader
  # pattern below that also happens to contain one of these path
  # substrings (e.g. "*install-state.json*", used generically further
  # down for a different remote call) — case matches top-to-bottom, so
  # this exact-command check has to come first or it would silently fall
  # through to an unrelated mock response. A clean disposable target has
  # none of these four markers by default; the "existing installation"
  # test scenario flips this by touching $TMPDIR_TEST/mock_existing_install.
  *"[ -e '/etc/vpn/deployment.toml' ]"* | \
  *"[ -e '/var/lib/singbox-vpn/install-state.json' ]"* | \
  *"[ -e '/var/lib/singbox-vpn/ownership.env' ]"* | \
  *"[ -e '/opt/singbox-vpn' ]"*)
    [ -f "$TMPDIR_TEST/mock_existing_install" ] && exit 0 || exit 1 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *:*grep*) echo 1; exit 0 ;;
  # The burst-crash-until-failed compound command (stage 13b): a real
  # multi-line script containing "systemctl show -p MainPID --value
  # sing-box" and "systemctl is-failed --quiet sing-box" as its LAST
  # command — matched here, BEFORE the generic MainPID/is-failed
  # patterns below, specifically so this one opaque invocation can flip
  # the shared state file to "failed" and report success (real
  # is-failed's contract: exit 0 means "yes, it is failed"). Matched on
  # "last_killed" — the loop variable that tracks the most recently
  # killed PID so it polls for MainPID actually changing instead of
  # assuming a fixed sleep — rather than an iteration-count/timing
  # literal, so this hook doesn't silently stop firing the next time the
  # real script's crash-loop timing is retuned.
  *"last_killed"*)
    echo failed > "$STATE_FILE"
    exit 0 ;;
  *"systemctl start vpn-service-watchdog.service"*)
    # Simulates the real watchdog script: only acts on a FAILED unit.
    [ "$(singbox_state)" = "failed" ] && echo active > "$STATE_FILE"
    exit 0 ;;
  *"systemctl stop sing-box"*)
    echo stopped > "$STATE_FILE"
    exit 0 ;;
  *"systemctl start sing-box"*)
    echo active > "$STATE_FILE"
    exit 0 ;;
  *"is-active --quiet sing-box"*)
    [ "$(singbox_state)" = "active" ] && exit 0 || exit 1 ;;
  *"is-failed --quiet sing-box"*)
    [ "$(singbox_state)" = "failed" ] && exit 0 || exit 1 ;;
  *MainPID*sing-box*)
    counter_file="$TMPDIR_TEST/mainpid_counter"
    n=0
    [ -f "$counter_file" ] && n="$(cat "$counter_file")"
    n=$((n + 1))
    echo "$n" > "$counter_file"
    echo "$((1000 + n))"
    exit 0 ;;
  *systemctl\ is-active*sshd*) exit 0 ;;
  *systemctl\ is-active*) exit 0 ;;
  *health-check.sh*) exit 0 ;;
  *ss\ -ltn*) echo ':443 LISTEN'; exit 0 ;;
  *list-timers*) echo 'singbox-vpn-cert-renew.timer'; exit 0 ;;
  *install-state.json*) echo '{"singbox_vpn_version":"mock"}'; exit 0 ;;
  *vpn-benchmark.sh*)
    cat <<'BENCH'
Hysteria2 protocol/server-side overhead (sing-box client on THIS VPS -> THIS VPS's public IP; NOT a remote-client network-path measurement)
--------------------------------------------------------------------------------------------------------------------------------------------
  throughput (Mbps), 1 run(s):
    min=42.00 median=42.00 max=42.00 (n=1)
Assessment
BENCH
    exit 0 ;;
  *doctor\ --protocol*)
    echo 'protocol self-test: a throwaway sing-box client using the CURRENT REALITY public_key/short_id and an active VLESS user completed a full handshake through 127.0.0.1:443 and returned application bytes end-to-end'
    exit 0 ;;
  *"vpn-admin doctor"*)
    # Bare `doctor` (no --protocol) — stateful so stage 13b's
    # during-failure check has something real to observe.
    if [ "$(singbox_state)" = "failed" ]; then
      echo "[FAIL] [L1  ] sing-box.service is in a FAILED state (restart budget exhausted — see StartLimitBurst in the unit file); vpn-service-watchdog.timer will retry it periodically"
      exit 1
    fi
    exit 0 ;;
  *"vpn-admin status"*)
    if [ "$(singbox_state)" = "failed" ]; then
      echo "sing-box              failed"
    else
      echo "sing-box              active"
    fi
    exit 0 ;;
  *acceptance-test.sh*) exit 0 ;;
  *systemctl\ reboot*) exit 0 ;;
  *sudo\ systemctl\ reboot*) exit 0 ;;
  *SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox*) exit 1 ;;
  *singbox-vpn-uninstall\ --yes*) echo 'uninstalled'; exit 0 ;;
  *iptables*) exit 0 ;;
  *vpn-admin\ user\ list*) echo 'mock-id-1 lifecycle-test-user yes'; exit 0 ;;
  *vpn-admin\ user*) exit 0 ;;
  *install.sh*) exit 0 ;;
  *SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch*update.sh*) exit 1 ;;
  *update.sh*) exit 0 ;;
  *certbot\ renew*) exit 0 ;;
  *"[ -e /opt/singbox-vpn ] || [ -e /etc/vpn ]"*) exit 0 ;;
  *"[ ! -e /etc/vpn ]"*) exit 0 ;;
  *"command -v curl"*)
    [ -f "$TMPDIR_TEST/mock_missing_curl" ] && exit 1 || exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH
chmod +x "$MOCKBIN/ssh"
export SSH_LOG TMPDIR_TEST

cat > "$MOCKBIN/sleep" <<'MOCKSLEEP'
#!/bin/bash
exit 0
MOCKSLEEP
chmod +x "$MOCKBIN/sleep"

run_harness() {
  : > "$SSH_LOG"
  rm -f "$TMPDIR_TEST/singbox_state" "$TMPDIR_TEST/mainpid_counter"
  PATH="$MOCKBIN:$PATH" "$SCRIPT" "$@"
}

echo "--- destructive opt-in is required ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && grep -qi 'i-understand-this-is-destructive' <<< "$out"; then
  ok "refuses to run without --i-understand-this-is-destructive"
else
  fail "did not refuse without destructive opt-in (rc=$rc)"
fi
[ ! -s "$SSH_LOG" ] && ok "no ssh calls made without destructive opt-in" || fail "ssh was invoked before the destructive opt-in gate"

echo
echo "--- --host is required ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --i-understand-this-is-destructive 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && grep -qi -- '--host is required' <<< "$out" \
  && ok "refuses to run without --host" || fail "did not refuse without --host"

echo
echo "--- localhost is refused even with destructive opt-in ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@localhost --i-understand-this-is-destructive 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && grep -qi 'localhost' <<< "$out" \
  && ok "refuses localhost target" || fail "did not refuse localhost target"

echo
echo "--- production host (SINGBOX_VPN_PRODUCTION_HOST) is refused ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" SINGBOX_VPN_PRODUCTION_HOST=prod.example.test "$SCRIPT" --host root@prod.example.test --i-understand-this-is-destructive 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && grep -qi 'SINGBOX_VPN_PRODUCTION_HOST' <<< "$out" \
  && ok "refuses SINGBOX_VPN_PRODUCTION_HOST target" || fail "did not refuse the configured production host"

echo
echo "--- non-numeric --ssh-port is rejected ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --ssh-port abc 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && grep -qi 'ssh-port must be numeric' <<< "$out" \
  && ok "rejects a non-numeric --ssh-port" || fail "did not reject a non-numeric --ssh-port"

echo
echo "--- malformed --version is rejected before SSH ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --version 'main;id' 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && grep -qi 'immutable vX.Y.Z release tag' <<< "$out" \
  && ok "rejects a mutable or shell-unsafe --version" || fail "did not reject malformed --version"

echo
echo "--- malicious --domain values are rejected before SSH, never reach the remote command string ---"
for bad_domain in 'example.com;touch /tmp/pwned' '$(touch /tmp/pwned)' 'foo`touch /tmp/pwned`' '"abc"' 'abc def' "abc'def"; do
  rc=0
  : > "$SSH_LOG"
  out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --domain "$bad_domain" 2>&1)" || rc=$?
  if [ "$rc" -ne 0 ] && grep -qi 'not a syntactically valid hostname\|contains characters that are never valid' <<< "$out"; then
    ok "rejects malicious --domain '$bad_domain' before any SSH call"
  else
    fail "did not reject malicious --domain '$bad_domain' (rc=$rc): $out"
  fi
  if [ -s "$SSH_LOG" ]; then
    fail "--domain '$bad_domain' reached ssh despite being rejected"
  fi
done

echo
echo "--- malicious --update-to-ref values are rejected before SSH ---"
for bad_ref in 'main;id' '$(id)' 'main`id`' '../../etc/passwd' '-rf' 'a b'; do
  rc=0
  : > "$SSH_LOG"
  out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --update-to-ref "$bad_ref" 2>&1)" || rc=$?
  if [ "$rc" -ne 0 ] && grep -qi 'not a syntactically valid git ref\|must not start with' <<< "$out"; then
    ok "rejects malicious --update-to-ref '$bad_ref' before any SSH call"
  else
    fail "did not reject malicious --update-to-ref '$bad_ref' (rc=$rc): $out"
  fi
  if [ -s "$SSH_LOG" ]; then
    fail "--update-to-ref '$bad_ref' reached ssh despite being rejected"
  fi
done

echo
echo "--- a valid --domain is threaded through to the remote install.sh command as one safely-quoted argument ---"
: > "$SSH_LOG"
set +e
run_harness --host root@disposable-test --i-understand-this-is-destructive --domain vpn.xn--p1aen4b.com --skip-reboot >"$TMPDIR_TEST/out-domain.log" 2>&1
set -e
install_line="$(grep -F 'install.sh' "$SSH_LOG" | grep -F 'curl' | head -1 || true)"
if printf '%s' "$install_line" | grep -qE -- '--domain vpn\.xn--p1aen4b\.com'; then
  ok "a valid IDN/punycode --domain reaches the remote install.sh invocation intact"
else
  fail "valid --domain vpn.xn--p1aen4b.com did not reach the remote install.sh command: $install_line"
fi

echo
echo "--- Stage 0a: a target with an existing singbox-vpn install refuses destruction without the second explicit override ---"
: > "$SSH_LOG"
: > "$TMPDIR_TEST/mock_existing_install"
set +e
out="$(run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1)"
rc=$?
set -e
if [ "$rc" -ne 0 ] && grep -qi 'already has an existing singbox-vpn installation' <<< "$out"; then
  ok "refuses to destroy a target with an existing singbox-vpn install (--i-understand-this-is-destructive alone is not enough)"
else
  fail "did not refuse a target with an existing singbox-vpn install (rc=$rc): $out"
fi
if grep -qE 'curl.*install\.sh.*\|.*bash' "$SSH_LOG"; then
  fail "the destructive install pipeline was invoked against a target with a detected pre-existing installation"
else
  ok "no destructive install is attempted against a target with a detected pre-existing installation, absent the second override"
fi

echo
echo "--- Stage 0a: the second override (--allow-destroy-existing-singbox-vpn-install) authorizes destruction of a detected existing install ---"
: > "$SSH_LOG"
set +e
out="$(run_harness --host root@disposable-test --i-understand-this-is-destructive --allow-destroy-existing-singbox-vpn-install --skip-reboot 2>&1)"
set -e
rm -f "$TMPDIR_TEST/mock_existing_install"
if grep -qi 'destruction explicitly authorized via --allow-destroy-existing-singbox-vpn-install' <<< "$out"; then
  ok "--allow-destroy-existing-singbox-vpn-install authorizes proceeding against a detected existing install"
else
  fail "the second override did not authorize proceeding: $out"
fi
if grep -qE 'curl.*install\.sh.*\|.*bash' "$SSH_LOG"; then
  ok "the destructive install pipeline runs once the second override is given"
else
  fail "the destructive install pipeline never ran even with the second override given"
fi

echo
echo "--- Stage 0b: missing remote curl is caught as ONE clear bootstrap-prerequisite failure, not a generic pipeline error, and blocks (not cascades into) dependent stages ---"
: > "$SSH_LOG"
: > "$TMPDIR_TEST/mock_missing_curl"
set +e
out="$(run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1)"
set -e
rm -f "$TMPDIR_TEST/mock_missing_curl"
if grep -qE '\[FAIL\]\[required\][[:space:]]+bootstrap prerequisites[[:space:]]+missing:.*curl' <<< "$out"; then
  ok "missing curl is reported as one clear '[FAIL][required] bootstrap prerequisites ... missing: curl' line"
else
  fail "missing curl was not reported as a clear bootstrap-prerequisite failure: $(grep -i 'bootstrap\|curl' <<< "$out" | head -5)"
fi
if grep -qE '\[BLOCKED\][[:space:]]+install\.sh \(clean\)' <<< "$out"; then
  ok "install.sh (clean) is reported BLOCKED, not attempted, once bootstrap prerequisites are known missing"
else
  fail "install.sh (clean) was not reported BLOCKED after a missing-curl bootstrap failure: $(grep -i 'install.sh (clean)' <<< "$out")"
fi
if grep -qE 'curl.*install\.sh.*\|.*bash' "$SSH_LOG"; then
  fail "the curl|bash install pipeline was invoked against a target already known to be missing curl"
else
  ok "the curl|bash install pipeline is never attempted once stage 0b knows curl is missing"
fi
required_fail_count="$(grep -cE '\[FAIL\]\[required\]' <<< "$out" || true)"
if [ "$required_fail_count" -le 2 ]; then
  ok "one root cause (missing curl) produces at most $required_fail_count [required] failure(s), not a cascade of ~20"
else
  fail "missing curl cascaded into $required_fail_count separate [required] failures instead of blocking dependents:"; grep -E '\[FAIL\]\[required\]' <<< "$out"
fi

echo
echo "--- full run: SSH port is not hardcoded to 22 ---"
set +e
run_harness --host root@disposable-test --i-understand-this-is-destructive --ssh-port 2222 >"$TMPDIR_TEST/out-2222.log" 2>&1
set -e
if grep -qP '(^|\t)-p\t2222(\t|$)' "$SSH_LOG"; then
  ok "ssh is invoked with -p 2222, not a hardcoded 22"
else
  fail "ssh was not invoked with the configured custom port"
fi
if grep -q ':2222' "$SSH_LOG"; then
  ok "SSH baseline stage checks the configured port, not a hardcoded :22"
else
  fail "SSH baseline stage did not reference the configured port"
fi
if grep -q -- '--ssh-port 2222' "$SSH_LOG"; then
  ok "install.sh is invoked with --ssh-port 2222 (threaded through, not silently dropped)"
else
  fail "install.sh invocation did not carry --ssh-port through to the target"
fi

if grep -q 'ACCEPTANCE CLASSIFICATION: DEVELOPMENT LIFECYCLE ONLY — NOT PRODUCTION ACCEPTANCE' "$TMPDIR_TEST/out-2222.log"; then
  ok "an unpinned branch run is explicitly classified as development-only"
else
  fail "an unpinned branch run could be mistaken for production acceptance"
fi

echo
echo "--- pinned release mode uses the stable checksum-verified bootstrap contract ---"
set +e
run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot --version v0.1.2 --update-to-version v0.1.3 >"$TMPDIR_TEST/out-version.log" 2>&1
set -e
if grep -q 'SINGBOX_VPN_VERSION=v0.1.2' "$SSH_LOG"; then
  ok "pinned release mode passes SINGBOX_VPN_VERSION to the remote installer"
else
  fail "pinned release mode did not pass SINGBOX_VPN_VERSION"
fi
if grep -q 'SINGBOX_VPN_CHANNEL=dev\|SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1' "$SSH_LOG"; then
  fail "pinned release mode incorrectly used development-source opt-ins"
else
  ok "pinned release mode never enables mutable development source"
fi
if grep -q 'acceptance scope: PRODUCTION RELEASE v0.1.2' "$TMPDIR_TEST/out-version.log"; then
  ok "pinned release mode identifies the exact production release under test"
else
  fail "pinned release mode did not identify its production scope"
fi
if grep -q 'update.sh --version v0.1.3' "$SSH_LOG" && grep -q 'update.sh --repair' "$SSH_LOG"; then
  ok "pinned release mode exercises production update and rollback paths"
else
  fail "pinned release mode did not exercise production update and rollback"
fi

# Restore the custom-port run log used by the remaining ordering assertions.
set +e
run_harness --host root@disposable-test --i-understand-this-is-destructive --ssh-port 2222 >"$TMPDIR_TEST/out-2222.log" 2>&1
set -e

echo
echo "--- test user is created and used for the REALITY/Hysteria2/recovery proofs ---"
if grep -q -- '--name lifecycle-test-user' "$SSH_LOG"; then
  ok "a persisted test user is created"
else
  fail "no persisted test user was created"
fi
if grep -q -- 'doctor --protocol --require-protocol' "$SSH_LOG"; then
  ok "doctor --protocol is invoked with --require-protocol (hard-fails instead of warning)"
else
  fail "doctor --protocol was not invoked with --require-protocol"
fi
create_line="$(grep -n -- '--name lifecycle-test-user' "$SSH_LOG" | head -1 | cut -d: -f1 || true)"
protocol_line="$(grep -n -- 'doctor --protocol --require-protocol' "$SSH_LOG" | head -1 | cut -d: -f1 || true)"
if [ -n "$create_line" ] && [ -n "$protocol_line" ] && [ "$create_line" -lt "$protocol_line" ]; then
  ok "the test user is created before the REALITY protocol proof runs"
else
  fail "the test user was not created before the REALITY protocol proof (ordering regression)"
fi

echo
echo "--- Hysteria2 real handshake+transfer proof reuses deploy/lib/vpn-benchmark.sh ---"
if grep -q 'vpn-benchmark.sh' "$SSH_LOG"; then
  ok "the Hysteria2 proof stage invokes deploy/lib/vpn-benchmark.sh"
else
  fail "no invocation of deploy/lib/vpn-benchmark.sh was found"
fi
if grep -A6 '=== 12. Hysteria2' "$TMPDIR_TEST/out-2222.log" | grep -q '\[PASS\]'; then
  ok "the Hysteria2 proof stage reports PASS against a healthy mocked transfer"
else
  fail "the Hysteria2 proof stage did not report PASS"
fi

echo
echo "--- SIGKILL recovery stage targets a real PID and proves the PID actually changed ---"
if grep -q -- 'sudo kill -9 1001' "$SSH_LOG"; then
  ok "sing-box is killed via its own MainPID (kill -9 <pid>), not a broad pkill"
else
  fail "sing-box was not killed via its captured MainPID"
fi
if [ "$(grep -c 'MainPID' "$SSH_LOG")" -ge 2 ]; then
  ok "MainPID is queried both before and after the kill (to prove a real respawn)"
else
  fail "MainPID was not queried both before and after the kill"
fi
if grep -A8 '=== 13. kill sing-box' "$TMPDIR_TEST/out-2222.log" | grep -q 'MainPID changed'; then
  ok "the recovery stage reports the MainPID actually changed"
else
  fail "the recovery stage did not report a MainPID change"
fi

echo
echo "--- StartLimitBurst exhaustion + vpn-service-watchdog recovery (stage 13b) ---"
stage13b_block="$(sed -n '/=== 13b\./,/=== 13c\./p' "$TMPDIR_TEST/out-2222.log")"
if printf '%s' "$stage13b_block" | grep -q 'reached FAILED state after exhausting StartLimitBurst'; then
  ok "the harness actually drives sing-box into a FAILED state, not just asserting one exists"
else
  fail "stage 13b did not report exhausting StartLimitBurst"
fi
if printf '%s' "$stage13b_block" | grep -q 'vpn-admin doctor correctly reports sing-box.service as FAILED'; then
  ok "vpn doctor is checked for the FAILED state during the outage, not just service state after recovery"
else
  fail "stage 13b did not check vpn-admin doctor during the outage"
fi
if printf '%s' "$stage13b_block" | grep -q 'vpn-admin status correctly reports sing-box as failed'; then
  ok "vpn status is also checked for the failed state during the outage"
else
  fail "stage 13b did not check vpn-admin status during the outage"
fi
if grep -q 'systemctl start vpn-service-watchdog.service' "$SSH_LOG"; then
  ok "the watchdog service is triggered directly to prove its recovery logic (not just waiting on its timer)"
else
  fail "stage 13b never invoked vpn-service-watchdog.service"
fi
if printf '%s' "$stage13b_block" | grep -q 'vpn-service-watchdog recovered sing-box.service from its parked FAILED state'; then
  ok "the harness proves the watchdog actually recovers a parked FAILED unit"
else
  fail "stage 13b did not prove watchdog recovery"
fi

echo
echo "--- systemctl stop still behaves normally, and the watchdog leaves a stopped unit alone (stage 13c) ---"
stage13c_block="$(sed -n '/=== 13c\./,/=== 14\./p' "$TMPDIR_TEST/out-2222.log")"
if printf '%s' "$stage13c_block" | grep -q "is inactive after systemctl stop"; then
  ok "a deliberate systemctl stop leaves sing-box inactive (not silently auto-restarted)"
else
  fail "stage 13c did not confirm sing-box went inactive after systemctl stop"
fi
if printf '%s' "$stage13c_block" | grep -q "'inactive', not 'failed', after a deliberate stop"; then
  ok "a deliberate stop is distinguished from a failure (inactive, never failed)"
else
  fail "stage 13c did not distinguish a deliberate stop from a failure"
fi
if printf '%s' "$stage13c_block" | grep -q "left the deliberately-stopped sing-box alone"; then
  ok "the watchdog is proven to never restart a deliberately-stopped unit"
else
  fail "stage 13c did not prove the watchdog leaves a stopped unit alone"
fi

echo
echo "--- backup is created, survives the destructive uninstall, and is restored afterward ---"
if grep -q -- 'vpn-admin backup --output /root/singbox-vpn-lifecycle-backup.tar' "$SSH_LOG"; then
  ok "vpn-admin backup is invoked with an explicit --output path outside singbox-vpn-managed trees"
else
  fail "vpn-admin backup was not invoked with the expected --output path"
fi
if grep -q -- 'vpn-admin restore /root/singbox-vpn-lifecycle-backup.tar' "$SSH_LOG"; then
  ok "vpn-admin restore is invoked against the backup created earlier in the run"
else
  fail "vpn-admin restore was not invoked against the earlier backup"
fi
backup_line="$(grep -n -- '17. create vpn backup' "$TMPDIR_TEST/out-2222.log" | head -1 | cut -d: -f1 || true)"
uninstall_line="$(grep -n -- '19. uninstall completely' "$TMPDIR_TEST/out-2222.log" | head -1 | cut -d: -f1 || true)"
restore_line="$(grep -n -- '22. restore backup' "$TMPDIR_TEST/out-2222.log" | head -1 | cut -d: -f1 || true)"
if [ -n "$backup_line" ] && [ -n "$uninstall_line" ] && [ -n "$restore_line" ] \
  && [ "$backup_line" -lt "$uninstall_line" ] && [ "$uninstall_line" -lt "$restore_line" ]; then
  ok "backup happens before the destructive uninstall, restore happens after reinstall"
else
  fail "backup/uninstall/restore stages are not in the expected order"
fi

echo
echo "--- a final uninstall + residue audit runs after the restore is verified ---"
if [ "$(grep -c -- 'singbox-vpn-uninstall --yes' "$SSH_LOG")" -ge 2 ]; then
  ok "singbox-vpn-uninstall runs at least twice (once before restore, once as the true final uninstall)"
else
  fail "singbox-vpn-uninstall did not run the expected number of times"
fi
final_uninstall_line="$(grep -n -- '25. final uninstall' "$TMPDIR_TEST/out-2222.log" | head -1 | cut -d: -f1 || true)"
residue_line="$(grep -n -- '27. final uninstall residue audit' "$TMPDIR_TEST/out-2222.log" | head -1 | cut -d: -f1 || true)"
if [ -n "$final_uninstall_line" ] && [ -n "$residue_line" ] && [ "$final_uninstall_line" -lt "$residue_line" ]; then
  ok "the residue audit runs after the final uninstall, not the interim one"
else
  fail "the residue audit did not run after the final uninstall"
fi

echo
echo "--- failure-injection env var reaches the bash process that execs install.sh, not curl ---"
if grep -qP 'curl[^\t]*\|\tsudo\tSINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox' "$SSH_LOG" \
  || grep -qE 'sudo SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox' "$SSH_LOG"; then
  ok "SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER is attached to the bash side of the curl|bash pipeline"
else
  fail "SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER was not found attached to the bash invocation"
fi
if grep -qP 'sudo\tSINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=install_singbox\tcurl' "$SSH_LOG"; then
  fail "regression: SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER is attached to curl's own exec again (the fixed bug reappeared)"
else
  ok "SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER is not mis-scoped to curl's exec"
fi

echo
echo "--- offline uninstall stage uses the local binary, never curl/GitHub ---"
if grep -q 'singbox-vpn-uninstall --yes' "$SSH_LOG"; then
  ok "offline uninstall stage invokes /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes"
else
  fail "offline uninstall stage did not invoke the local singbox-vpn-uninstall binary"
fi
if grep -A2 '=== 19. uninstall completely' "$TMPDIR_TEST/out-2222.log" | grep -qi 'uninstall.sh | bash'; then
  fail "stage 19's own uninstall call still uses curl | bash instead of the offline binary"
else
  ok "stage 19's uninstall call does not use curl | bash"
fi

echo
echo "--- update version-change assertion: a no-op update must not be reported as an update ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_NOOP'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
case "$cmd" in
  true) exit 0 ;;
  *"[ -e '/etc/vpn/deployment.toml' ]"* | \
  *"[ -e '/var/lib/singbox-vpn/install-state.json' ]"* | \
  *"[ -e '/var/lib/singbox-vpn/ownership.env' ]"* | \
  *"[ -e '/opt/singbox-vpn' ]"*)
    exit 1 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *"command -v "*) exit 0 ;;
  *install-state.json*) echo '{"singbox_vpn_version":"same"}'; exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH_NOOP
chmod +x "$MOCKBIN/ssh"
set +e
noop_out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --skip-reboot --update-to-ref main 2>&1)"
noop_rc=$?
set -e
if [ "$noop_rc" -ne 0 ] && grep -qi 'command succeeded but version-state did not change' <<< "$noop_out"; then
  ok "harness detects a no-op update (identical before/after version-state) and fails it"
else
  fail "harness did not detect a no-op update as a failure (rc=$noop_rc)"
fi

echo
echo "--- stage 27 residue audit: a baseline that was already dirty (pre-existing leftover state) must not fail a run that leaves the host CLEANER than it found it ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_RESIDUE'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
COUNTER_FILE="$TMPDIR_TEST/residue_call_count"
case "$cmd" in
  true) exit 0 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *"opt_singbox-vpn="*)
    n=0
    [ -f "$COUNTER_FILE" ] && n="$(cat "$COUNTER_FILE")"
    n=$((n + 1))
    echo "$n" > "$COUNTER_FILE"
    if [ "$n" -eq 1 ]; then
      # stage 1b (host baseline, captured BEFORE this run's own install):
      # dirty on purpose — simulates a target this run was explicitly
      # authorized to reuse (--allow-destroy-existing-singbox-vpn-install,
      # stage 0a) that already had /var/lib/singbox-vpn from an earlier,
      # unrelated interrupted test.
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=1\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=0\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    else
      # stage 27 (after the final uninstall): CLEANER than baseline —
      # this run's own uninstall removed the leftover /var/lib/singbox-vpn
      # that baseline had. That is a strictly better outcome than
      # baseline, not residue, and must not fail the gate.
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=0\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=0\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    fi
    exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH_RESIDUE
chmod +x "$MOCKBIN/ssh"
: > "$SSH_LOG"
rm -f "$TMPDIR_TEST/residue_call_count"
set +e
residue_out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --allow-destroy-existing-singbox-vpn-install --skip-reboot 2>&1)"
set -e
if grep -qE '\[PASS\][[:space:]]+no NEW singbox-vpn-owned' <<< "$residue_out"; then
  ok "a dirty baseline that this run cleaned up (not worsened) is reported PASS, not exact-equality FAIL"
else
  fail "stage 27 did not PASS a run that left the host cleaner than its (dirty) baseline: $(grep -i 'residue' <<< "$residue_out")"
fi
if grep -qE '\[FAIL\]\[required\][[:space:]]+new singbox-vpn-owned residue' <<< "$residue_out"; then
  fail "stage 27 reported new-residue FAIL even though after-uninstall state was a strict subset of (dirty) baseline"
fi

echo
echo "--- stage 27 residue audit: a field that is NEW after uninstall (not present at baseline) must still fail ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_RESIDUE2'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
COUNTER_FILE="$TMPDIR_TEST/residue_call_count"
case "$cmd" in
  true) exit 0 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *"opt_singbox-vpn="*)
    n=0
    [ -f "$COUNTER_FILE" ] && n="$(cat "$COUNTER_FILE")"
    n=$((n + 1))
    echo "$n" > "$COUNTER_FILE"
    if [ "$n" -eq 1 ]; then
      # Clean baseline: nothing pre-existing.
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=0\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=0\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    else
      # After "uninstall": nginx_conf residue that was NOT in the
      # baseline — a real leak this run's uninstall failed to remove.
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=0\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=1\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    fi
    exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH_RESIDUE2
chmod +x "$MOCKBIN/ssh"
: > "$SSH_LOG"
rm -f "$TMPDIR_TEST/residue_call_count"
set +e
newresidue_out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --allow-destroy-existing-singbox-vpn-install --skip-reboot 2>&1)"
set -e
if grep -qE '\[FAIL\]\[required\][[:space:]]+new singbox-vpn-owned residue.*nginx_conf' <<< "$newresidue_out"; then
  ok "a field genuinely new after uninstall (absent at baseline) is still correctly reported as residue and fails"
else
  fail "stage 27 did not catch genuinely new residue (nginx_conf) introduced beyond baseline: $(grep -i 'residue' <<< "$newresidue_out")"
fi

echo
echo "--- run_install()/run_install_abort_after_singbox()/dev-rebuild use the long SSH timeout, not the short per-probe one ---"
# Grep the file directly rather than piping a captured variable through
# `echo ... | grep -q`: under `set -o pipefail` (this file's own shebang
# options), `grep -q` exiting early on an easy match can close its stdin
# before `echo` finishes writing a large (~800-line) string, killing
# `echo` with SIGPIPE — and pipefail then reports THAT non-zero exit
# instead of grep's own successful match, turning a real pass into a
# flaky false FAIL (reproduced on a real CI runner). A here-string is
# fully materialized by bash before the reader starts, so it has no such
# race.
if grep -qE '^ssh_run_long\(\) \{ timeout [0-9]+ ssh' "$SCRIPT"; then
  ok "ssh_run_long() exists with its own (longer) timeout"
else
  fail "ssh_run_long() is missing — a from-source dev-channel install/update can legitimately run past ssh_run()'s short timeout and would be falsely reported as failed"
fi
run_install_body="$(sed -n '/^run_install() {/,/^}/p' "$SCRIPT")"
if grep -q 'ssh_run_long ' <<< "$run_install_body"; then
  ok "run_install() uses ssh_run_long (a from-source dev-channel build can run well past a short timeout)"
else
  fail "run_install() does not use ssh_run_long — a slow-but-correct from-source install would be falsely reported as [FAIL]"
fi
run_install_abort_body="$(sed -n '/^run_install_abort_after_singbox() {/,/^}/p' "$SCRIPT")"
if grep -q 'ssh_run_long ' <<< "$run_install_abort_body"; then
  ok "run_install_abort_after_singbox() uses ssh_run_long"
else
  fail "run_install_abort_after_singbox() does not use ssh_run_long"
fi

echo
echo "--- a required-stage failure cannot produce an overall PASS ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_FAILREQ'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
case "$cmd" in
  true) exit 1 ;;
  *) exit 1 ;;
esac
MOCKSSH_FAILREQ
chmod +x "$MOCKBIN/ssh"
set +e
PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --skip-reboot >"$TMPDIR_TEST/out-failreq.log" 2>&1
rc=$?
set -e
if [ "$rc" -ne 0 ] && grep -q '^LIFECYCLE GATE: FAIL$' "$TMPDIR_TEST/out-failreq.log"; then
  ok "a required-stage failure produces LIFECYCLE GATE: FAIL, never PASS"
else
  fail "a required-stage failure did not produce the expected FAIL result"
fi

echo
echo "--- UNVERIFIED items are reported distinctly, not silently folded into PASS ---"
if grep -q 'UNVERIFIED' "$TMPDIR_TEST/out-2222.log"; then
  ok "UNVERIFIED items appear in the report output"
else
  fail "no UNVERIFIED items were reported (expected at least public-reachability/Hiddify/cert)"
fi
if grep -q 'unverified items: [1-9]' "$TMPDIR_TEST/out-2222.log"; then
  ok "summary carries a non-zero unverified-item count distinct from failing stages"
else
  fail "summary did not carry a distinct unverified-item count"
fi

echo
if [ "$failures" -eq 0 ]; then
  echo "PASS: test-lifecycle-acceptance-harness.sh"
  exit 0
else
  echo "FAIL: test-lifecycle-acceptance-harness.sh ($failures failure(s))"
  exit 1
fi
