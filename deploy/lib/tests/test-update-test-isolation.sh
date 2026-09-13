#!/usr/bin/env bash
# Regression tests for defect D6 (real two-VPS acceptance): the test phase
# of `update.sh --dev-rebuild` / `--repair` must not be able to control the
# live host's services, must fail closed when isolation is unavailable, and
# the updater must report live-service state from observation.
#
# Uses fixtures only. As root (the CI shell job runs every test with sudo)
# it also proves the mount-namespace layer: inside the isolated run the
# systemd manager and D-Bus sockets are not reachable.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
UPDATE_SH="$REPO_ROOT/deploy/almalinux/update.sh"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/test-isolation.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# A "host" systemctl first in the caller's PATH: stands in for the real one
# on a live server and records anything that reaches it.
mkdir -p "$work/host-bin"
cat > "$work/host-bin/systemctl" <<EOF
#!/bin/sh
echo "\$*" >> "$work/host-systemctl.log"
case "\$1" in show) echo "MainPID=4242"; echo "NRestarts=0" ;; esac
exit 0
EOF
chmod 0755 "$work/host-bin/systemctl"
PATH="$work/host-bin:$PATH"
export PATH

# What a misbehaving test suite does.
cat > "$work/misbehaving-tests.sh" <<'EOF'
#!/bin/bash
systemctl reload-or-restart sing-box
"$SINGBOX_VPN_SYSTEMCTL" restart vpn-subscription
systemctl --version
firewall-cmd --reload
useradd acceptance-test-user
echo "SINGBOX_VPN_SYSTEMCTL=$SINGBOX_VPN_SYSTEMCTL" > "$RESULT_DIR/env"
for socket in /run/systemd/private /run/dbus/system_bus_socket; do
  if [ -S "$socket" ]; then
    echo "$socket reachable" >> "$RESULT_DIR/sockets"
  elif [ -e "$socket" ]; then
    echo "$socket masked" >> "$RESULT_DIR/sockets"
  fi
done
if [ -x /usr/bin/systemctl ]; then
  if /usr/bin/systemctl show -p Version --value >/dev/null 2>&1; then
    echo reachable > "$RESULT_DIR/absolute-systemctl"
  else
    echo unreachable > "$RESULT_DIR/absolute-systemctl"
  fi
fi
exit 0
EOF
chmod 0755 "$work/misbehaving-tests.sh"

echo "--- A/C: a test phase cannot reach the host's service manager ---"
mkdir -p "$work/result"
guard="$work/guard"
rc=0
RESULT_DIR="$work/result" run_isolated_from_host "$guard" "$work/misbehaving-tests.sh" || rc=$?
[ "$rc" -eq 0 ] && ok "the isolated command ran (exit $rc)" || fail "isolated command exit $rc"
if [ -s "$work/host-systemctl.log" ]; then
  fail "the host's systemctl was reached: $(tr '\n' ';' < "$work/host-systemctl.log")"
else
  ok "zero calls reached the host's systemctl"
fi
refused="$(cat "$guard/refused-host-control.log")"
for expected in "systemctl reload-or-restart sing-box" "systemctl restart vpn-subscription" \
  "firewall-cmd --reload" "useradd acceptance-test-user"; do
  if printf '%s\n' "$refused" | grep -qxF "$expected"; then
    ok "refused and recorded: $expected"
  else
    fail "not recorded as refused: $expected (log: $(printf '%s' "$refused" | tr '\n' ';'))"
  fi
done
if printf '%s\n' "$refused" | grep -q -- "--version"; then
  fail "a read-only query was recorded as a host-control attempt"
else
  ok "read-only queries are refused without being reported as attempts"
fi
grep -qxF "SINGBOX_VPN_SYSTEMCTL=$guard/bin/systemctl" "$work/result/env" \
  && ok "SINGBOX_VPN_SYSTEMCTL names the guard inside the test phase" \
  || fail "SINGBOX_VPN_SYSTEMCTL was not set to the guard"

if [ "$(id -u)" -eq 0 ]; then
  if [ -f "$work/result/sockets" ] && grep -q "reachable" "$work/result/sockets"; then
    fail "service-manager socket reachable inside the isolated run: $(tr '\n' ';' < "$work/result/sockets")"
  elif [ -f "$work/result/sockets" ]; then
    ok "service-manager sockets masked inside the isolated run ($(tr '\n' ';' < "$work/result/sockets"))"
  else
    ok "no service-manager sockets exist on this host (namespace layer not observable here)"
  fi
  for socket in "${HOST_CONTROL_SOCKETS[@]}"; do
    if [ -e "$socket" ] && [ ! -S "$socket" ]; then
      fail "$socket is no longer a socket OUTSIDE the isolated run — masking leaked to the host"
    fi
  done
  ok "masking did not leak outside the namespace"
  if [ -x /usr/bin/systemctl ] && /usr/bin/systemctl show -p Version --value >/dev/null 2>&1; then
    if [ "$(cat "$work/result/absolute-systemctl" 2>/dev/null)" = "unreachable" ]; then
      ok "an absolute /usr/bin/systemctl reaches PID 1 outside but not inside the isolated run"
    else
      fail "an absolute /usr/bin/systemctl reached the service manager inside the isolated run"
    fi
  else
    echo "skip: no running systemd manager on this host to compare against"
  fi

  echo "--- B: no namespace isolation, no test run ---"
  mkdir -p "$work/no-unshare"
  cat > "$work/no-unshare/unshare" <<'EOF'
