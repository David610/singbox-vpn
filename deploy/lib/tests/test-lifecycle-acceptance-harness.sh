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
case "$cmd" in
  true) exit 0 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *:*grep*) echo 1; exit 0 ;;
  *systemctl\ is-active*sshd*) exit 0 ;;
  *systemctl\ is-active*) exit 0 ;;
  *health-check.sh*) exit 0 ;;
  *ss\ -ltn*) echo ':443 LISTEN'; exit 0 ;;
  *list-timers*) echo 'vpn1-cert-renew.timer'; exit 0 ;;
  *install-state.json*) echo '{"vpn1_version":"mock"}'; exit 0 ;;
  *doctor\ --protocol*) exit 0 ;;
  *acceptance-test.sh*) exit 0 ;;
  *systemctl\ reboot*) exit 0 ;;
  *sudo\ systemctl\ reboot*) exit 0 ;;
  *VPN1_LIFECYCLE_GATE_ABORT_AFTER=install_singbox*) exit 1 ;;
  *vpn1-uninstall\ --yes*) echo 'uninstalled'; exit 0 ;;
  *iptables*) exit 0 ;;
  *vpn-admin\ user*) exit 0 ;;
  *install.sh*) exit 0 ;;
  *VPN1_LIFECYCLE_GATE_ABORT_AFTER=after_switch*update.sh*) exit 1 ;;
  *update.sh*) exit 0 ;;
  *certbot\ renew*) exit 0 ;;
  *"[ -e /opt/vpn1 ] || [ -e /etc/vpn ]"*) exit 0 ;;
  *"[ ! -e /etc/vpn ]"*) exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH
chmod +x "$MOCKBIN/ssh"
export SSH_LOG

cat > "$MOCKBIN/sleep" <<'MOCKSLEEP'
#!/bin/bash
exit 0
MOCKSLEEP
chmod +x "$MOCKBIN/sleep"

run_harness() {
  : > "$SSH_LOG"
  PATH="$MOCKBIN:$PATH" "$SCRIPT" "$@"
}

echo "--- destructive opt-in is required ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test 2>&1)" || rc=$?
if [ "$rc" -ne 0 ] && echo "$out" | grep -qi 'i-understand-this-is-destructive'; then
  ok "refuses to run without --i-understand-this-is-destructive"
else
  fail "did not refuse without destructive opt-in (rc=$rc)"
fi
[ ! -s "$SSH_LOG" ] && ok "no ssh calls made without destructive opt-in" || fail "ssh was invoked before the destructive opt-in gate"

echo
echo "--- --host is required ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --i-understand-this-is-destructive 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && echo "$out" | grep -qi -- '--host is required' \
  && ok "refuses to run without --host" || fail "did not refuse without --host"

echo
echo "--- localhost is refused even with destructive opt-in ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@localhost --i-understand-this-is-destructive 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && echo "$out" | grep -qi 'localhost' \
  && ok "refuses localhost target" || fail "did not refuse localhost target"

echo
echo "--- production host (VPN1_PRODUCTION_HOST) is refused ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" VPN1_PRODUCTION_HOST=prod.example.test "$SCRIPT" --host root@prod.example.test --i-understand-this-is-destructive 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && echo "$out" | grep -qi 'VPN1_PRODUCTION_HOST' \
  && ok "refuses VPN1_PRODUCTION_HOST target" || fail "did not refuse the configured production host"

echo
echo "--- non-numeric --ssh-port is rejected ---"
rc=0
out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --ssh-port abc 2>&1)" || rc=$?
[ "$rc" -ne 0 ] && echo "$out" | grep -qi 'ssh-port must be numeric' \
  && ok "rejects a non-numeric --ssh-port" || fail "did not reject a non-numeric --ssh-port"

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

echo
echo "--- failure-injection env var reaches the bash process that execs install.sh, not curl ---"
if grep -qP 'curl[^\t]*\|\tsudo\tVPN1_LIFECYCLE_GATE_ABORT_AFTER=install_singbox' "$SSH_LOG" \
  || grep -qE 'sudo VPN1_LIFECYCLE_GATE_ABORT_AFTER=install_singbox' "$SSH_LOG"; then
  ok "VPN1_LIFECYCLE_GATE_ABORT_AFTER is attached to the bash side of the curl|bash pipeline"
else
  fail "VPN1_LIFECYCLE_GATE_ABORT_AFTER was not found attached to the bash invocation"
fi
if grep -qP 'sudo\tVPN1_LIFECYCLE_GATE_ABORT_AFTER=install_singbox\tcurl' "$SSH_LOG"; then
  fail "regression: VPN1_LIFECYCLE_GATE_ABORT_AFTER is attached to curl's own exec again (the fixed bug reappeared)"
else
  ok "VPN1_LIFECYCLE_GATE_ABORT_AFTER is not mis-scoped to curl's exec"
fi

echo
echo "--- offline uninstall stage uses the local binary, never curl/GitHub ---"
if grep -q 'vpn1-uninstall --yes' "$SSH_LOG"; then
  ok "offline uninstall stage invokes /opt/vpn1/bin/vpn1-uninstall --yes"
else
  fail "offline uninstall stage did not invoke the local vpn1-uninstall binary"
fi
if grep -A2 '=== 11. offline uninstall' "$TMPDIR_TEST/out-2222.log" | grep -qi 'uninstall.sh | bash'; then
  fail "stage 11's own uninstall call still uses curl | bash instead of the offline binary"
else
  ok "stage 11's uninstall call does not use curl | bash"
fi

echo
echo "--- update version-change assertion: a no-op update must not be reported as an update ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_NOOP'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
case "$cmd" in
  true) exit 0 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *install-state.json*) echo '{"vpn1_version":"same"}'; exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH_NOOP
chmod +x "$MOCKBIN/ssh"
set +e
noop_out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --skip-reboot --update-to-ref main 2>&1)"
set -e
if echo "$noop_out" | grep -qi 'must not be reported as an update'; then
  ok "harness detects a no-op update (identical before/after version-state) and fails it"
else
  fail "harness did not detect a no-op update as a failure"
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
