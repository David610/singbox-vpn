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
WATCHDOG_TIMER_STATE_FILE="$TMPDIR_TEST/watchdog_timer_state"
# Defaults to "active": a freshly installed host has this timer enabled
# (WantedBy=timers.target); a scenario that needs it pre-set to inactive
# writes "inactive" to this file before calling run_harness.
watchdog_timer_state() { cat "$WATCHDOG_TIMER_STATE_FILE" 2>/dev/null || echo active; }
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
  # Stage 5's post-reboot verification script: matched here, before ANY
  # of the granular sing-box/systemctl/health-check.sh patterns further
  # down, since this one opaque multi-check script contains several of
  # those exact substrings itself (e.g. "is-active --quiet sing-box") —
  # placed later, those patterns would intercept it first and this test
  # scenario would never see the marker it's supposed to react to.
  # Reports the same "everything healthy" marker the real script emits on
  # success, so stage 5 recognizes it as a pass the same way a real
  # healthy host would.
  *"POST_REBOOT_ALL_OK"*|*"POST_REBOOT_FAILED_CHECKS"*)
    echo "POST_REBOOT_ALL_OK"; exit 0 ;;
  # Stage 15's entire scratch-user procedure is ONE opaque multi-line
  # remote script (one ssh_run call) — matched here, BEFORE the generic
  # "*:*grep*" catch-all just below, which would otherwise intercept it
  # first: the script's own `grep -o "\"id\": ..."` pattern text contains
  # a literal colon followed later by the `user list | grep -q` call,
  # satisfying that catch-all and short-circuiting the whole block to a
  # bare "exit 0". The happy-path outcome here (used by the end-to-end
  # "stage 15 reports PASS" assertion) is exactly that: success. The
  # script's OWN create/parse-id classification logic is exercised for
  # real elsewhere (see "stage 15's OWN parsing pipeline" below), which
  # extracts and directly executes the real embedded script body — a
  # mocked ssh can only ever fake this block's end result, never its
  # internal grep/sed behavior, since ssh here never actually executes
  # remote text.
  *"cleanup_scratch"*) exit 0 ;;
  # Stage 18's certificate-lineage lookup and its --no-random-sleep-on-renew
  # feature probe — matched here, BEFORE the generic "*:*grep*" catch-all
  # just below, which would otherwise intercept both: each genuinely
  # contains a ':' followed later by 'grep' as part of its own real logic
  # (parsing certbot's `certificates`/`--help renew` output), so placed
  # after that catch-all they would silently fall through to "echo 1; exit
  # 0" instead of either script's real response.
  *"--help renew"*)
    # `certbot --help renew | grep -q -- flag`: this ssh_run call's exit
    # code IS the pipe's outcome the harness branches on. Default here
    # (no flag file): supported.
    exit 0 ;;
  *"CERT_LOOKUP:NO_DEPLOYMENT_HOST"*)
    if [ -f "$TMPDIR_TEST/mock_cert_lookup_no_deployment_host" ]; then
      echo "CERT_LOOKUP:NO_DEPLOYMENT_HOST"; exit 2
    elif [ -f "$TMPDIR_TEST/mock_cert_lookup_no_lineage" ]; then
      echo "CERT_LOOKUP:NO_MATCHING_LINEAGE:example.test"; exit 3
    elif [ -f "$TMPDIR_TEST/mock_cert_lookup_domain_mismatch" ]; then
      echo "CERT_LOOKUP:DOMAIN_MISMATCH:example.test"; exit 4
    else
      echo "CERT_LOOKUP:OK:example.test"; exit 0
    fi ;;
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
  # The stage 13b suspend check: a real multi-line script that stops the
  # timer, reads its own ActiveState back, echoes it, then asserts
  # inactive. Matched on the literal "stop vpn-service-watchdog.timer"
  # text — BEFORE the generic is-active/timer patterns below — so this
  # one opaque invocation can flip the shared timer-state file and print
  # the same "ActiveState=..." line the real remote script would.
  *"stop vpn-service-watchdog.timer"*)
    echo inactive > "$WATCHDOG_TIMER_STATE_FILE"
    echo "ActiveState=inactive"
    exit 0 ;;
  # The stage 13b re-arm check: `systemctl start ... && is-active --quiet
  # ...`. Matched before the standalone is-active pattern below for the
  # same reason as above.
  *"start vpn-service-watchdog.timer"*)
    echo active > "$WATCHDOG_TIMER_STATE_FILE"
    exit 0 ;;
  # The pre-test "was it active before" probe (a bare is-active, no
  # stop/start alongside it) — reflects whatever state a scenario
  # pre-seeded via $WATCHDOG_TIMER_STATE_FILE, defaulting to active.
  *"is-active --quiet vpn-service-watchdog.timer"*)
    [ "$(watchdog_timer_state)" = "active" ] && exit 0 || exit 1 ;;
  # Diagnostic-only show call used when the suspend check fails; content
  # doesn't affect any pass/fail assertion.
  *"vpn-service-watchdog.timer"*)
    echo "ActiveState=$(watchdog_timer_state)"; exit 0 ;;
  *"systemctl start vpn-service-watchdog.service"*)
    # Simulates the real watchdog script: only acts on a FAILED unit.
    [ "$(singbox_state)" = "failed" ] && echo active > "$STATE_FILE"
    exit 0 ;;
  *"systemctl stop sing-box"*)
    echo stopped > "$STATE_FILE"
    exit 0 ;;
  *"systemctl start sing-box"*)
    # Reproduced twice on real VPS runs: a plain `systemctl start
    # sing-box` in stage 13c can genuinely fail. Injectable via a flag
    # file so a dedicated scenario can prove stage 14/15 correctly go
    # BLOCKED (not independently FAILed) when this happens.
    if [ -f "$TMPDIR_TEST/mock_singbox_start_fails" ]; then
      exit 1
    fi
    echo active > "$STATE_FILE"
    exit 0 ;;
  *"is-active --quiet sing-box"*)
    [ "$(singbox_state)" = "active" ] && exit 0 || exit 1 ;;
  *"is-failed --quiet sing-box"*)
    [ "$(singbox_state)" = "failed" ] && exit 0 || exit 1 ;;
  # Stage 18's actual remote certbot invocation (ssh_run_certbot, section
  # 18 of lifecycle-acceptance.sh) — matched on the inner-timeout-wrapped
  # command text, which is unique to this one call. Scenario-controllable
  # via flag files so dedicated fixtures below can exercise every
  # classification branch without waiting out any real timeout. Placed
  # here, BEFORE the generic "systemctl is-active"/etc. patterns further
  # down: this and the two arms after it embed real remote shell script
  # bodies (see lifecycle-acceptance.sh's stage 18) whose own text
  # happens to CONTAIN those generic substrings (e.g. the post-check
  # script below starts with "systemctl is-active --quiet nginx") — a
  # broad pattern positioned earlier would silently intercept the whole
  # call and return empty output instead of this scenario's real
  # response. Reproduced directly: with these arms placed after the
  # generic "systemctl is-active" pattern, the post-check call always
  # returned empty, which fed an unmatched `grep -o` into a plain
  # assignment and (correctly, per lifecycle-acceptance.sh's own
  # defensive `|| true` there) never crashed the target script — but it
  # also meant every "clean renewal" scenario here silently lost its
  # post-renewal PASS line and this whole test suite's assertions about
  # it stopped meaning anything.
  *"timeout -k 10s 300s sudo certbot renew --dry-run"*)
    if [ -f "$TMPDIR_TEST/mock_certbot_zero_renewals" ]; then
      echo "Processing /etc/letsencrypt/renewal/example.test.conf"
      echo "No simulated renewals were attempted."
      certbot_mock_rc=0
    elif [ -f "$TMPDIR_TEST/mock_certbot_nonzero" ]; then
      echo "Processing /etc/letsencrypt/renewal/example.test.conf"
      echo "Simulating renewal of an existing certificate for example.test"
      echo "All simulated renewals failed. The following certs could not be renewed:"
      certbot_mock_rc=1
    else
      echo "Processing /etc/letsencrypt/renewal/example.test.conf"
      echo "Simulating renewal of an existing certificate for example.test"
      echo "Congratulations, all simulated renewals succeeded:"
      certbot_mock_rc=0
    fi
    if [ -f "$TMPDIR_TEST/mock_certbot_no_sentinel" ]; then
      # Simulates the SSH session ending WITHOUT the remote script ever
      # reaching its own sentinel print (e.g. dropped mid-run) — the
      # controller must never treat this as proof of anything.
      exit "$certbot_mock_rc"
    fi
    printf '\n__SINGBOX_VPN_CERTBOT_DONE__ rc=%d\n' "$certbot_mock_rc"
    exit "$certbot_mock_rc" ;;
  # Stage 18's post-renewal cleanup-contract check — matched on its
  # distinctive "compgen -G" marker-glob text, BEFORE the generic
  # firewall-cmd pattern below (which this same script also contains).
  *"compgen -G "*)
    if [ -f "$TMPDIR_TEST/mock_certbot_marker_stale" ]; then
      echo "PORT80_AFTER=closed"
      echo "POST_CERTBOT_FAILED: stale-hook-markers"
      exit 1
    elif [ -f "$TMPDIR_TEST/mock_certbot_nginx_down" ]; then
      echo "PORT80_AFTER=closed"
      echo "POST_CERTBOT_FAILED: nginx-not-active"
      exit 1
    else
      echo "PORT80_AFTER=closed"
      echo "POST_CERTBOT_OK"
      exit 0
    fi ;;
  # Stage 18's pre-renewal TCP/80 baseline probe (port80_before). Reports
  # "closed" by default — a firewalld host with no pre-existing TCP/80
  # allow rule, the common case.
  *"firewall-cmd --query-port=80/tcp"*) echo closed; exit 0 ;;
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
    # Matches the REAL vpn-benchmark.sh's `kv "throughput (Mbps), N
    # run(s)" "min=... ..."` output shape: key and value on the SAME
    # line. A real VPS run showed the harness FAIL every real, successful
    # benchmark because this fixture used to model an (incorrect) 2-line
    # format the old `grep -A1 | tail -1` parsing happened to expect —
    # this fixture never caught the bug because it was equally wrong.
    cat <<'BENCH'