#!/bin/sh
echo "unshare: unshare failed: Operation not permitted" >&2
exit 1
EOF
  chmod 0755 "$work/no-unshare/unshare"
  rm -f "$work/ran"
  rc=0
  PATH="$work/no-unshare:$PATH" run_isolated_from_host "$work/guard-b" \
    sh -c "touch '$work/ran'" || rc=$?
  if [ "$rc" -ne 0 ] && [ ! -e "$work/ran" ]; then
    ok "isolation failure is fatal and the tests never ran (exit $rc)"
  else
    fail "tests ran without namespace isolation (exit $rc)"
  fi
else
  echo "skip: namespace-layer assertions need root (the CI shell job runs this with sudo)"
fi

echo "--- B: an unwritable guard directory fails closed ---"
rm -f "$work/ran"
rc=0
run_isolated_from_host "/proc/singbox-vpn-no-such-guard" sh -c "touch '$work/ran'" 2>/dev/null || rc=$?
if [ "$rc" -ne 0 ] && [ ! -e "$work/ran" ]; then
  ok "no guards, no test run (exit $rc)"
else
  fail "tests ran without guards (exit $rc)"
fi

echo "--- D/E: service identity is observed, and update.sh reports it truthfully ---"
fingerprint="$(host_service_fingerprint)"
printf '%s\n' "$fingerprint" | grep -q "^sing-box .*MainPID=4242" \
  && printf '%s\n' "$fingerprint" | grep -q "^vpn-subscription .*NRestarts=0" \
  && ok "host_service_fingerprint records MainPID/NRestarts per unit" \
  || fail "unexpected fingerprint: $fingerprint"

dev_block="$(awk '/^if \[ "\$DEV_REBUILD" -eq 1 \]; then$/,/^  log "building new binaries\.\.\."$/' "$UPDATE_SH")"
if printf '%s' "$dev_block" | grep -q 'run_isolated_from_host "$test_guard_dir"' \
  && printf '%s' "$dev_block" | grep -q 'cargo test --workspace --locked'; then
  ok "update.sh runs the test phase through run_isolated_from_host"
else
  fail "update.sh's test phase is not isolated"
fi
if grep -nE '^[^#]*\bcargo test\b' "$UPDATE_SH" | grep -v "bash -c 'cd \"\$1\" && cargo test" | grep -q .; then
  fail "update.sh has a cargo test invocation outside the isolated runner"
else
  ok "no unisolated cargo test in update.sh"
fi
if grep -q "installed state was not changed" "$UPDATE_SH"; then
  fail "update.sh still claims 'installed state was not changed' without observing services"
else
  ok "the unverified 'installed state was not changed' claim is gone"
fi
before_line="$(printf '%s' "$dev_block" | grep -n 'services_before_tests="$(host_service_fingerprint)"' | cut -d: -f1)"
after_line="$(printf '%s' "$dev_block" | grep -n 'services_after_tests="$(host_service_fingerprint)"' | cut -d: -f1)"
run_line="$(printf '%s' "$dev_block" | grep -n 'run_isolated_from_host "$test_guard_dir"' | cut -d: -f1)"
if [ -n "$before_line" ] && [ -n "$after_line" ] && [ -n "$run_line" ] \
  && [ "$before_line" -lt "$run_line" ] && [ "$run_line" -lt "$after_line" ]; then
  ok "service identity is captured before and after the test phase"
else
  fail "service identity is not captured around the test phase"
fi
printf '%s' "$dev_block" | grep -q 'live services CHANGED while the test phase ran' \
  && printf '%s' "$dev_block" | grep -q 'attempted to control this host' \
  && ok "changed services and refused attempts each have their own truthful failure" \
  || fail "missing truthful failure messages for the test phase"
grep -q 'deliberate service activation (apply stage, not the test phase)' "$UPDATE_SH" \
  && ok "the apply stage's intentional restart is labelled as such" \
  || fail "the apply stage's restart is indistinguishable from test-side activity"

echo
if [ "$failures" -ne 0 ]; then
  echo "FAIL: test-update-test-isolation.sh ($failures failure(s))"
  exit 1
fi
echo "PASS: test-update-test-isolation.sh"
