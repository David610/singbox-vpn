#!/usr/bin/env bash
# Regression test for activate_firewalld_ssh_safe() (deploy/almalinux/install.sh,
# install_dependencies_rhel's AlmaLinux/RHEL firewall activation).
#
# BUG this guards against: activate_firewalld_ssh_safe() used to run
# `systemctl start firewalld` FIRST, then add the SSH-allow rule
# afterward with `firewall-cmd ... --add-service=ssh >/dev/null 2>&1 ||
# true`. That leaves a real (if brief) runtime window where firewalld
# enforces its distro-default policy on the zone — default-deny for a
# custom SSH port — before any rule permits it, and the `|| true` meant a
# failed add was silently treated as success. The fix stages the SSH
# allow into firewalld's permanent (offline) configuration via
# firewall-offline-cmd — which needs no running daemon — verifies it
# landed, THEN starts firewalld, THEN verifies the runtime rule too, with
# no fail-open anywhere in the chain.
#
# This test extracts activate_firewalld_ssh_safe()'s REAL body from
# install.sh and eval's it with systemctl/firewall-cmd/firewall-offline-cmd
# stubbed — not a reimplementation of the logic.
#
# (SSH_PORT is read by the eval'd function body below; the static
# analyzer cannot see that dynamic use.)
# shellcheck disable=SC2034
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

ACTIVATE_BODY="$(sed -n '/^activate_firewalld_ssh_safe() {/,/^}/p' "$INSTALL_SH")"
if [ -z "$ACTIVATE_BODY" ]; then
  fail "could not extract activate_firewalld_ssh_safe() from $INSTALL_SH — has it been renamed/moved?"
  echo "$failures test(s) FAILED"
  exit 1
fi

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

# Runs activate_firewalld_ssh_safe() in a subshell with systemctl/
# firewall-cmd/firewall-offline-cmd stubbed. The stubs model real
# firewalld behavior: `systemctl start firewalld` loads whatever was
# staged in the "permanent" (offline) state into the "runtime" state —
# unless break_runtime_load=yes, which simulates activation NOT actually
# picking up the staged rule (the scenario the runtime verification step
# must catch).
#
# no_offline_cmd=yes removes the firewall-offline-cmd stub entirely, to
# exercise the "tool unavailable" fail-closed path.
# break_offline_query=yes makes the offline query-service/query-port
# calls report "not present" even after add succeeds, to exercise the
# offline-verification fail-closed path.
run_activate_firewalld_ssh_safe() {
  local ssh_port="$1" initially_active="$2" calls_file="$3"
  local break_runtime_load="${4:-no}" no_offline_cmd="${5:-no}" break_offline_query="${6:-no}"
  (
    set -Eeuo pipefail
    log() { :; }
    warn() { :; }
    die() { echo "die: $*" >&2; exit 1; }
    SSH_PORT="$ssh_port"
    FIREWALLD_ACTIVE="$initially_active"
    OFFLINE_SSH_SERVICE=0
    OFFLINE_SSH_PORT=0
    RUNTIME_SSH_SERVICE=0
    RUNTIME_SSH_PORT=0
    BREAK_RUNTIME_LOAD="$break_runtime_load"
    BREAK_OFFLINE_QUERY="$break_offline_query"

    systemctl() {
      echo "systemctl $*" >> "$calls_file"
      if [ "$1" = "is-active" ]; then
        [ "$FIREWALLD_ACTIVE" = "yes" ]
        return $?
      fi
      if [ "$1" = "start" ] && [ "$2" = "firewalld" ]; then
        FIREWALLD_ACTIVE="yes"
        if [ "$BREAK_RUNTIME_LOAD" != "yes" ]; then
          RUNTIME_SSH_SERVICE="$OFFLINE_SSH_SERVICE"
          RUNTIME_SSH_PORT="$OFFLINE_SSH_PORT"
        fi
        return 0
      fi
      if [ "$1" = "enable" ]; then
        return 0
      fi
      return 0
    }

    if [ "$no_offline_cmd" != "yes" ]; then
      firewall-offline-cmd() {
        echo "firewall-offline-cmd $*" >> "$calls_file"
        case "$1" in
          --get-default-zone) echo "public"; return 0 ;;
        esac
        local zone_arg="$1" op="$2"
        case "$op" in
          --add-service=ssh) OFFLINE_SSH_SERVICE=1; return 0 ;;
          --add-port=*/tcp) OFFLINE_SSH_PORT=1; return 0 ;;
          --query-service=ssh)
            [ "$BREAK_OFFLINE_QUERY" != "yes" ] && [ "$OFFLINE_SSH_SERVICE" = "1" ]
            return $? ;;
          --query-port=*/tcp)
            [ "$BREAK_OFFLINE_QUERY" != "yes" ] && [ "$OFFLINE_SSH_PORT" = "1" ]
            return $? ;;
        esac
        return 0
      }
    fi

    firewall-cmd() {
      echo "firewall-cmd $*" >> "$calls_file"
      case "$1" in
        --get-default-zone) echo "public"; return 0 ;;
      esac
      local zone_arg="$1" op="$2"
      case "$op" in
        --query-service=ssh) [ "$RUNTIME_SSH_SERVICE" = "1" ]; return $? ;;
        --query-port=*/tcp) [ "$RUNTIME_SSH_PORT" = "1" ]; return $? ;;
        --add-service=ssh) RUNTIME_SSH_SERVICE=1; return 0 ;;
        --add-port=*/tcp) RUNTIME_SSH_PORT=1; return 0 ;;
        --permanent) return 0 ;;
        --reload) return 0 ;;
      esac
      return 0
    }

    eval "$ACTIVATE_BODY"
    activate_firewalld_ssh_safe
  )
}