Hysteria2 protocol/server-side overhead (sing-box client on THIS VPS -> THIS VPS's public IP; NOT a remote-client network-path measurement)
--------------------------------------------------------------------------------------------------------------------------------------------
throughput (Mbps), 1 run(s): min=42.00 median=42.00 max=42.00 (n=1)
Assessment
BENCH
    exit 0 ;;
  *doctor\ --protocol*)
    # Injectable via a flag file so a dedicated scenario can prove stage
    # 14 stays a required FAIL (never BLOCKED) when sing-box is healthy
    # but the protocol self-test genuinely fails on its own.
    if [ -f "$TMPDIR_TEST/mock_protocol_fails" ]; then
      echo 'protocol self-test FAILED: a throwaway sing-box client using the CURRENT REALITY public_key/short_id could not complete a handshake through 127.0.0.1:443'
      exit 1
    fi
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
  *singbox-vpn-uninstall\ --yes*)
    echo 'uninstalled'
    printf '\n__SINGBOX_VPN_UNINSTALL_DONE__ rc=0\n'
    exit 0 ;;
  *iptables*) exit 0 ;;
  # Stage 9's persisted test-user create --json (the scratch-user create
  # in stage 15 is a separate, opaque multi-line block matched earlier by
  # "cleanup_scratch" above — this arm covers the standalone call only).
  # Real `vpn-admin user create --json` output is NOT pure JSON —
  # apply_users_and_save/render_and_apply_singbox_config print
  # human-readable status lines (verified in apps/admin/src/main.rs)
  # BEFORE the final JSON blob.
  *"vpn-admin user create"*"--json"*)
    scratch_id="mock-scratch-id-1"
    cat <<USERCREATE
sing-box config updated at "/etc/vpn/compat/sing-box/config.json" (validated by \`sing-box check\`).
reloading sing-box (1 active user(s) in the new config) — this is a full restart (sing-box has no in-place reload), so all currently connected clients will be briefly disconnected.
sing-box reloaded and verified active (including a real REALITY handshake self-test that PASSED).
{
  "id": "$scratch_id",
  "name": "lifecycle-scratch-user",
  "enabled": true,
  "subscription_url": "https://sub.mock.example.com:8443/sub/mocktoken?format=hiddify"
}
USERCREATE
    exit 0 ;;
  *vpn-admin\ user\ list*)
    echo "mock-id-1 lifecycle-test-user yes mock-scratch-id-1 lifecycle-scratch-user yes"
    exit 0 ;;
  *vpn-admin\ user*) exit 0 ;;
  *install.sh*) exit 0 ;;
  *SINGBOX_VPN_LIFECYCLE_GATE_ABORT_AFTER=after_switch*update.sh*) exit 1 ;;
  *update.sh*) exit 0 ;;
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
echo "--- stage 13b: timer suspension uses a real ActiveState check, never the invalid 'systemctl is-inactive' verb ---"
# Confirmed directly against a real system: `systemctl is-inactive` is not
# a real systemctl verb (systemd rejects it: "Unknown command verb
# 'is-inactive', did you mean 'is-active'?", exit 1) — this made the old
# check here fail unconditionally on every real host, regardless of
# whether the stop itself worked.
if grep -v '^\s*#' "$SCRIPT" | grep -q 'systemctl is-inactive'; then
  fail "lifecycle-acceptance.sh still invokes the nonexistent 'systemctl is-inactive' verb outside a comment"
else
  ok "no 'systemctl is-inactive' invocation remains anywhere in the script (outside explanatory comments)"
fi
if printf '%s' "$stage13b_block" | grep -q 'vpn-service-watchdog.timer suspended for deterministic crash-loop test'; then
  ok "the watchdog timer is successfully suspended using a real systemctl check"
else
  fail "stage 13b did not report the timer as successfully suspended"
fi
if printf '%s' "$stage13b_block" | grep -q 'vpn-service-watchdog.timer re-armed after deterministic crash-loop test'; then
  ok "the watchdog timer is re-armed (restored to its pre-test active state) after the crash-loop test"
else
  fail "stage 13b did not report the timer as re-armed"
fi

echo
echo "--- stage 13b: a timer that was already inactive before the test is restored to inactive, never turned on ---"
SSH_LOG_BACKUP="$(cat "$SSH_LOG" 2>/dev/null || true)"
: > "$SSH_LOG"
echo inactive > "$TMPDIR_TEST/watchdog_timer_state"
set +e
timer_inactive_out="$(run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1)"
set -e
rm -f "$TMPDIR_TEST/watchdog_timer_state"
if grep -q 'left inactive after the crash-loop test' <<< "$timer_inactive_out"; then
  ok "a timer that was inactive before the test is reported as correctly left inactive, not activated"
else
  fail "a pre-test-inactive timer was not correctly handled"
fi
if grep -q 'systemctl start vpn-service-watchdog.timer' "$SSH_LOG"; then
  fail "the harness sent 'start vpn-service-watchdog.timer' even though it was inactive beforehand — it must never activate a timer that was intentionally off"
else
  ok "no 'start vpn-service-watchdog.timer' command was sent when the timer was inactive beforehand"
fi
printf '%s' "$SSH_LOG_BACKUP" > "$SSH_LOG"

echo
echo "--- stage 13c restart failure BLOCKs stage 14 and stage 15 as prerequisite failures, not independent defects ---"
touch "$TMPDIR_TEST/mock_singbox_start_fails"
set +e
d_out="$(run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1)"
set -e
rm -f "$TMPDIR_TEST/mock_singbox_start_fails"
if grep -qE '\[FAIL\]\[required\][[:space:]]+sing-box restarted normally after the deliberate-stop test' <<< "$d_out"; then
  ok "stage 13c reports the sing-box restart failure as a required FAIL"
else
  fail "stage 13c did not report the restart failure as a required FAIL"
fi
if grep -qE '\[BLOCKED\][[:space:]]+REALITY handshake self-test after recovery' <<< "$d_out"; then
  ok "stage 14 is correctly BLOCKED (not an independent FAIL) when its sing-box prerequisite is down"
else
  fail "stage 14 was not correctly BLOCKED after stage 13c failed"
fi
if grep -qE '\[BLOCKED\][[:space:]]+scratch user create/rotate/disable/remove' <<< "$d_out"; then
  ok "stage 15 is correctly BLOCKED (not an independent FAIL) when its sing-box prerequisite is down"
else
  fail "stage 15 was not correctly BLOCKED after stage 13c failed"
fi
if grep -A2 'sing-box restarted normally after the deliberate-stop test' <<< "$d_out" | grep -q 'diagnostics:'; then
  ok "stage 13c's failure message includes real diagnostic detail, not a bare FAIL"
else
  fail "stage 13c's failure message lacks diagnostic detail"
fi

echo
echo "--- stage 14 remains a required FAIL (never BLOCKED) when sing-box is healthy but the protocol self-test genuinely fails ---"
touch "$TMPDIR_TEST/mock_protocol_fails"
set +e
e_out="$(run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1)"
set -e
rm -f "$TMPDIR_TEST/mock_protocol_fails"
stage14_block="$(sed -n '/=== 14\./,/=== 15\./p' <<< "$e_out")"
if printf '%s' "$stage14_block" | grep -qE '\[FAIL\]\[required\]'; then
  ok "stage 14 is a required FAIL when sing-box is healthy but the protocol self-test fails on its own"
else
  fail "stage 14 was not reported as a required FAIL for a genuine protocol failure"
fi
if printf '%s' "$stage14_block" | grep -q 'BLOCKED'; then
  fail "stage 14 was incorrectly BLOCKED even though its sing-box prerequisite (13c) succeeded"
else
  ok "stage 14 is not blocked when its prerequisite (sing-box) actually succeeded"
fi
if printf '%s' "$stage14_block" | grep -q 'see remote output above'; then
  fail "stage 14 still claims output is 'above' even though it is only ever captured into a variable, never printed"
else
  ok "stage 14 no longer claims diagnostic output is displayed 'above' when it isn't"
fi
if printf '%s' "$stage14_block" | grep -qE 'output: '; then
  ok "stage 14's failure message includes the actual doctor --protocol output"
else
  fail "stage 14's failure message does not include diagnostic output"
fi

echo
echo "--- stage 15's OWN parsing pipeline (grep/sed, not the ssh mock): create failure, malformed response, realistic success ---"
# The stage 15 scratch-user procedure is one opaque multi-line remote
# script sent as a SINGLE ssh_run argument — mocking ssh can only decide
# the whole block's outcome, it can never exercise that script's own
# embedded `grep -o "\"id\": ...\" | sed -E ...` parsing logic (ssh here
# never actually executes remote text, it only pattern-matches it). To
# actually test that parsing — the exact logic this fix changed — extract
# the real script body from lifecycle-acceptance.sh and execute it for
# real, in bash, against a stubbed `sudo` that plays vpn-admin.
SCRATCH_BODY="$(sed -n '/^    scratch_id=""$/,/^    trap - EXIT$/p' "$SCRIPT")"
if [ -z "$SCRATCH_BODY" ]; then
  fail "could not extract stage 15's scratch-user script body from $SCRIPT — its marker lines may have changed"
else
  run_scratch_scenario() {
    local scenario="$1"
    cat > "$TMPDIR_TEST/scratch_stub.sh" <<STUBEOF
sudo() {
  local bin="\$1"; shift
  case "\$bin" in
    */vpn-admin)
      case "\$1 \$2" in
        "user create")
          if [ "$scenario" = "create-fails" ]; then
            echo "Error: sing-box reload failed after applying the new config" >&2
            return 1
          fi
          # Realistic --json output: non-JSON status lines BEFORE the
          # JSON blob (verified against apps/admin/src/main.rs) — a
          # parser that assumed stdout starts with '{' would break here.
          echo 'sing-box config updated at "/etc/vpn/compat/sing-box/config.json" (validated by \`sing-box check\`).'
          echo 'reloading sing-box (1 active user(s) in the new config) — this is a full restart.'
          if [ "$scenario" = "no-id" ]; then
            echo '{"name": "lifecycle-scratch-user", "enabled": true}'
          else
            echo '{"id": "real-scratch-id-1", "name": "lifecycle-scratch-user", "enabled": true}'
          fi
          return 0 ;;
        "user list") echo "real-scratch-id-1 lifecycle-scratch-user yes"; return 0 ;;
        *) return 0 ;;
      esac ;;
    *) return 0 ;;
  esac
}
$SCRATCH_BODY
STUBEOF
    bash "$TMPDIR_TEST/scratch_stub.sh"
  }

  set +e
  out_ok="$(run_scratch_scenario success 2>/dev/null)"; rc_ok=$?
  out_create_fail="$(run_scratch_scenario create-fails 2>/dev/null)"; rc_create_fail=$?
  out_no_id="$(run_scratch_scenario no-id 2>/dev/null)"; rc_no_id=$?
  set -e

  if [ "$rc_ok" -eq 0 ]; then
    ok "the real parsing pipeline succeeds end-to-end against realistic non-JSON-prefixed --json output"
  else
    fail "the real parsing pipeline failed against a realistic successful create response (got: '$out_ok', rc=$rc_ok)"
  fi

  if [ "$rc_create_fail" -ne 0 ] && [ "$out_create_fail" = "create" ]; then
    ok "the real parsing pipeline reports 'create' (not 'parse-id') when vpn-admin user create itself fails"
  else
    fail "a real create-command failure was not correctly classified (got: '$out_create_fail', rc=$rc_create_fail) — this is the exact bug this fix addresses"
  fi

  if [ "$rc_no_id" -ne 0 ] && [ "$out_no_id" = "parse-id" ]; then
    ok "the real parsing pipeline reports 'parse-id' when create exits 0 but no id is present in the response"
  else
    fail "a malformed (no-id) create response was not correctly classified (got: '$out_no_id', rc=$rc_no_id)"
  fi
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
echo "--- stage 15 reports PASS end-to-end against the default (healthy) mocked target ---"
# The actual id-extraction-against-realistic-output proof lives in "stage
# 15's OWN parsing pipeline" above (real execution, not a mocked ssh) —
# this assertion only confirms the STAGE reports PASS when its remote
# calls succeed, i.e. that stage 13c's new BLOCKED-gating didn't
# accidentally break the ordinary healthy-target path.
stage15_block="$(sed -n '/=== 15\./,/=== 16/p' "$TMPDIR_TEST/out-2222.log")"
if printf '%s' "$stage15_block" | grep -qE '\[PASS\][[:space:]]+scratch user create/rotate/disable/remove'; then
  ok "the scratch-user lifecycle reports PASS against a healthy mocked target"