echo "--- Scenario A: firewalld inactive, SSH port 22 ---"
calls="$TMPDIR_TEST/calls-a"
if run_activate_firewalld_ssh_safe 22 no "$calls"; then
  offline_line="$(grep -n 'firewall-offline-cmd.*--add-service=ssh' "$calls" | head -1 | cut -d: -f1)"
  offline_query_line="$(grep -n 'firewall-offline-cmd.*--query-service=ssh' "$calls" | head -1 | cut -d: -f1)"
  start_line="$(grep -n '^systemctl start firewalld' "$calls" | head -1 | cut -d: -f1)"
  runtime_query_line="$(grep -n 'firewall-cmd.*--query-service=ssh' "$calls" | head -1 | cut -d: -f1)"
  if [ -n "$offline_line" ] && [ -n "$offline_query_line" ] && [ -n "$start_line" ] && [ -n "$runtime_query_line" ] \
      && [ "$offline_line" -lt "$start_line" ] && [ "$offline_query_line" -lt "$start_line" ] \
      && [ "$start_line" -lt "$runtime_query_line" ]; then
    ok "SSH allowance staged+verified offline before firewalld starts, then verified again at runtime"
  else
    fail "wrong ordering (offline_add=$offline_line offline_query=$offline_query_line start=$start_line runtime_query=$runtime_query_line):"; sed 's/^/    /' "$calls"
  fi
else
  fail "activate_firewalld_ssh_safe() unexpectedly failed for port 22:"; sed 's/^/    /' "$calls"
fi

echo
echo "--- Scenario B: firewalld inactive, custom SSH port 2222 ---"
calls="$TMPDIR_TEST/calls-b"
if run_activate_firewalld_ssh_safe 2222 no "$calls"; then
  if grep -q -- '--add-port=2222/tcp' "$calls"; then
    ok "exact 2222/tcp allowance staged"
  else
    fail "2222/tcp was never staged:"; sed 's/^/    /' "$calls"
  fi
  # No line anywhere may show firewalld starting before the 2222 rule exists.
  start_line="$(grep -n '^systemctl start firewalld' "$calls" | head -1 | cut -d: -f1)"
  port_add_line="$(grep -n -- '--add-port=2222/tcp' "$calls" | head -1 | cut -d: -f1)"
  runtime_port_query_line="$(grep -n -- 'firewall-cmd.*--query-port=2222/tcp' "$calls" | head -1 | cut -d: -f1)"
  if [ -n "$start_line" ] && [ -n "$port_add_line" ] && [ "$port_add_line" -lt "$start_line" ] \
      && [ -n "$runtime_port_query_line" ] && [ "$start_line" -lt "$runtime_port_query_line" ]; then
    ok "no ordering where firewalld starts before 2222/tcp exists; runtime 2222/tcp verified after activation"
  else
    fail "2222/tcp was not staged before start and verified after (start=$start_line add=$port_add_line runtime_query=$runtime_port_query_line):"; sed 's/^/    /' "$calls"
  fi
else
  fail "activate_firewalld_ssh_safe() unexpectedly failed for port 2222:"; sed 's/^/    /' "$calls"
fi

echo
echo "--- Scenario C: firewalld already active — must not reset/flush, no offline staging needed ---"
calls="$TMPDIR_TEST/calls-c"
if run_activate_firewalld_ssh_safe 22 yes "$calls"; then
  if grep -q '^systemctl start firewalld' "$calls"; then
    fail "firewalld was already active but activate_firewalld_ssh_safe() started it again anyway:"; sed 's/^/    /' "$calls"
  else
    ok "an already-active firewalld is left completely alone (existing configuration preserved)"
  fi
else
  fail "activate_firewalld_ssh_safe() unexpectedly failed when firewalld was already active:"; sed 's/^/    /' "$calls"
fi

echo
echo "--- Scenario D1: firewall-offline-cmd unavailable — must fail closed, not silently skip to unsafe ordering ---"
calls="$TMPDIR_TEST/calls-d1"
if run_activate_firewalld_ssh_safe 22 no "$calls" no yes 2>"$TMPDIR_TEST/stderr-d1"; then
  fail "activate_firewalld_ssh_safe() succeeded even though firewall-offline-cmd is unavailable"
else
  if grep -q 'firewall-offline-cmd not found' "$TMPDIR_TEST/stderr-d1" && ! grep -q '^systemctl start firewalld' "$calls"; then
    ok "fails closed before starting firewalld when firewall-offline-cmd is unavailable"
  else
    fail "did not fail closed correctly for missing firewall-offline-cmd:"; cat "$TMPDIR_TEST/stderr-d1"; sed 's/^/    /' "$calls"
  fi
fi

echo
echo "--- Scenario D2: offline rule add 'succeeds' but offline verification can't confirm it — must fail closed before starting firewalld ---"
calls="$TMPDIR_TEST/calls-d2"
if run_activate_firewalld_ssh_safe 22 no "$calls" no no yes 2>"$TMPDIR_TEST/stderr-d2"; then
  fail "activate_firewalld_ssh_safe() succeeded even though offline verification never confirmed the SSH rule"
else
  if grep -q 'firewall-offline-cmd does not report it present' "$TMPDIR_TEST/stderr-d2" && ! grep -q '^systemctl start firewalld' "$calls"; then
    ok "fails closed before starting firewalld when offline verification can't confirm the staged rule"
  else
    fail "did not fail closed correctly on broken offline verification:"; cat "$TMPDIR_TEST/stderr-d2"; sed 's/^/    /' "$calls"
  fi
fi

echo
echo "--- Scenario D3: firewalld starts but the runtime rule never actually took (activation/load divergence) — must fail closed, not report success ---"
calls="$TMPDIR_TEST/calls-d3"
if run_activate_firewalld_ssh_safe 22 no "$calls" yes 2>"$TMPDIR_TEST/stderr-d3"; then
  fail "activate_firewalld_ssh_safe() reported success even though the runtime SSH rule was never actually confirmed after firewalld started"
else
  if grep -q 'NOT present in its effective runtime configuration' "$TMPDIR_TEST/stderr-d3"; then
    ok "fails closed (and warns loudly, without disconnecting) when the runtime rule can't be confirmed after activation"
  else
    fail "did not fail closed correctly on a runtime verification mismatch:"; cat "$TMPDIR_TEST/stderr-d3"
  fi
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