else
  fail "stage 15 did not pass against a healthy mocked target"
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
echo "--- run_install()/run_install_abort_after_singbox() suppress onboarding credentials (install.sh's own transcript streams live to this harness's stdout, uncaptured) ---"
if grep -q 'SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1' <<< "$run_install_body"; then
  ok "run_install() sets SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 on the remote install.sh invocation"
else
  fail "run_install() no longer suppresses onboarding secrets — install.sh's real subscription URL/QR would stream into this harness's own transcript (CI logs, release evidence)"
fi
if grep -q 'SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1' <<< "$run_install_abort_body"; then
  ok "run_install_abort_after_singbox() sets SINGBOX_VPN_SUPPRESS_ONBOARDING_SECRETS=1 on the remote install.sh invocation"
else
  fail "run_install_abort_after_singbox() no longer suppresses onboarding secrets"
fi

echo
echo "--- Stage 18 no longer uses ssh_run_long (Rust-build-sized 1200s), it has its own dedicated ssh_run_certbot bound ---"
# Originally fixed by switching stage 18 to ssh_run_long (correct relative
# to ssh_run's 180s, but still wrong: certbot inherited a 20-minute Rust
# -build timeout and its entire output was hidden inside a plain
# `VAR="$(...)"` substitution until the whole run finished — on a real VPS
# this made an already-finished renewal look permanently frozen. Stage 18
# now has its own certbot-sized helper and streams output live.
if grep -qE '^ssh_run_certbot\(\) \{' "$SCRIPT"; then
  ok "ssh_run_certbot() exists as its own dedicated helper"
else
  fail "ssh_run_certbot() is missing"
fi
ssh_run_certbot_body="$(sed -n '/^ssh_run_certbot() {/,/^}/p' "$SCRIPT")"
if grep -qE 'timeout -k 10s 360s ssh' <<< "$ssh_run_certbot_body"; then
  ok "ssh_run_certbot() has its own bounded outer timeout with a guaranteed forced-kill (-k)"
else
  fail "ssh_run_certbot() does not use a bounded timeout with -k (forced kill)"
fi
if grep -q 'ServerAliveInterval=15' <<< "$ssh_run_certbot_body" && grep -q 'ServerAliveCountMax=4' <<< "$ssh_run_certbot_body"; then
  ok "ssh_run_certbot() sets SSH keepalive options to detect a stalled transport"
else
  fail "ssh_run_certbot() does not set SSH keepalive options"
fi
if grep -qE '^  section "18\.' "$SCRIPT" && \
   ! grep -qE 'CERTBOT_DRY_OUT="\$\(ssh_run(_long)? ' "$SCRIPT"; then
  ok "stage 18 no longer hides the entire certbot run inside a plain \"\$(...)\" command substitution"
else
  fail "stage 18 still captures certbot's output with a plain VAR=\"\$(...)\" substitution — output would be hidden until the whole run finishes"
fi
if grep -q 'ssh_run_certbot "\$remote_certbot_cmd" 2>&1 | tee "\$CERTBOT_LOG_FILE"' "$SCRIPT"; then
  ok "stage 18 streams certbot's output live via tee while still capturing it"
else
  fail "stage 18 does not stream certbot's output live via tee"
fi
if grep -q 'certbot_dry_rc=\${PIPESTATUS\[0\]}' "$SCRIPT"; then
  ok "stage 18 reads the real remote exit code via PIPESTATUS[0], not tee's own exit code"
else
  fail "stage 18 does not correctly capture the piped command's real exit status via PIPESTATUS"
fi
if grep -q "CERTBOT_SENTINEL='__SINGBOX_VPN_CERTBOT_DONE__'" "$SCRIPT" \
    && grep -q '__SINGBOX_VPN_CERTBOT_DONE__ rc=%d' "$SCRIPT"; then
  ok "stage 18 appends a project-specific completion sentinel carrying the real remote exit code"
else
  fail "stage 18 is missing its completion sentinel"
fi

echo
echo "--- Stage 18: sentinel + real exit code drive PASS/FAIL, not the sentinel alone ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_CERTBOT'
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
  *"doctor --protocol"*) echo 'completed a full handshake'; exit 0 ;;
  *"vpn-benchmark.sh"*) echo "throughput (Mbps), 1 run(s): min=42.00 median=42.00 max=42.00 (n=1)"; exit 0 ;;
  # Stage 18's certificate-lineage lookup and --no-random-sleep-on-renew
  # feature probe, run BEFORE the actual renewal attempt below.
  # Scenario-controlled the same way as the renewal mock itself.
  *"--help renew"*)
    # The real remote command is `certbot --help renew | grep -q -- flag`
    # (grep -q, no stdout) — this ssh_run call's exit code IS the pipe's
    # outcome the harness branches on (0 = flag supported, 1 = not), so
    # this mock must decide THAT exit code, not just always exit 0.
    if [ -f "$TMPDIR_TEST/mock_certbot_no_random_sleep_unsupported" ]; then
      exit 1
    fi
    exit 0 ;;
  *"CERT_LOOKUP:NO_DEPLOYMENT_HOST"*)
    if [ -f "$TMPDIR_TEST/mock_cert_lookup_no_deployment_host" ]; then
      echo "CERT_LOOKUP:NO_DEPLOYMENT_HOST"; exit 2
    elif [ -f "$TMPDIR_TEST/mock_cert_lookup_no_lineage" ]; then
      echo "CERT_LOOKUP:NO_MATCHING_LINEAGE:example.test"; exit 3
    elif [ -f "$TMPDIR_TEST/mock_cert_lookup_domain_mismatch" ]; then
      echo "CERT_LOOKUP:DOMAIN_MISMATCH:example.test"; exit 4
    else
      echo "CERT_LOOKUP:OK:example.test"; exit 0
    fi ;;
  # The real remote certbot invocation. Scenario-controlled via flag files
  # so every classification branch is exercised end-to-end, through the
  # SAME code this harness would run for real, with no real waiting.
  *"timeout -k 10s 300s sudo certbot renew --dry-run"*)
    if [ -f "$TMPDIR_TEST/mock_certbot_transport_timeout" ]; then
      # Simulates ssh_run_certbot's own OUTER bound firing: the remote
      # script never got a chance to print anything, sentinel included.
      exit 124
    fi
    if [ -f "$TMPDIR_TEST/mock_certbot_zero_renewals" ]; then
      echo "No simulated renewals were attempted."
      certbot_mock_rc=0
    elif [ -f "$TMPDIR_TEST/mock_certbot_nonzero" ]; then
      echo "All simulated renewals failed. The following certs could not be renewed:"
      certbot_mock_rc=1
    elif [ -f "$TMPDIR_TEST/mock_certbot_remote_timeout" ]; then
      # Simulates the REMOTE inner `timeout -k 10s 300s` firing: the
      # wrapping script still runs to completion and reports it via the
      # sentinel — this must be told apart from a dead transport.
      certbot_mock_rc=124
    else
      echo "Congratulations, all simulated renewals succeeded:"
      certbot_mock_rc=0
    fi
    if [ -f "$TMPDIR_TEST/mock_certbot_no_sentinel" ]; then
      # SSH session ends WITHOUT the remote script ever reaching its own
      # sentinel print — must never be treated as proof of anything.
      exit "$certbot_mock_rc"
    fi
    printf '\n__SINGBOX_VPN_CERTBOT_DONE__ rc=%d\n' "$certbot_mock_rc"
    exit "$certbot_mock_rc" ;;
  # Post-renewal cleanup-contract check — matched on its distinctive
  # "compgen -G" text, before the generic firewall-cmd pattern below
  # (which this same remote script also contains).
  *"compgen -G "*)
    if [ -f "$TMPDIR_TEST/mock_certbot_marker_stale" ]; then
      echo "PORT80_AFTER=closed"; echo "POST_CERTBOT_FAILED: stale-hook-markers"; exit 1
    elif [ -f "$TMPDIR_TEST/mock_certbot_nginx_down" ]; then
      echo "PORT80_AFTER=closed"; echo "POST_CERTBOT_FAILED: nginx-not-active"; exit 1
    else
      echo "PORT80_AFTER=closed"; echo "POST_CERTBOT_OK"; exit 0
    fi ;;
  *"firewall-cmd --query-port=80/tcp"*) echo closed; exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH_CERTBOT
chmod +x "$MOCKBIN/ssh"

run_certbot_scenario() {
  : > "$SSH_LOG"
  rm -f "$TMPDIR_TEST"/mock_certbot_* "$TMPDIR_TEST"/mock_cert_lookup_*
  for f in "$@"; do : > "$TMPDIR_TEST/mock_certbot_$f"; done
  set +e
  run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1
  set -e
}

run_certbot_lookup_scenario() {
  : > "$SSH_LOG"
  rm -f "$TMPDIR_TEST"/mock_certbot_* "$TMPDIR_TEST"/mock_cert_lookup_*
  for f in "$@"; do : > "$TMPDIR_TEST/mock_cert_lookup_$f"; done
  set +e
  run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1
  set -e
}

echo "  - (A) clean renewal + full cleanup contract -> PASS"
out_a="$(run_certbot_scenario)"
stage18_a="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_a")"
if grep -qE '\[PASS\][[:space:]]+certbot renew --dry-run' <<< "$stage18_a"; then
  ok "(A) certbot renewal reports PASS"
else
  fail "(A) certbot renewal did not report PASS: $stage18_a"
fi
if grep -qE '\[PASS\][[:space:]]+nginx restored, certbot hook markers cleaned' <<< "$stage18_a"; then
  ok "(A) post-renewal cleanup contract (nginx/markers/firewall) reports PASS"
else
  fail "(A) post-renewal cleanup contract did not report PASS: $stage18_a"
fi
if grep -q -- 'certbot renew --dry-run --cert-name example.test' "$SSH_LOG"; then
  ok "(A) the renewal is scoped to this deployment's own lineage via --cert-name, never a bare/global 'certbot renew'"
else
  fail "(A) the renewal was not scoped via --cert-name — this is the exact unrelated-lineage false-failure bug this fix addresses"
fi
if grep -q -- '--no-random-sleep-on-renew' "$SSH_LOG"; then
  ok "(A) --no-random-sleep-on-renew is added when the installed certbot supports it (probed live, not assumed)"
else
  fail "(A) --no-random-sleep-on-renew was not added even though the mocked certbot advertises support for it"
fi

echo "  - (K) certbot lacks --no-random-sleep-on-renew support -> the flag is omitted, never blindly assumed"
out_k="$(run_certbot_scenario no_random_sleep_unsupported)"
# Note: SSH_LOG's own PROBE command line legitimately contains the literal
# string "--no-random-sleep-on-renew" (it's grepping FOR that flag in
# certbot's --help output) — so the assertion must target the actual
# renewal invocation line specifically, not just search the whole log for
# that substring.
if grep -q -- 'certbot renew --dry-run --cert-name example.test$' "$SSH_LOG"; then
  ok "(K) the flag is correctly omitted against an older certbot that doesn't advertise it"
else
  fail "(K) the flag was sent even though the mocked certbot's --help renew doesn't advertise support for it: $(grep -- 'certbot renew --dry-run' "$SSH_LOG")"
fi
stage18_k="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_k")"
if grep -qE '\[PASS\][[:space:]]+certbot renew --dry-run' <<< "$stage18_k"; then
  ok "(K) the renewal itself still PASSes normally without the flag"
else
  fail "(K) omitting the flag broke the otherwise-healthy renewal: $stage18_k"
fi

echo "  - (D-new) no current-deployment certificate lineage exists -> FAIL, classified CERT_LINEAGE_LOOKUP_FAILED, and certbot is NEVER invoked globally"
out_nolineage="$(run_certbot_lookup_scenario no_lineage)"
stage18_nolineage="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_nolineage")"
if grep -qE '\[FAIL\]\[required\].*CERT_LINEAGE_LOOKUP_FAILED' <<< "$stage18_nolineage"; then
  ok "(D-new) a missing current-deployment lineage is a required FAIL, classified CERT_LINEAGE_LOOKUP_FAILED"
else
  fail "(D-new) a missing current-deployment lineage was not correctly classified: $stage18_nolineage"
fi
if grep -q -- 'sudo certbot renew --dry-run' "$SSH_LOG"; then
  fail "(D-new) certbot renew was invoked even though no matching current-deployment lineage was ever found — this could still be affected by/affect unrelated lineages"
else
  ok "(D-new) certbot renew is never attempted at all when the current deployment's own lineage cannot be established"
fi

echo "  - (D-new2) deployment.toml has no public_host at all -> FAIL, classified CERT_LINEAGE_LOOKUP_FAILED"
out_nohost="$(run_certbot_lookup_scenario no_deployment_host)"
stage18_nohost="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_nohost")"
if grep -qE '\[FAIL\]\[required\].*CERT_LINEAGE_LOOKUP_FAILED' <<< "$stage18_nohost"; then
  ok "(D-new2) a missing deployment public_host is a required FAIL, classified CERT_LINEAGE_LOOKUP_FAILED"
else
  fail "(D-new2) a missing deployment public_host was not correctly classified: $stage18_nohost"
fi

echo "  - (domain-mismatch) the matched lineage's Domains: list does not actually contain the current public_host -> FAIL, classified CERT_LINEAGE_LOOKUP_FAILED"
out_mismatch="$(run_certbot_lookup_scenario domain_mismatch)"
stage18_mismatch="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_mismatch")"
if grep -qE '\[FAIL\]\[required\].*CERT_LINEAGE_LOOKUP_FAILED' <<< "$stage18_mismatch"; then
  ok "(domain-mismatch) a lineage whose Domains: list doesn't include the current host is rejected, not silently used"
else
  fail "(domain-mismatch) a domain/lineage mismatch was not correctly classified: $stage18_mismatch"
fi

echo "  - (C) certbot exits nonzero -> FAIL, classified CERTBOT_EXIT_NONZERO"
out_c="$(run_certbot_scenario nonzero)"
stage18_c="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_c")"
if grep -qE '\[FAIL\]\[required\].*CERTBOT_EXIT_NONZERO' <<< "$stage18_c"; then
  ok "(C) nonzero certbot exit is a required FAIL classified CERTBOT_EXIT_NONZERO"
else
  fail "(C) nonzero certbot exit was not classified CERTBOT_EXIT_NONZERO: $stage18_c"
fi

echo "  - (F) zero lineages tested (\"No simulated renewals were attempted.\") -> FAIL, classified NO_RENEWAL_ATTEMPTED, even though certbot itself exited 0"
out_f="$(run_certbot_scenario zero_renewals)"
stage18_f="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_f")"
if grep -qE '\[FAIL\]\[required\].*NO_RENEWAL_ATTEMPTED' <<< "$stage18_f"; then
  ok "(F) zero-lineage dry-run (exit 0) is still a required FAIL, classified NO_RENEWAL_ATTEMPTED — this gate is not weakened"
else
  fail "(F) zero-lineage dry-run was not correctly rejected: $stage18_f"
fi

echo "  - (J) sentinel present but its OWN carried exit code is nonzero -> still FAILS (the sentinel is evidence, never a substitute for the real exit code)"
if grep -qF '__SINGBOX_VPN_CERTBOT_DONE__' <<< "$out_c" && grep -qE '\[FAIL\]\[required\]' <<< "$stage18_c"; then
  ok "(J) a completion sentinel is present in scenario (C) yet the stage still correctly FAILs on the nonzero exit code it carries"
else
  fail "(J) scenario (C) either lost its sentinel or incorrectly passed despite a nonzero carried exit code"
fi

echo "  - sentinel missing after an apparent clean SSH exit -> FAIL, classified SSH_SESSION_ENDED_UNEXPECTEDLY (suspicious transport, not silently accepted)"
out_nosent="$(run_certbot_scenario no_sentinel)"
stage18_nosent="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_nosent")"
if grep -qE '\[FAIL\]\[required\].*SSH_SESSION_ENDED_UNEXPECTEDLY' <<< "$stage18_nosent"; then
  ok "a missing completion sentinel is classified SSH_SESSION_ENDED_UNEXPECTEDLY and FAILs, never silently accepted as PASS"
else
  fail "a missing completion sentinel was not correctly classified/rejected: $stage18_nosent"
fi

echo "  - (D) remote inner timeout fires (sentinel present, carried rc=124) -> FAIL, classified CERTBOT_REMOTE_TIMEOUT"
out_rtimeout="$(run_certbot_scenario remote_timeout)"
stage18_rtimeout="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_rtimeout")"
if grep -qE '\[FAIL\]\[required\].*CERTBOT_REMOTE_TIMEOUT' <<< "$stage18_rtimeout"; then
  ok "(D) a remote inner-timeout completion (rc=124, sentinel present) is classified CERTBOT_REMOTE_TIMEOUT"
else
  fail "(D) a remote timeout was not correctly classified: $stage18_rtimeout"
fi

echo "  - (E) local/outer SSH timeout fires before the remote script ever prints anything -> FAIL, classified SSH_TRANSPORT_TIMEOUT (distinct from a remote timeout)"
out_ttimeout="$(run_certbot_scenario transport_timeout)"
stage18_ttimeout="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_ttimeout")"
if grep -qE '\[FAIL\]\[required\].*SSH_TRANSPORT_TIMEOUT' <<< "$stage18_ttimeout"; then
  ok "(E) an outer SSH/transport timeout (no sentinel ever seen) is classified SSH_TRANSPORT_TIMEOUT, distinct from CERTBOT_REMOTE_TIMEOUT"
else
  fail "(E) an outer SSH/transport timeout was not correctly classified: $stage18_ttimeout"
fi

echo "  - (G) certbot succeeds but nginx was not restored afterward -> FAIL, classified POST_RENEWAL_STATE_INVALID"
out_g="$(run_certbot_scenario nginx_down)"
stage18_g="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_g")"
if grep -qE '\[PASS\][[:space:]]+certbot renew --dry-run' <<< "$stage18_g" \
    && grep -qE '\[FAIL\]\[required\].*POST_RENEWAL_STATE_INVALID' <<< "$stage18_g" \
    && grep -q 'nginx-not-active' <<< "$stage18_g"; then
  ok "(G) a successful certbot run with nginx left down still FAILs the post-renewal cleanup check"
else
  fail "(G) nginx not being restored after a successful renewal was not caught: $stage18_g"
fi

echo "  - (H) certbot succeeds but a hook marker file was left behind -> FAIL, classified POST_RENEWAL_STATE_INVALID"
out_h="$(run_certbot_scenario marker_stale)"
stage18_h="$(sed -n '/=== 18\./,/=== 19\./p' <<< "$out_h")"
if grep -qE '\[PASS\][[:space:]]+certbot renew --dry-run' <<< "$stage18_h" \
    && grep -qE '\[FAIL\]\[required\].*POST_RENEWAL_STATE_INVALID' <<< "$stage18_h" \
    && grep -q 'stale-hook-markers' <<< "$stage18_h"; then
  ok "(H) a successful certbot run with a leftover /run marker still FAILs the post-renewal cleanup check"
else
  fail "(H) a leftover hook marker after a successful renewal was not caught: $stage18_h"
fi

echo
echo "--- Stage 18 classification logic (extracted from the real script, executed directly — not a duplicate) ---"
CLASS_BODY="$(sed -n '/^  certbot_class=""$/,/^  fi$/p' "$SCRIPT")"
if [ -z "$CLASS_BODY" ]; then
  fail "could not extract stage 18's classification logic from $SCRIPT — its marker lines may have changed"
else
  # Runs the REAL extracted classification block in an isolated subshell
  # with the four inputs it reads set as plain shell variables first —
  # `eval` (not string-interpolating a nested `bash -c` argument) so `$`
  # inside CLASS_BODY is expanded exactly once, by this subshell, using
  # these values, never pre-expanded by the caller.
  # These four are consumed by $CLASS_BODY via `eval`, which shellcheck
  # cannot see into (it's the real classification logic extracted from
  # lifecycle-acceptance.sh, not a static string), so it reports both
  # "appears unused" on these assignments and "referenced but not
  # assigned" on certbot_class below. Real usage — proven by the
  # classification assertions that follow actually passing.
  run_class_scenario() (
    # shellcheck disable=SC2034
    sentinel_present="$1"
    # shellcheck disable=SC2034
    sentinel_rc="$2"
    # shellcheck disable=SC2034
    certbot_dry_rc="$3"
    # shellcheck disable=SC2034
    zero_renewals="$4"
    eval "$CLASS_BODY"
    # shellcheck disable=SC2154
    echo "$certbot_class"
  )
  class_all_ok=1
  check_class() {
    local desc="$1" expected="$2" got
    shift 2
    got="$(run_class_scenario "$@")"
    if [ "$got" = "$expected" ]; then
      ok "classification: $desc -> '$expected'"
    else
      fail "classification: $desc -> expected '$expected', got '$got'"
      class_all_ok=0
    fi
  }
  check_class "no sentinel, local timeout exit code (124)" "SSH_TRANSPORT_TIMEOUT" 0 "" 124 0
  check_class "no sentinel, local timeout exit code (137, SIGKILL)" "SSH_TRANSPORT_TIMEOUT" 0 "" 137 0
  check_class "no sentinel, non-timeout SSH exit (255, connection lost)" "SSH_SESSION_ENDED_UNEXPECTEDLY" 0 "" 255 0
  check_class "sentinel present, carried rc=124 (remote inner timeout)" "CERTBOT_REMOTE_TIMEOUT" 1 124 124 0
  check_class "sentinel present, carried rc=137 (remote inner SIGKILL)" "CERTBOT_REMOTE_TIMEOUT" 1 137 137 0
  check_class "sentinel present, rc=0, but zero lineages tested" "NO_RENEWAL_ATTEMPTED" 1 0 0 1
  check_class "sentinel present, carried rc=1 (genuine certbot failure)" "CERTBOT_EXIT_NONZERO" 1 1 1 0
  check_class "sentinel present, rc=0, at least one lineage tested (PASS candidate)" "" 1 0 0 0
  [ "$class_all_ok" -eq 1 ] && ok "all classification branches resolve correctly against the real extracted logic"
fi

echo
echo "--- Stage 19: uninstall gets its own execution classification instead of one opaque FAIL ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_UNINSTALL'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
COUNTER_FILE="$TMPDIR_TEST/uninstall_residue_counter"
case "$cmd" in
  true) exit 0 ;;
  *"[ -e '/etc/vpn/deployment.toml' ]"* | \
  *"[ -e '/var/lib/singbox-vpn/install-state.json' ]"* | \
  *"[ -e '/var/lib/singbox-vpn/ownership.env' ]"* | \
  *"[ -e '/opt/singbox-vpn' ]"*)
    exit 1 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  *"test -x /opt/singbox-vpn/bin/singbox-vpn-uninstall"*) exit 0 ;;
  # The lock-contention probe (flock -n -w 0 ...), run only when the
  # uninstaller itself exited nonzero — matched on its own distinctive
  # marker text, before the uninstall-invocation arm below.
  *"flock -n -w 0"*)
    [ -f "$TMPDIR_TEST/mock_uninstall_lock_held" ] && exit 1 || exit 0 ;;
  # The real remote uninstall invocation (run_uninstall_classified()).
  *"timeout -k 10s 480s sudo /opt/singbox-vpn/bin/singbox-vpn-uninstall --yes"*)
    if [ -f "$TMPDIR_TEST/mock_uninstall_ssh_timeout" ]; then
      exit 124
    fi
    if [ -f "$TMPDIR_TEST/mock_uninstall_nonzero" ] || [ -f "$TMPDIR_TEST/mock_uninstall_lock_held" ]; then
      rc=1
    elif [ -f "$TMPDIR_TEST/mock_uninstall_remote_timeout" ]; then
      rc=124
    else
      rc=0
    fi
    printf '\n__SINGBOX_VPN_UNINSTALL_DONE__ rc=%d\n' "$rc"
    exit "$rc" ;;
  # check_residue_vs_baseline(): 1st call is stage 1b's own baseline
  # capture (clean); subsequent calls are stage 19's (and later stage
  # 27's) post-uninstall checks.
  *"opt_singbox-vpn="*)
    n=0
    [ -f "$COUNTER_FILE" ] && n="$(cat "$COUNTER_FILE")"
    n=$((n + 1))
    echo "$n" > "$COUNTER_FILE"
    if [ "$n" -eq 1 ]; then
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=0\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=0\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    elif [ -f "$TMPDIR_TEST/mock_uninstall_residue_left" ]; then
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=0\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=1\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    else
      printf 'opt_singbox-vpn=0\netc_vpn=0\nvar_lib_singbox-vpn=0\nuser_singbox=0\nuser_vpnsub=0\nunit_singbox=0\nunit_vpnsub=0\nnginx_conf=0\ncertbot_hook=0\nlisteners=0\nlocks=0\n'
    fi
    exit 0 ;;
  *) exit 0 ;;
esac
MOCKSSH_UNINSTALL
chmod +x "$MOCKBIN/ssh"

run_uninstall_scenario() {
  : > "$SSH_LOG"
  rm -f "$TMPDIR_TEST"/mock_uninstall_* "$TMPDIR_TEST/uninstall_residue_counter"
  for f in "$@"; do : > "$TMPDIR_TEST/mock_uninstall_$f"; done
  set +e
  run_harness --host root@disposable-test --i-understand-this-is-destructive --skip-reboot 2>&1
  set -e
}

echo "  - default: uninstall succeeds and leaves no new residue -> PASS, residue check PASS"
out19_ok="$(run_uninstall_scenario)"
stage19_ok="$(sed -n '/=== 19\./,/=== 20\./p' <<< "$out19_ok")"
if grep -qE '\[PASS\][[:space:]]+singbox-vpn-uninstall --yes \(offline, local binary only\)' <<< "$stage19_ok" \
    && grep -qE '\[PASS\][[:space:]]+offline uninstall left no NEW singbox-vpn-owned residue' <<< "$stage19_ok"; then
  ok "default: stage 19 and its post-uninstall residue check both PASS"
else
  fail "default: stage 19 did not PASS cleanly: $stage19_ok"
fi

echo "  - uninstaller exits nonzero (no lock held) -> FAIL, classified UNINSTALL_EXIT_NONZERO"
out19_nz="$(run_uninstall_scenario nonzero)"
stage19_nz="$(sed -n '/=== 19\./,/=== 20\./p' <<< "$out19_nz")"
if grep -qE '\[FAIL\]\[required\].*UNINSTALL_EXIT_NONZERO' <<< "$stage19_nz"; then
  ok "a genuine nonzero uninstall exit is classified UNINSTALL_EXIT_NONZERO"
else
  fail "a nonzero uninstall exit was not correctly classified: $stage19_nz"
fi

echo "  - uninstaller exits nonzero AND a singbox-vpn lock is still held -> FAIL, classified UNINSTALL_LOCK_CONTENTION (not the generic class)"
out19_lock="$(run_uninstall_scenario lock_held)"
stage19_lock="$(sed -n '/=== 19\./,/=== 20\./p' <<< "$out19_lock")"
if grep -qE '\[FAIL\]\[required\].*UNINSTALL_LOCK_CONTENTION' <<< "$stage19_lock"; then
  ok "a nonzero exit with a held singbox-vpn lock is specifically classified UNINSTALL_LOCK_CONTENTION"
else
  fail "lock contention was not distinguished from a generic nonzero exit: $stage19_lock"
fi

echo "  - remote inner timeout fires (sentinel present, carried rc=124) -> FAIL, classified UNINSTALL_REMOTE_TIMEOUT"
out19_rt="$(run_uninstall_scenario remote_timeout)"
stage19_rt="$(sed -n '/=== 19\./,/=== 20\./p' <<< "$out19_rt")"
if grep -qE '\[FAIL\]\[required\].*UNINSTALL_REMOTE_TIMEOUT' <<< "$stage19_rt"; then
  ok "a remote inner-timeout completion is classified UNINSTALL_REMOTE_TIMEOUT"
else
  fail "a remote uninstall timeout was not correctly classified: $stage19_rt"
fi

echo "  - outer SSH/transport timeout fires before any sentinel -> FAIL, classified UNINSTALL_SSH_TIMEOUT (distinct from a remote timeout)"
out19_st="$(run_uninstall_scenario ssh_timeout)"
stage19_st="$(sed -n '/=== 19\./,/=== 20\./p' <<< "$out19_st")"
if grep -qE '\[FAIL\]\[required\].*UNINSTALL_SSH_TIMEOUT' <<< "$stage19_st"; then
  ok "an outer SSH/transport timeout (no sentinel) is classified UNINSTALL_SSH_TIMEOUT"
else
  fail "an SSH transport timeout was not correctly classified: $stage19_st"
fi

echo "  - uninstaller exits 0 but leaves NEW residue beyond baseline -> uninstall PASS, but post-condition FAILs classified UNINSTALL_POST_STATE_INVALID"
out19_res="$(run_uninstall_scenario residue_left)"
stage19_res="$(sed -n '/=== 19\./,/=== 20\./p' <<< "$out19_res")"
if grep -qE '\[PASS\][[:space:]]+singbox-vpn-uninstall --yes \(offline, local binary only\)' <<< "$stage19_res" \
    && grep -qE '\[FAIL\]\[required\].*UNINSTALL_POST_STATE_INVALID' <<< "$stage19_res" \
    && grep -q 'nginx_conf' <<< "$stage19_res"; then
  ok "a zero exit code alone is not enough — leftover residue after uninstall still FAILs, naming the field"
else
  fail "leftover residue after a zero-exit uninstall was not caught: $stage19_res"
fi

echo
echo "--- every vpn-admin/vpn invocation on the remote host uses an absolute path, never bare PATH lookup ---"
# Reproduced on a real AlmaLinux VPS: 'sudo vpn-admin ...' failed with
# 'sudo: vpn-admin: command not found' even though 'vpn-admin' (no sudo)
# and 'sudo /usr/local/bin/vpn-admin' both worked — RHEL-family sudo's
# default secure_path excludes /usr/local/bin (unlike Debian/Ubuntu),
# where install.sh (BIN_DIR=/usr/local/bin) installs both binaries. Every
# `sudo vpn-admin`/`sudo vpn ` call in this script must use the absolute
# path so it cannot depend on sudo's secure_path containing /usr/local/bin.
if grep -nE "sudo vpn-admin\b|sudo vpn ['\" ]" "$SCRIPT" | grep -v '/usr/local/bin/vpn'; then
  fail "found a bare 'sudo vpn-admin'/'sudo vpn' call that relies on sudo's secure_path containing /usr/local/bin (see above)"
else
  ok "no bare 'sudo vpn-admin'/'sudo vpn' calls remain — all use the absolute /usr/local/bin path"
fi

echo
echo "--- stage 5 (reboot+health) names the specific check that failed, instead of one opaque [FAIL] ---"
cat > "$MOCKBIN/ssh" <<'MOCKSSH_REBOOT'
#!/bin/bash
{ printf '%s\t' "$@"; echo; } >> "$SSH_LOG"
cmd="${*: -1}"
case "$cmd" in
  true) exit 0 ;;
  *os-release*) echo 'ID=almalinux'; exit 0 ;;
  *uname\ -m*) echo x86_64; exit 0 ;;
  # The real post-reboot script checks nginx among others; simulate
  # nginx specifically having failed to come back up after reboot,
  # while everything else in that script would have passed.
  *"POST_REBOOT_ALL_OK"*|*"POST_REBOOT_FAILED_CHECKS"*)
    echo "POST_REBOOT_FAILED_CHECKS: nginx"; exit 1 ;;
  *) exit 0 ;;
esac
MOCKSSH_REBOOT
chmod +x "$MOCKBIN/ssh"
: > "$SSH_LOG"
set +e
reboot_out="$(PATH="$MOCKBIN:$PATH" "$SCRIPT" --host root@disposable-test --i-understand-this-is-destructive --allow-destroy-existing-singbox-vpn-install 2>&1)"
set -e
if grep -qE '\[FAIL\]\[required\][[:space:]]+reboot \+ independent post-reboot verification[[:space:]]+\(.*nginx' <<< "$reboot_out"; then
  ok "stage 5 names the specific failed check (nginx) instead of a blank/generic failure"
else
  fail "stage 5 did not name which post-reboot check failed: $(grep -A1 'reboot + independent post-reboot verification' <<< "$reboot_out")"
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
